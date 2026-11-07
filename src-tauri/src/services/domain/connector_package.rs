//! A published connector: one `connector.md` file, like an agent's
//! `skill.md`. Short frontmatter says how the MCP server runs, what it
//! reaches and what sign-in it needs; the body is plain instructions for
//! agents, which go into an agent's context with the tools once the agent is
//! granted the connector.
//!
//! ```markdown
//! ---
//! name: Web page fetch
//! description: Fetch a web page and read it as Markdown.
//! version: 1.0.0
//! run: uvx mcp-server-fetch==2025.4.7      # or npx -y <package>@<x.y.z>, or url: https://…
//! auth: none                               # none | api_key | oauth | device_code | {type, env, label, scopes}
//! hosts: [example.com]
//! tools:
//!   - { name: fetch, description: Fetch a URL and return its text, access: read }
//! reads: [Web pages you ask it to open]
//! writes: []
//! tags: [web]
//! ---
//! Use it to read a page the person names. Never send credentials in a URL.
//! ```
//!
//! The format is in `nandi-spec/registry-schema/connector-md.md`, and the
//! registry entry built from it in `connector.schema.json`. Installed, a
//! package is a `kind: mcp` declaration in `connectors.yaml` (built by
//! [`ConnectorPackage::declaration`]) plus this file at
//! `.nandi/connectors/<id>.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

use super::agent::split_frontmatter;
use super::connector::Connector;
use super::grant::GrantAccess;
use super::settings::SecretRef;
use super::{ConnectorId, DomainError, DomainResult};

const FILE: &str = "connector.md";
/// Agents read the instructions on every turn: keep them short
pub const MAX_INSTRUCTION_WORDS: usize = 600;

/// How the server runs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum RunSpec {
    /// `npx <package>@<version>` or `uvx <package>==<version>`, an exact version
    Package {
        command: PackageRunner,
        package: String,
        version: String,
        args: Vec<String>,
    },
    /// A Streamable HTTP MCP server
    Url { url: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PackageRunner {
    Npx,
    Uvx,
}

impl PackageRunner {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Npx => "npx",
            Self::Uvx => "uvx",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Placement {
    /// Its own container, reaching only its hosts through the egress proxy
    #[default]
    Container,
    /// On this computer, as the person. Never from the public registry
    Host,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    #[default]
    None,
    ApiKey,
    Oauth,
    DeviceCode,
}

impl AuthKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ApiKey => "api_key",
            Self::Oauth => "oauth",
            Self::DeviceCode => "device_code",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct AuthSpec {
    pub kind: AuthKind,
    /// api_key on a stdio server: the variables the key goes into
    pub env: Vec<String>,
    pub scopes: Vec<String>,
    /// What the key or account is called ("Linear API key")
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackageTool {
    pub name: String,
    pub description: String,
    pub access: GrantAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorPackage {
    pub id: ConnectorId,
    pub name: String,
    pub description: String,
    pub version: semver::Version,
    pub run: RunSpec,
    pub image: Option<String>,
    pub placement: Placement,
    pub auth: AuthSpec,
    pub hosts: Vec<String>,
    pub tools: Vec<PackageTool>,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub tags: Vec<String>,
    /// The body: plain instructions for agents
    pub instructions: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frontmatter {
    name: String,
    description: String,
    version: String,
    #[serde(default)]
    run: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    placement: Placement,
    #[serde(default)]
    auth: Option<AuthField>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    tools: Vec<ToolField>,
    #[serde(default)]
    reads: Vec<String>,
    #[serde(default)]
    writes: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AuthField {
    Kind(AuthKind),
    Full(AuthMapping),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthMapping {
    #[serde(rename = "type")]
    kind: AuthKind,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolField {
    name: String,
    description: String,
    access: GrantAccess,
}

#[derive(Serialize)]
struct FrontmatterOut<'a> {
    name: &'a str,
    description: &'a str,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<&'a str>,
    #[serde(skip_serializing_if = "is_container")]
    placement: Placement,
    auth: Value,
    hosts: &'a [String],
    tools: Vec<Value>,
    reads: &'a [String],
    writes: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    tags: &'a [String],
}

fn is_container(placement: &Placement) -> bool {
    *placement == Placement::Container
}

fn invalid(field: &'static str, reason: impl Into<String>) -> DomainError {
    DomainError::invalid(FILE, field, reason)
}

/// An exact version: digits and dots, an optional pre-release; no ranges,
/// wildcards or tags
fn exact_version(version: &str, runner: PackageRunner) -> Result<(), String> {
    let ok = !version.is_empty()
        && version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        && !version.split('.').any(|part| part == "x" || part == "X");
    if !ok {
        return Err(format!("'{version}' isn't an exact version"));
    }
    // npm reads `1.2` as a range
    if runner == PackageRunner::Npx && semver::Version::parse(version).is_err() {
        return Err(format!("'{version}' isn't an exact npm version (x.y.z)"));
    }
    Ok(())
}

fn valid_package(name: &str) -> bool {
    let (scope, rest) = match name.strip_prefix('@') {
        Some(scoped) => match scoped.split_once('/') {
            Some((scope, rest)) => (Some(scope), rest),
            None => return false,
        },
        None => (None, name),
    };
    let part = |s: &str, first_upper: bool| {
        s.chars().next().is_some_and(|c| {
            c.is_ascii_digit() || c.is_ascii_lowercase() || (first_upper && c.is_ascii_uppercase())
        }) && s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    scope.is_none_or(|s| part(s, false)) && part(rest, true)
}

impl RunSpec {
    /// `npx [-y] <package>@<version> [args…]` or `uvx <package>==<version> [args…]`
    pub fn parse_command(line: &str) -> Result<Self, String> {
        let mut words = line.split_whitespace();
        let runner = match words.next() {
            Some("npx") => PackageRunner::Npx,
            Some("uvx") => PackageRunner::Uvx,
            _ => return Err("must start with npx or uvx".to_string()),
        };
        let mut spec = words.next();
        if runner == PackageRunner::Npx && matches!(spec, Some("-y" | "--yes")) {
            spec = words.next();
        }
        let spec = spec.ok_or("names no package")?;
        let (package, version) = match runner {
            // `@scope/name@1.2.3`: the version is after the last `@`
            PackageRunner::Npx => match spec.rfind('@') {
                Some(at) if at > 0 => (&spec[..at], &spec[at + 1..]),
                _ => return Err(format!("'{spec}' has no exact version (<package>@<x.y.z>)")),
            },
            PackageRunner::Uvx => spec
                .split_once("==")
                .ok_or_else(|| format!("'{spec}' has no exact version (<package>==<version>)"))?,
        };
        if !valid_package(package) {
            return Err(format!("'{package}' isn't a package name"));
        }
        exact_version(version, runner)?;
        Ok(Self::Package {
            command: runner,
            package: package.to_string(),
            version: version.to_string(),
            args: words.map(str::to_string).collect(),
        })
    }

    /// The command line: `npx -y <package>@<version> …`
    pub fn command_line(&self) -> Option<(String, Vec<String>)> {
        let Self::Package {
            command,
            package,
            version,
            args,
        } = self
        else {
            return None;
        };
        let mut out = match command {
            PackageRunner::Npx => vec!["-y".to_string(), format!("{package}@{version}")],
            PackageRunner::Uvx => vec![format!("{package}=={version}")],
        };
        out.extend(args.iter().cloned());
        Some((command.as_str().to_string(), out))
    }

    fn to_frontmatter(&self) -> String {
        match self.command_line() {
            Some((command, args)) => format!("{command} {}", args.join(" ")),
            None => String::new(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Package { .. } => "mcp-stdio",
            Self::Url { .. } => "mcp-http",
        }
    }

    fn to_entry(&self) -> Value {
        match self {
            Self::Package {
                command,
                package,
                version,
                args,
            } => {
                let mut run = json!({
                    "command": command.as_str(),
                    "package": package,
                    "version": version,
                });
                if !args.is_empty() {
                    run["args"] = json!(args);
                }
                run
            }
            Self::Url { url } => json!({ "url": url }),
        }
    }
}

/// `example.com`, `*.example.com`, optionally with a port
pub fn valid_host(host: &str) -> bool {
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    };
    if port.is_some_and(|p| p.is_empty() || p.len() > 5 || !p.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }
    let name = name.strip_prefix("*.").unwrap_or(name);
    let labels: Vec<&str> = name.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

fn env_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

impl ConnectorPackage {
    /// Parses `connector.md`; `id` is its folder name
    pub fn parse(id: &str, content: &str) -> DomainResult<Self> {
        let id = ConnectorId::parse(id.to_string())?;
        let (frontmatter, body) =
            split_frontmatter(content).map_err(|e| invalid("frontmatter", e))?;
        let frontmatter = frontmatter.ok_or_else(|| invalid("frontmatter", "is missing"))?;
        let front: Frontmatter =
            serde_yaml::from_str(frontmatter).map_err(|e| DomainError::parse(FILE, e))?;
        let run = match (front.run.as_deref(), front.url.as_deref()) {
            (Some(run), None) => RunSpec::parse_command(run).map_err(|e| invalid("run", e))?,
            (None, Some(url))
                if url.starts_with("https://") && !url.contains(char::is_whitespace) =>
            {
                RunSpec::Url {
                    url: url.to_string(),
                }
            }
            (None, Some(_)) => return Err(invalid("url", "must be an https:// address")),
            _ => return Err(invalid("run", "set exactly one of run (a package) or url")),
        };
        let auth = match front.auth {
            None => AuthSpec {
                scopes: front.scopes,
                ..AuthSpec::default()
            },
            Some(AuthField::Kind(kind)) => AuthSpec {
                kind,
                scopes: front.scopes,
                ..AuthSpec::default()
            },
            Some(AuthField::Full(auth)) => AuthSpec {
                kind: auth.kind,
                env: auth.env,
                scopes: if auth.scopes.is_empty() {
                    front.scopes
                } else {
                    auth.scopes
                },
                label: auth.label,
            },
        };
        let package = Self {
            id,
            name: front.name.trim().to_string(),
            description: front.description.trim().to_string(),
            version: semver::Version::parse(front.version.trim().trim_start_matches('v'))
                .map_err(|e| invalid("version", format!("isn't x.y.z: {e}")))?,
            run,
            image: front
                .image
                .map(|i| i.trim().to_string())
                .filter(|i| !i.is_empty()),
            placement: front.placement,
            auth,
            hosts: front
                .hosts
                .into_iter()
                .map(|h| h.trim().to_ascii_lowercase())
                .collect(),
            tools: front
                .tools
                .into_iter()
                .map(|t| PackageTool {
                    name: t.name,
                    description: t.description.trim().to_string(),
                    access: t.access,
                })
                .collect(),
            reads: front.reads,
            writes: front.writes,
            tags: front.tags,
            instructions: body.trim().to_string(),
        };
        package.check()?;
        Ok(package)
    }

    fn check(&self) -> DomainResult<()> {
        if self.name.is_empty() {
            return Err(invalid("name", "is required"));
        }
        if self.description.is_empty() || self.description.chars().count() > 200 {
            return Err(invalid("description", "must be 1-200 characters"));
        }
        if matches!(self.run, RunSpec::Url { .. }) {
            if self.image.is_some() {
                return Err(invalid(
                    "image",
                    "an HTTP server runs elsewhere: it has no image",
                ));
            }
            if self.placement == Placement::Host {
                return Err(invalid("placement", "an HTTP server has no placement"));
            }
        }
        if let Some(host) = self.hosts.iter().find(|h| !valid_host(h)) {
            return Err(invalid("hosts", format!("'{host}' isn't a host name")));
        }
        if let Some(var) = self.auth.env.iter().find(|v| !env_name(v)) {
            return Err(invalid(
                "auth.env",
                format!("'{var}' isn't an environment variable name"),
            ));
        }
        if self.auth.kind == AuthKind::ApiKey
            && self.auth.env.is_empty()
            && matches!(self.run, RunSpec::Package { .. })
        {
            return Err(invalid(
                "auth.env",
                "an API key needs the variable it goes into",
            ));
        }
        let mut names = std::collections::HashSet::new();
        for tool in &self.tools {
            let ok = !tool.name.is_empty()
                && tool.name.len() <= 64
                && tool
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !ok {
                return Err(invalid(
                    "tools",
                    format!("'{}' isn't a tool name", tool.name),
                ));
            }
            if !names.insert(tool.name.as_str()) {
                return Err(invalid("tools", format!("'{}' is listed twice", tool.name)));
            }
            if tool.description.is_empty() || tool.description.chars().count() > 200 {
                return Err(invalid(
                    "tools",
                    format!("'{}' needs a 1-200 character description", tool.name),
                ));
            }
        }
        if self.tools.is_empty() {
            return Err(invalid(
                "tools",
                "list every tool, with read or write access",
            ));
        }
        if self.instructions.split_whitespace().count() > MAX_INSTRUCTION_WORDS {
            return Err(invalid(
                "instructions",
                format!("keep them to {MAX_INSTRUCTION_WORDS} words or fewer"),
            ));
        }
        Ok(())
    }

    /// What the public registry refuses (decision 2): a connector that runs
    /// on the host
    pub fn check_public(&self) -> Result<(), String> {
        if self.placement == Placement::Host {
            return Err(format!(
                "'{}' runs on the host: the public registry lists container connectors only. \
                 Add host connectors by hand or from a company registry.",
                self.id
            ));
        }
        Ok(())
    }

    /// The access a tool needs. Tools the file doesn't list count as write
    pub fn tool_access(&self, tool: &str) -> GrantAccess {
        self.tools
            .iter()
            .find(|t| t.name == tool)
            .map_or(GrantAccess::Write, |t| t.access)
    }

    /// The keyring entry of the API key
    pub fn key_entry(&self) -> String {
        format!("nandi.connector.{}.key", self.id)
    }

    /// The `connectors.yaml` declaration that runs it. Read tools are
    /// `safe_tools`; everything else asks for each call's approval
    pub fn declaration(&self, source: Option<&PackageSource>) -> Connector {
        let mut config: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();
        let text = |s: &str| serde_yaml::Value::from(s);
        let list =
            |items: &[String]| serde_yaml::Value::Sequence(items.iter().map(|s| text(s)).collect());
        config.insert("name".into(), text(&self.name));
        config.insert("description".into(), text(&self.description));
        match &self.run {
            RunSpec::Package { .. } => {
                let (command, args) = self.run.command_line().unwrap_or_default();
                config.insert("command".into(), text(&command));
                config.insert("args".into(), list(&args));
            }
            RunSpec::Url { url } => {
                config.insert("url".into(), text(url));
            }
        }
        if let Some(image) = &self.image {
            config.insert("image".into(), text(image));
        }
        config.insert("network".into(), list(&self.hosts));
        let safe: Vec<String> = self
            .tools
            .iter()
            .filter(|t| t.access == GrantAccess::Read)
            .map(|t| t.name.clone())
            .collect();
        config.insert("safe_tools".into(), list(&safe));
        let mut package = serde_yaml::Mapping::new();
        package.insert(text("version"), text(&self.version.to_string()));
        if let Some(source) = source {
            package.insert(text("registry"), text(&source.registry));
            package.insert(text("publisher"), text(&source.publisher));
            package.insert(text("commit"), text(&source.commit));
        }
        config.insert("package".into(), serde_yaml::Value::Mapping(package));
        let mut secret_refs = BTreeMap::new();
        if self.auth.kind == AuthKind::ApiKey {
            let entry =
                SecretRef::parse(self.key_entry()).expect("connector key entries are valid");
            match &self.run {
                RunSpec::Package { .. } => {
                    for var in &self.auth.env {
                        secret_refs.insert(var.clone(), entry.clone());
                    }
                }
                RunSpec::Url { .. } => {
                    secret_refs.insert("bearer_token".to_string(), entry);
                }
            }
        }
        Connector {
            id: self.id.clone(),
            kind: "mcp".to_string(),
            config,
            secret_refs,
        }
    }

    /// The registry entry for a release (`connector.schema.json`)
    pub fn entry(&self, release: &PackageRelease) -> Value {
        let mut auth = json!({ "type": self.auth.kind.as_str() });
        if !self.auth.env.is_empty() {
            auth["env"] = json!(self.auth.env);
        }
        if !self.auth.scopes.is_empty() {
            auth["scopes"] = json!(self.auth.scopes);
        }
        if let Some(label) = &self.auth.label {
            auth["label"] = json!(label);
        }
        let mut entry = json!({
            "id": self.id.as_str(),
            "name": self.name,
            "description": self.description,
            "publisher": release.publisher,
            "repo": release.repo,
            "path": release.path,
            "latest_version": self.version.to_string(),
            "ref": release.tag,
            "commit": release.commit,
            "min_nandi_version": release.min_nandi_version,
            "tags": if self.tags.is_empty() { vec!["connector".to_string()] } else { self.tags.clone() },
            "kind": self.run.kind(),
            "run": self.run.to_entry(),
            "placement": match self.placement { Placement::Container => "container", Placement::Host => "host" },
            "auth": auth,
            "tools": self.tools.iter().map(|t| json!({
                "name": t.name, "description": t.description, "access": t.access.as_str(),
            })).collect::<Vec<_>>(),
            "data": { "read": self.reads, "write": self.writes },
            "hosts": self.hosts,
        });
        if let Some(image) = &self.image {
            entry["image"] = json!(image);
        }
        if let Some(signature) = &release.signature {
            entry["signature"] = json!(signature);
        }
        entry
    }

    /// Whether a registry entry describes exactly this file: an install
    /// refuses an entry that promises something else than what runs
    pub fn matches_entry(&self, entry: &Value) -> Result<(), String> {
        let ours = self.entry(&PackageRelease::default());
        for key in [
            "id",
            "latest_version",
            "kind",
            "run",
            "image",
            "placement",
            "auth",
            "tools",
            "hosts",
            "data",
        ] {
            let theirs = entry.get(key).unwrap_or(&Value::Null);
            let mine = ours.get(key).unwrap_or(&Value::Null);
            if theirs != mine {
                return Err(format!(
                    "The registry entry's '{key}' doesn't match the connector's connector.md"
                ));
            }
        }
        Ok(())
    }

    pub fn to_markdown(&self) -> String {
        let mut auth = serde_json::Map::new();
        auth.insert("type".into(), json!(self.auth.kind.as_str()));
        if !self.auth.env.is_empty() {
            auth.insert("env".into(), json!(self.auth.env));
        }
        if !self.auth.scopes.is_empty() {
            auth.insert("scopes".into(), json!(self.auth.scopes));
        }
        if let Some(label) = &self.auth.label {
            auth.insert("label".into(), json!(label));
        }
        let auth = if auth.len() == 1 {
            json!(self.auth.kind.as_str())
        } else {
            Value::Object(auth)
        };
        let front = FrontmatterOut {
            name: &self.name,
            description: &self.description,
            version: self.version.to_string(),
            run: matches!(self.run, RunSpec::Package { .. }).then(|| self.run.to_frontmatter()),
            url: match &self.run {
                RunSpec::Url { url } => Some(url),
                RunSpec::Package { .. } => None,
            },
            image: self.image.as_deref(),
            placement: self.placement,
            auth,
            hosts: &self.hosts,
            tools: self
                .tools
                .iter()
                .map(|t| json!({"name": t.name, "description": t.description, "access": t.access.as_str()}))
                .collect(),
            reads: &self.reads,
            writes: &self.writes,
            tags: &self.tags,
        };
        let yaml = serde_yaml::to_string(&front).unwrap_or_default();
        format!("---\n{yaml}---\n\n{}\n", self.instructions.trim())
    }
}

/// Where an installed package came from (kept in `config.package`)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageSource {
    pub registry: String,
    pub publisher: String,
    pub commit: String,
}

/// A release being published
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageRelease {
    pub publisher: String,
    /// `github.com/<owner>/<repo>`, or a local path
    pub repo: String,
    /// The folder holding connector.md
    pub path: String,
    pub tag: String,
    pub commit: String,
    pub min_nandi_version: String,
    pub signature: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const FETCH: &str = r#"---
name: Web page fetch
description: Fetch a web page and read it as Markdown.
version: 1.0.0
run: uvx mcp-server-fetch==2025.4.7 --ignore-robots-txt
auth: none
hosts: [example.com, "*.example.org"]
tools:
  - { name: fetch, description: Fetch a URL and return its text, access: read }
  - { name: post, description: Post a form, access: write }
reads: [Web pages you ask it to open]
writes: [Forms you ask it to send]
tags: [web]
---
Use it to read a page the person names.
"#;

    #[test]
    fn parses_the_documented_format() {
        let p = ConnectorPackage::parse("fetch", FETCH).unwrap();
        assert_eq!(p.name, "Web page fetch");
        assert_eq!(p.version.to_string(), "1.0.0");
        assert_eq!(
            p.run,
            RunSpec::Package {
                command: PackageRunner::Uvx,
                package: "mcp-server-fetch".into(),
                version: "2025.4.7".into(),
                args: vec!["--ignore-robots-txt".into()],
            }
        );
        assert_eq!(p.placement, Placement::Container);
        assert_eq!(p.tool_access("fetch"), GrantAccess::Read);
        assert_eq!(p.tool_access("post"), GrantAccess::Write);
        assert_eq!(p.tool_access("unlisted"), GrantAccess::Write);
        assert_eq!(p.instructions, "Use it to read a page the person names.");
        // The file round-trips through its own rendering
        assert_eq!(
            ConnectorPackage::parse("fetch", &p.to_markdown()).unwrap(),
            p
        );
    }

    #[test]
    fn versions_must_be_exact() {
        for good in [
            "npx -y @acme/tracker-mcp@1.2.3",
            "npx tracker@0.1.0-beta.2",
            "uvx pkg==1.2",
        ] {
            assert!(RunSpec::parse_command(good).is_ok(), "{good}");
        }
        for bad in [
            "npx @acme/tracker-mcp",
            "npx @acme/tracker-mcp@latest",
            "npx tracker@^1.2.3",
            "npx tracker@1.2",
            "npx tracker@1.x",
            "uvx pkg",
            "uvx pkg>=1.0",
            "uvx pkg==*",
            "node server.js",
            "npx 'bad name'@1.0.0",
        ] {
            assert!(RunSpec::parse_command(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn refuses_what_the_install_dialog_couldnt_describe() {
        let with = |from: &str, to: &str| FETCH.replace(from, to);
        for (content, expected) in [
            (with("hosts: [example.com, \"*.example.org\"]", "hosts: [\"*\"]"), "hosts"),
            (with(", access: read }", " }"), "connector.md"),
            (with("tools:\n  - { name: fetch, description: Fetch a URL and return its text, access: read }\n  - { name: post, description: Post a form, access: write }\n", "tools: []\n"), "tools"),
            (with("run: uvx mcp-server-fetch==2025.4.7 --ignore-robots-txt", "url: http://insecure.example.com/mcp"), "url"),
            (with("auth: none", "auth: api_key"), "auth.env"),
            (with("tags: [web]", "tags: [web]\nsecret: x"), "connector.md"),
        ] {
            let error = ConnectorPackage::parse("fetch", &content).unwrap_err().to_string();
            assert!(error.contains(expected), "{expected}: {error}");
        }
        let http_with_image = FETCH.replace(
            "run: uvx mcp-server-fetch==2025.4.7 --ignore-robots-txt",
            "url: https://mcp.example.com/mcp\nimage: ghcr.io/x/y:1",
        );
        assert!(ConnectorPackage::parse("fetch", &http_with_image).is_err());
    }

    #[test]
    fn host_connectors_are_not_public() {
        let host = FETCH.replace("auth: none", "auth: none\nplacement: host");
        let p = ConnectorPackage::parse("fetch", &host).unwrap();
        assert!(p.check_public().unwrap_err().contains("runs on the host"));
        assert!(ConnectorPackage::parse("fetch", FETCH)
            .unwrap()
            .check_public()
            .is_ok());
    }

    #[test]
    fn the_declaration_runs_the_pinned_package_with_read_tools_safe() {
        let api = FETCH.replace(
            "auth: none",
            "auth: {type: api_key, env: [FETCH_TOKEN], label: Fetch key}",
        );
        let p = ConnectorPackage::parse("fetch", &api).unwrap();
        let decl = p.declaration(None);
        assert_eq!(decl.kind, "mcp");
        let yaml = serde_yaml::to_string(&decl).unwrap();
        assert!(yaml.contains("command: uvx"), "{yaml}");
        assert!(yaml.contains("mcp-server-fetch==2025.4.7"), "{yaml}");
        assert!(yaml.contains("safe_tools:\n  - fetch"), "{yaml}");
        assert_eq!(
            decl.secret_refs["FETCH_TOKEN"].as_str(),
            "nandi.connector.fetch.key"
        );
        // Valid in connectors.yaml
        crate::services::domain::ConnectorManifest::new(vec![decl]).unwrap();
    }

    #[test]
    fn the_entry_matches_its_file_and_nothing_else() {
        let p = ConnectorPackage::parse("fetch", FETCH).unwrap();
        let release = PackageRelease {
            publisher: "alice".into(),
            repo: "github.com/alice/nandi-connectors".into(),
            path: "connectors/fetch".into(),
            tag: "fetch-v1.0.0".into(),
            commit: "a".repeat(40),
            min_nandi_version: "0.1.0".into(),
            signature: Some("ed25519:abc=".into()),
        };
        let entry = p.entry(&release);
        assert_eq!(entry["kind"], "mcp-stdio");
        assert_eq!(entry["run"]["package"], "mcp-server-fetch");
        assert_eq!(entry["tools"][1]["access"], "write");
        assert_eq!(entry["data"]["write"][0], "Forms you ask it to send");
        p.matches_entry(&entry).unwrap();
        let mut changed = entry.clone();
        changed["hosts"] = json!(["evil.example.com"]);
        assert!(p.matches_entry(&changed).unwrap_err().contains("hosts"));
        let mut changed = entry;
        changed["tools"][1]["access"] = json!("read");
        assert!(p.matches_entry(&changed).unwrap_err().contains("tools"));
    }
}
