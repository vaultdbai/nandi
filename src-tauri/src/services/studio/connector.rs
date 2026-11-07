//! Studio › New connector: a connector made from an existing MCP server.
//!
//! The person names the server (`npx <package>@<version>`, `uvx …`, or an
//! HTTP URL), its sign-in and the hosts it reaches. Nandi can start it
//! once, in its container, to list its tools; each tool is read unless its
//! name says it changes something. The result is a `connector.md` in the
//! agents repo (`.nandi/connectors/<id>/connector.md`), installed here,
//! ready for **Publish** (`marketplace::connectors::publish`).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::connectors::{self, ConnectorInfo};
use crate::services::domain::connector_package::{
    AuthKind, AuthSpec, ConnectorPackage, PackageTool, Placement, RunSpec,
};
use crate::services::domain::grant::GrantAccess;
use crate::services::domain::ConnectorId;
use crate::services::AppState;

#[derive(Debug, Clone, Deserialize, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldConnectorInput {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `x.y.z`, the first release
    #[serde(default = "first_version")]
    pub version: String,
    /// `npx <package>@<version>` or `uvx <package>==<version>`; or `url`
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub auth: AuthSpec,
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Empty: start the server once and list its tools
    #[serde(default)]
    pub tools: Vec<PackageTool>,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// For agents: when to use it, what to be careful with
    #[serde(default)]
    pub instructions: String,
    /// The API key, for listing the tools and for use here (keyring only)
    #[serde(default)]
    pub api_key: Option<String>,
}

fn first_version() -> String {
    "0.1.0".to_string()
}

/// A tool's access from its name: reading verbs are read, anything else
/// counts as write (the person can change it before publishing)
pub fn guess_access(name: &str) -> GrantAccess {
    const READ: &[&str] = &[
        "get", "list", "search", "read", "fetch", "find", "query", "describe", "show", "view",
        "lookup", "count", "check", "download",
    ];
    let lower = name.to_ascii_lowercase();
    let first = lower.split(['_', '-', '.']).next().unwrap_or_default();
    if READ.contains(&first) {
        GrantAccess::Read
    } else {
        GrantAccess::Write
    }
}

/// The package the input describes, before tools are known
fn package_of(
    input: &ScaffoldConnectorInput,
    tools: Vec<PackageTool>,
) -> Result<ConnectorPackage, String> {
    let id = ConnectorId::parse(input.id.trim().to_string()).map_err(|e| e.to_string())?;
    let run =
        match (
            input.run.as_deref().map(str::trim),
            input.url.as_deref().map(str::trim),
        ) {
            (Some(run), None | Some("")) if !run.is_empty() => RunSpec::parse_command(run)?,
            (None | Some(""), Some(url)) if url.starts_with("https://") => RunSpec::Url {
                url: url.to_string(),
            },
            _ => return Err(
                "Give the server's package (npx or uvx, an exact version) or its https:// address"
                    .into(),
            ),
        };
    let version = semver::Version::parse(input.version.trim().trim_start_matches('v'))
        .map_err(|e| format!("The version isn't x.y.z: {e}"))?;
    let package = ConnectorPackage {
        id,
        name: input.name.trim().to_string(),
        description: input.description.trim().to_string(),
        version,
        run,
        image: input.image.clone().filter(|i| !i.trim().is_empty()),
        placement: Placement::Container,
        auth: input.auth.clone(),
        hosts: input
            .hosts
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect(),
        tools,
        reads: input.reads.clone(),
        writes: input.writes.clone(),
        tags: input.tags.clone(),
        instructions: input.instructions.trim().to_string(),
    };
    // The same checks as a published file
    ConnectorPackage::parse(package.id.as_str(), &package.to_markdown()).map_err(|e| e.to_string())
}

/// Starts the server once (in its container) and lists its tools
async fn discover(
    state: &AppState,
    input: &ScaffoldConnectorInput,
) -> Result<Vec<PackageTool>, String> {
    let placeholder = vec![PackageTool {
        name: "placeholder".into(),
        description: "Listed once the server starts".into(),
        access: GrantAccess::Read,
    }];
    let package = package_of(input, placeholder)?;
    let decl = package.declaration(None);
    if let Some(key) = input.api_key.as_ref().filter(|k| !k.trim().is_empty()) {
        state.secrets().set(&package.key_entry(), key.trim())?;
    }
    let registry = state.connectors().ok_or("Connectors aren't available")?;
    let tools = registry.discover_tools(&decl, state.secrets()).await?;
    Ok(tools
        .into_iter()
        .map(|t| PackageTool {
            access: guess_access(&t.name),
            description: {
                let d: String = t.description.trim().chars().take(200).collect();
                if d.is_empty() {
                    t.name.clone()
                } else {
                    d
                }
            },
            name: t.name,
        })
        .collect())
}

/// Writes the connector.md and installs it here. Returns the Library entry
pub async fn scaffold(
    state: &AppState,
    input: ScaffoldConnectorInput,
) -> Result<ConnectorInfo, String> {
    if connectors::is_account_connector(input.id.trim()) {
        return Err(format!("'{}' is one of Nandi's own connectors", input.id));
    }
    let tools = if input.tools.is_empty() {
        discover(state, &input).await?
    } else {
        input.tools.clone()
    };
    let package = package_of(&input, tools)?;
    if package.auth.kind == AuthKind::ApiKey
        && input.api_key.as_deref().is_none_or(|k| k.trim().is_empty())
    {
        log::info!(
            "Connector '{}' has no API key yet; it asks at Start",
            package.id
        );
    }
    connectors::install_package(state, &package, None, input.api_key.clone()).await?;
    connectors::list_connectors(state)
        .await?
        .into_iter()
        .find(|c| c.id == package.id.as_str())
        .ok_or_else(|| "The connector was saved but couldn't be read back".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_suggest_read_or_write() {
        assert_eq!(guess_access("list_issues"), GrantAccess::Read);
        assert_eq!(guess_access("get-page"), GrantAccess::Read);
        assert_eq!(guess_access("search"), GrantAccess::Read);
        assert_eq!(guess_access("create_issue"), GrantAccess::Write);
        assert_eq!(guess_access("delete"), GrantAccess::Write);
        assert_eq!(guess_access("listing_maker"), GrantAccess::Write);
    }

    #[test]
    fn the_input_becomes_a_valid_connector_md() {
        let input = ScaffoldConnectorInput {
            id: "tracker".into(),
            name: "Issue tracker".into(),
            description: "Read and file issues.".into(),
            version: "1.0.0".into(),
            run: Some("npx -y @acme/tracker-mcp@1.2.3".into()),
            url: None,
            image: None,
            auth: AuthSpec {
                kind: AuthKind::ApiKey,
                env: vec!["TRACKER_TOKEN".into()],
                scopes: vec![],
                label: Some("Tracker API key".into()),
            },
            hosts: vec!["api.tracker.example".into()],
            tools: vec![PackageTool {
                name: "list_issues".into(),
                description: "List issues".into(),
                access: GrantAccess::Read,
            }],
            reads: vec!["Issues in your projects".into()],
            writes: vec![],
            tags: vec!["dev".into()],
            instructions: "Use it to look up issues.".into(),
            api_key: None,
        };
        let package = package_of(&input, input.tools.clone()).unwrap();
        assert_eq!(package.version.to_string(), "1.0.0");
        assert!(package
            .to_markdown()
            .contains("run: npx -y @acme/tracker-mcp@1.2.3"));
        let mut bad = input.clone();
        bad.run = Some("npx @acme/tracker-mcp@latest".into());
        assert!(package_of(&bad, input.tools.clone()).is_err());
        let mut none = input;
        none.run = None;
        assert!(package_of(&none, vec![]).is_err());
    }
}
