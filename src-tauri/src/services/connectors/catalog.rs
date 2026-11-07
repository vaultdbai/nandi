//! What connectors exist: the built-ins (filesystem, web_search, github,
//! aws, s3_backup) and external MCP servers declared as
//! `kind: mcp` in `.nandi/connectors.yaml`:
//!
//! ```yaml
//! connectors:
//!   - id: filesystem
//!     kind: filesystem
//!     config:
//!       folders: [{id: docs, path: ~/Documents/agent-share, access: read}]
//!   - id: notes
//!     kind: mcp
//!     config:
//!       name: Notes
//!       command: npx
//!       args: [-y, "@modelcontextprotocol/server-memory"]
//!       safe_tools: [read_graph, search_nodes]   # everything else needs approval
//!     secret_refs:
//!       NOTES_TOKEN: nandi.connector.notes.token  # env var <- keyring entry
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use super::aws::{AwsConnector, CliRunner};
use super::filesystem::FilesystemConnector;
use super::github::{self, GithubConnector};
use super::mcp::McpConnector;
use super::s3_backup::{self, BackupSettings, S3BackupConnector, SystemHost};
use super::web_search::{Engine, WebSearchConnector};
use super::{Connector, ConnectorDecl, ToolDef};
use crate::services::domain::ConnectorId;
use crate::services::secrets::Secrets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorKind {
    /// An MCP server (in-process built-in or external process / URL)
    Mcp,
}

impl ConnectorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
        }
    }
}

/// How a connector shows up and which of its tools need approval.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: ConnectorKind,
    /// Tools that go through the approval flow
    pub sensitive_tools: Vec<String>,
}

pub(super) const FILESYSTEM: &str = "filesystem";
const WEB_SEARCH: &str = "web_search";
const GITHUB: &str = "github";
const AWS: &str = "aws";
pub(super) const S3_BACKUP: &str = "s3_backup";
pub const MCP: &str = "mcp";

/// The connectors the catalog offers to install.
pub fn builtins() -> Vec<ConnectorDefinition> {
    let definition =
        |id: &str, name: &str, description: &str, kind, sensitive: &[&str]| ConnectorDefinition {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            kind,
            sensitive_tools: sensitive.iter().map(|s| s.to_string()).collect(),
        };
    vec![
        definition(
            FILESYSTEM,
            "Filesystem",
            "Read, and if you allow it write, files in folders on this computer that you share. Each agent gets only the folders you give it.",
            ConnectorKind::Mcp,
            &["write_file"],
        ),
        definition(
            WEB_SEARCH,
            "Web search",
            "Search the web and read pages, only on the sites each agent may reach (its Internet access). No account or API key.",
            ConnectorKind::Mcp,
            &[],
        ),
        definition(
            GITHUB,
            "GitHub",
            "Read pull requests and files, and post reviews (after you approve them), with a GitHub token. The token stays on this computer.",
            ConnectorKind::Mcp,
            &["post_review"],
        ),
        definition(
            AWS,
            "AWS",
            "CloudWatch alarms, metrics and Logs, AWS Batch jobs, ECS services and EC2 status, through the AWS CLI with your profile. Restarting a failed job and redeploying a service wait for your approval. Credentials stay on this computer.",
            ConnectorKind::Mcp,
            &["batch_restart", "ecs_redeploy"],
        ),
        definition(
            S3_BACKUP,
            "S3 Backup",
            "Backs up the folders you assign to your own Amazon S3 bucket as Glacier Deep Archive (the cheapest storage, about $1 per TB a month). It can open the AWS sign-up page, create the bucket and an upload-only user for you; those steps wait for your approval. Runs through the AWS CLI on this computer.",
            ConnectorKind::Mcp,
            &["open_aws_signup", "create_backup_bucket", "create_upload_user"],
        ),
    ]
}

pub fn builtin(id: &str) -> Option<ConnectorDefinition> {
    builtins().into_iter().find(|d| d.id == id)
}

/// A fresh declaration for a built-in: no config, no secret refs (built-ins
/// know their keyring entries).
pub fn declaration(definition: &ConnectorDefinition) -> ConnectorDecl {
    // AWS and S3 Backup start with the settings Setup shows
    let config = match definition.id.as_str() {
        AWS => BTreeMap::from([
            ("profile".to_string(), serde_yaml::Value::from("default")),
            ("region".to_string(), serde_yaml::Value::from("")),
        ]),
        S3_BACKUP => BTreeMap::from([
            ("profile".to_string(), serde_yaml::Value::from("default")),
            (
                "upload_profile".to_string(),
                serde_yaml::Value::from(s3_backup::DEFAULT_UPLOAD_PROFILE),
            ),
            ("region".to_string(), serde_yaml::Value::from("")),
            ("bucket".to_string(), serde_yaml::Value::from("")),
            (
                "folders".to_string(),
                serde_yaml::Value::Sequence(Vec::new()),
            ),
        ]),
        _ => BTreeMap::new(),
    };
    ConnectorDecl {
        id: ConnectorId::parse(definition.id.clone()).expect("built-in connector IDs are valid"),
        kind: definition.id.clone(),
        config,
        secret_refs: BTreeMap::new(),
    }
}

pub(super) fn config_str(decl: &ConnectorDecl, key: &str) -> Option<String> {
    decl.config
        .get(key)
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn config_list(decl: &ConnectorDecl, key: &str) -> Vec<String> {
    match decl.config.get(key) {
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .filter_map(|item| match item {
                serde_yaml::Value::String(s) => Some(s.clone()),
                serde_yaml::Value::Number(n) => Some(n.to_string()),
                serde_yaml::Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
            .collect(),
        Some(serde_yaml::Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// The definition behind a declaration: built-ins by `kind`, with the
/// declaration's ID and optional `config.name` / `config.description`.
pub fn definition_for(decl: &ConnectorDecl) -> ConnectorDefinition {
    let mut definition = builtin(&decl.kind).unwrap_or_else(|| ConnectorDefinition {
        id: decl.id.to_string(),
        name: decl.id.to_string(),
        description: if decl.kind == MCP {
            "An MCP server declared in connectors.yaml.".to_string()
        } else {
            format!("Connector kind '{}' has no runtime yet.", decl.kind)
        },
        kind: ConnectorKind::Mcp,
        sensitive_tools: Vec::new(),
    });
    definition.id = decl.id.to_string();
    if let Some(name) = config_str(decl, "name") {
        definition.name = name;
    }
    if let Some(description) = config_str(decl, "description") {
        definition.description = description;
    }
    definition
        .sensitive_tools
        .extend(config_list(decl, "sensitive_tools"));
    definition
}

/// Tools known without starting the connector (for the UI).
pub fn static_tools(kind: &str) -> Vec<ToolDef> {
    match kind {
        FILESYSTEM => FilesystemConnector::tool_defs(),
        WEB_SEARCH => WebSearchConnector::tool_defs(),
        GITHUB => GithubConnector::tool_defs(),
        AWS => AwsConnector::tool_defs(),
        S3_BACKUP => S3BackupConnector::tool_defs(),
        _ => Vec::new(),
    }
}

/// `(key, keyring entry)` for every secret the connector reads.
pub fn secret_entries(decl: &ConnectorDecl) -> Vec<(String, String)> {
    if decl.kind == GITHUB {
        return vec![("token".to_string(), github::key_entry(decl))];
    }
    decl.secret_refs
        .iter()
        .map(|(key, entry)| (key.clone(), entry.as_str().to_string()))
        .collect()
}

/// Whether Start needs `key` set: GitHub's token is optional, since the
/// GitHub CLI's sign-in stands in for it (the setup hint says when neither
/// is there)
pub fn secret_required(decl: &ConnectorDecl, key: &str) -> bool {
    !(decl.kind == GITHUB && key == "token")
}

/// The GitHub sign-in's token, unless the connector points at another
/// GitHub (Enterprise `api_url`): the token is only good on github.com
fn sign_in_token(decl: &ConnectorDecl, secrets: &Secrets) -> Option<String> {
    let on_github_com = config_str(decl, "api_url")
        .is_none_or(|url| url.trim_end_matches('/') == "https://api.github.com");
    if on_github_com {
        crate::services::auth::github::token(secrets)
    } else {
        None
    }
}

/// What's missing before `decl` can start, if anything, in plain words
/// (the Library shows it under a disabled Start; no file or keychain names).
pub fn setup_hint(decl: &ConnectorDecl, secrets: &Secrets) -> Option<String> {
    let has = |entry: &str| secrets.get(entry).ok().flatten().is_some();
    match decl.kind.as_str() {
        FILESYSTEM => match super::folders::shared_folders(decl) {
            Ok(folders) if folders.is_empty() => Some("Add a folder agents can use".to_string()),
            Ok(_) => None,
            Err(e) => Some(e),
        },
        GITHUB
            if !has(&github::key_entry(decl))
                && sign_in_token(decl, secrets).is_none()
                && github::cli_token().is_none() =>
        {
            Some("Sign in to GitHub, or add a GitHub token".to_string())
        }
        MCP if config_str(decl, "command").is_none() && config_str(decl, "url").is_none() => {
            Some("Set the command or address of the MCP server".to_string())
        }
        _ => None,
    }
}

/// Builds a runnable connector from its declaration.
pub async fn build(
    decl: &ConnectorDecl,
    definition: &ConnectorDefinition,
    secrets: &Secrets,
    placement: &super::mcp::Placement,
) -> Result<Arc<dyn Connector>, String> {
    match decl.kind.as_str() {
        FILESYSTEM => {
            let folders = super::folders::shared_folders(decl)?;
            let connector = FilesystemConnector::new(&folders, &definition.sensitive_tools)?;
            Ok(Arc::new(connector))
        }
        WEB_SEARCH => {
            // `config.engine`: bing (default) or duckduckgo
            let engine = Engine::parse(config_str(decl, "engine").as_deref())?;
            let endpoint = config_str(decl, "endpoint");
            Ok(Arc::new(WebSearchConnector::new(engine, endpoint)))
        }
        GITHUB => {
            let entry = github::key_entry(decl);
            // A pasted token first, then the GitHub sign-in, else the
            // GitHub CLI's
            let token = secrets
                .get(&entry)?
                .filter(|t| !t.trim().is_empty())
                .or_else(|| sign_in_token(decl, secrets))
                .or_else(github::cli_token)
                .ok_or_else(|| {
                    "No GitHub token: sign in to GitHub, or paste a token in Setup".to_string()
                })?;
            Ok(Arc::new(GithubConnector::new(
                token,
                config_str(decl, "api_url"),
            )))
        }
        AWS => Ok(Arc::new(AwsConnector::new(Arc::new(CliRunner::new(
            config_str(decl, "profile"),
            config_str(decl, "region"),
        ))))),
        S3_BACKUP => Ok(Arc::new(S3BackupConnector::new(
            Arc::new(SystemHost),
            BackupSettings::from_decl(decl)?,
        ))),
        MCP => {
            let connector = McpConnector::connect(decl, definition, secrets, placement).await?;
            Ok(Arc::new(connector))
        }
        _ => Err(format!("Connector kind '{}' has no runtime yet", decl.kind)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::ConnectorManifest;

    #[test]
    fn builtins_are_installable_and_valid() {
        let decls: Vec<_> = builtins().iter().map(declaration).collect();
        let manifest = ConnectorManifest::new(decls).unwrap();
        let ids: Vec<_> = manifest
            .connectors()
            .iter()
            .map(|c| c.id.to_string())
            .collect();
        assert_eq!(
            ids,
            ["filesystem", "web_search", "github", "aws", "s3_backup",]
        );
        let fs = definition_for(manifest.get("filesystem").unwrap());
        assert_eq!(fs.sensitive_tools, ["write_file"]);
        let github = definition_for(manifest.get("github").unwrap());
        assert_eq!(github.kind, ConnectorKind::Mcp);
        assert_eq!(github.sensitive_tools, ["post_review"]);
    }

    #[test]
    fn declared_mcp_servers_take_name_and_sensitive_tools_from_config() {
        let manifest = ConnectorManifest::from_yaml(
            "connectors:\n  - id: notes\n    kind: mcp\n    config:\n      name: Notes\n      command: npx\n      sensitive_tools: [delete]\n",
        )
        .unwrap();
        let notes = manifest.get("notes").unwrap();
        let definition = definition_for(notes);
        assert_eq!(definition.name, "Notes");
        assert_eq!(definition.kind, ConnectorKind::Mcp);
        assert_eq!(definition.sensitive_tools, ["delete"]);
        assert_eq!(setup_hint(notes, &Secrets::in_memory()), None);
    }

    #[test]
    fn setup_hints_name_what_is_missing_in_plain_words() {
        let secrets = Secrets::in_memory();
        let fs = declaration(&builtin("filesystem").unwrap());
        assert_eq!(
            setup_hint(&fs, &secrets).as_deref(),
            Some("Add a folder agents can use")
        );
        // Web search needs no key
        let search = declaration(&builtin("web_search").unwrap());
        assert_eq!(setup_hint(&search, &secrets), None);
        assert!(secret_entries(&search).is_empty());
        let github = declaration(&builtin("github").unwrap());
        github::set_cli_token_for_tests(None);
        assert_eq!(
            setup_hint(&github, &secrets).as_deref(),
            Some("Sign in to GitHub, or add a GitHub token")
        );
        // Signed in to GitHub: nothing to add
        secrets
            .set(
                crate::services::auth::github::TOKEN_ENTRY,
                r#"{"access_token":"gho_signed_in"}"#,
            )
            .unwrap();
        assert_eq!(setup_hint(&github, &secrets), None);
        secrets
            .delete(crate::services::auth::github::TOKEN_ENTRY)
            .unwrap();
        // Signed in with the GitHub CLI: nothing to add
        github::set_cli_token_for_tests(Some("gho_cli"));
        assert_eq!(setup_hint(&github, &secrets), None);
        github::set_cli_token_for_tests(None);
        secrets.set("connectors.github.token", "t").unwrap();
        assert_eq!(setup_hint(&github, &secrets), None);
    }
}
