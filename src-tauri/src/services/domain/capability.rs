//! What an agent may do: tools, pilots, network hosts, and AWS services.
//! Declared in skill.md frontmatter:
//!
//! ```yaml
//! allowed-tools: [shell, read_file]   # omitted: every runtime tool
//! requires_approval: [shell]          # omitted: shell + write_file
//! pilots: [browser]
//! network:
//!   allow:
//!     - host: api.github.com
//!       methods: [GET]
//! aws:
//!   mode: proxy
//!   services:
//!     - service: s3
//!       actions: [GetObject]
//!       resources: ["arn:aws:s3:::reports/*"]
//! serves:                             # what it does for other agents
//!   - name: summarize-paper
//!     description: Summarizes an arXiv paper
//!     input_schema: { type: object }
//! accepts_delegation: true            # omitted: false
//! max_delegation_concurrent: 2        # omitted: 1 when accepting
//! ```
//!
//! The egress proxy enforces the network and AWS policies
//! (`services::egress::capabilities` converts from these types).

use serde::{Deserialize, Serialize};

use super::{DomainError, DomainResult, PilotRef};

// The wire shape lives in `nandi-protocol`
pub use nandi_protocol::ServesCapability;

/// Tools that need approval when skill.md doesn't say otherwise: anything
/// that runs code or changes files.
pub const DEFAULT_REQUIRES_APPROVAL: &[&str] = &["shell", "write_file"];

/// The built-in tool an agent calls to delegate to another agent (wave 2A,
/// Track C). Cross-user calls need approval unless skill.md says otherwise.
pub const DELEGATION_TOOL: &str = "delegate_to_agent";

/// Concurrent delegated tasks an accepting agent runs when skill.md
/// doesn't set `max_delegation_concurrent`.
pub const DEFAULT_MAX_DELEGATION_CONCURRENT: u32 = 1;

/// What an agent may do with a pilot without asking, and what it fills
/// forms with (skill.md: a `pilots:` item written as a mapping,
/// `{id: browser, allow: [...], values: {...}, secrets: {...}}`)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PilotOptions {
    /// Kinds of action that run without asking ([`PILOT_ALLOWS`])
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
    /// Plain values the agent may type into forms (it reads them)
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub values: std::collections::BTreeMap<String, String>,
    /// Secrets by the name the agent types (`SITE_PASSWORD`) → their
    /// keychain entry. The pilot puts the value in; the agent never sees it.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub secrets: std::collections::BTreeMap<String, super::settings::SecretRef>,
}

/// What `PilotOptions::allow` may list: clicking and typing, submitting a
/// form, typing a secret, uploading or downloading a file, paying
pub const PILOT_ALLOWS: &[&str] = &["interact", "submit", "secrets", "upload", "pay"];

impl PilotOptions {
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.values.is_empty() && self.secrets.is_empty()
    }

    pub fn allows(&self, kind: &str) -> bool {
        self.allow.iter().any(|a| a == kind)
    }

    fn validated(mut self, pilot: &str) -> DomainResult<Self> {
        for kind in &mut self.allow {
            *kind = kind.trim().to_ascii_lowercase();
            if !PILOT_ALLOWS.contains(&kind.as_str()) {
                return Err(DomainError::invalid(
                    "capabilities",
                    "pilots",
                    format!(
                        "'{pilot}' allows '{kind}', which isn't one of: {}",
                        PILOT_ALLOWS.join(", ")
                    ),
                ));
            }
        }
        for name in self.secrets.keys() {
            let ok = name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            if !ok {
                return Err(DomainError::invalid(
                    "capabilities",
                    "pilots",
                    format!("'{pilot}' names a secret '{name}': use capitals, digits and '_', like SITE_PASSWORD"),
                ));
            }
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CapabilitySet {
    tools: ToolPolicy,
    pilots: Vec<PilotRef>,
    /// Per pilot (by ID), for the ones skill.md gives options
    pilot_options: std::collections::BTreeMap<String, PilotOptions>,
    network: NetworkPolicy,
    aws: Option<AwsPolicy>,
    /// `None` when skill.md declares none of the delegation keys (boxed:
    /// most agents don't, and `Agent` stays small)
    delegation: Option<Box<Delegation>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Delegation {
    /// What this agent does for other agents (`serves:`)
    serves: Vec<ServesCapability>,
    /// Whether remote agents may send it tasks
    accepts: bool,
    max_concurrent: Option<u32>,
    /// Capability IDs from the `provides:` shorthand that have no `serves:`
    /// entry of their own (they are in `serves` too, as bare entries; kept
    /// apart so skill.md is written back the way it was)
    provides: Vec<String>,
}

impl CapabilitySet {
    pub fn new(
        tools: ToolPolicy,
        pilots: Vec<PilotRef>,
        network: NetworkPolicy,
        aws: Option<AwsPolicy>,
    ) -> DomainResult<Self> {
        let mut seen = std::collections::HashSet::new();
        if let Some(dup) = pilots.iter().find(|p| !seen.insert(p.as_str())) {
            return Err(DomainError::invalid(
                "capabilities",
                "pilots",
                format!("lists '{dup}' twice"),
            ));
        }
        Ok(Self {
            tools,
            pilots,
            pilot_options: Default::default(),
            network: network.normalized()?,
            aws: aws.map(AwsPolicy::normalized).transpose()?,
            delegation: None,
        })
    }

    /// Adds what the agent serves and whether it accepts delegated tasks.
    pub fn with_delegation(
        mut self,
        serves: Vec<ServesCapability>,
        accepts_delegation: bool,
        max_delegation_concurrent: Option<u32>,
    ) -> DomainResult<Self> {
        let mut seen = std::collections::HashSet::new();
        let mut cleaned = Vec::with_capacity(serves.len());
        for mut capability in serves {
            capability.name = capability.name.trim().to_string();
            capability.description = capability.description.trim().to_string();
            if capability.name.is_empty() || capability.name.chars().any(char::is_whitespace) {
                return Err(DomainError::invalid(
                    "capabilities",
                    "serves",
                    format!("has an invalid name '{}'", capability.name),
                ));
            }
            if !seen.insert(capability.name.clone()) {
                return Err(DomainError::invalid(
                    "capabilities",
                    "serves",
                    format!("lists '{}' twice", capability.name),
                ));
            }
            cleaned.push(capability);
        }
        if max_delegation_concurrent == Some(0) {
            return Err(DomainError::invalid(
                "capabilities",
                "max_delegation_concurrent",
                "must be at least 1",
            ));
        }
        let delegation = Delegation {
            serves: cleaned,
            accepts: accepts_delegation,
            max_concurrent: max_delegation_concurrent,
            provides: Vec::new(),
        };
        self.delegation = (delegation != Delegation::default()).then(|| Box::new(delegation));
        Ok(self)
    }

    /// Adds the `provides: [id, ...]` shorthand (wave A): each ID not
    /// already in `serves:` becomes a bare serves entry (same name rules).
    pub fn with_provides(mut self, provides: Vec<String>) -> DomainResult<Self> {
        let mut delegation = self.delegation.take().map(|d| *d).unwrap_or_default();
        for id in provides {
            let id = id.trim().to_string();
            if id.is_empty() || id.chars().any(char::is_whitespace) {
                return Err(DomainError::invalid(
                    "capabilities",
                    "provides",
                    format!("has an invalid capability ID '{id}'"),
                ));
            }
            if delegation.provides.contains(&id) {
                return Err(DomainError::invalid(
                    "capabilities",
                    "provides",
                    format!("lists '{id}' twice"),
                ));
            }
            if delegation.serves.iter().any(|s| s.name == id) {
                continue;
            }
            delegation.serves.push(ServesCapability {
                name: id.clone(),
                description: String::new(),
                input_schema: None,
                output_schema: None,
            });
            delegation.provides.push(id);
        }
        self.delegation = (delegation != Delegation::default()).then(|| Box::new(delegation));
        Ok(self)
    }

    /// The `serves:` entries as written (without the `provides:` ones)
    pub fn declared_serves(&self) -> Vec<&ServesCapability> {
        let provides = self.provides();
        self.serves()
            .iter()
            .filter(|s| !provides.contains(&s.name))
            .collect()
    }

    /// The `provides:` IDs that have no `serves:` entry
    pub fn provides(&self) -> &[String] {
        self.delegation.as_ref().map_or(&[], |d| &d.provides)
    }

    pub fn tools(&self) -> &ToolPolicy {
        &self.tools
    }

    pub fn pilots(&self) -> &[PilotRef] {
        &self.pilots
    }

    /// The options skill.md gives a pilot it lists, if any
    pub fn pilot_options(&self, pilot: &str) -> Option<&PilotOptions> {
        self.pilot_options.get(pilot)
    }

    /// Sets each listed pilot's options (empty ones are dropped)
    pub fn with_pilot_options(
        mut self,
        options: std::collections::BTreeMap<String, PilotOptions>,
    ) -> DomainResult<Self> {
        for (pilot, options) in options {
            if options.is_empty() {
                continue;
            }
            let options = options.validated(&pilot)?;
            self.pilot_options.insert(pilot, options);
        }
        Ok(self)
    }

    pub fn network(&self) -> &NetworkPolicy {
        &self.network
    }

    /// Adds `hosts` to `network.allow` (any method). `false`: each was
    /// there already
    pub fn allow_hosts(&mut self, hosts: &[String]) -> DomainResult<bool> {
        let mut network = self.network.clone();
        for host in hosts {
            let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
            if !network
                .allow
                .iter()
                .any(|rule| rule.host == host && rule.methods.is_empty())
            {
                network.allow.push(HostRule {
                    host,
                    methods: Vec::new(),
                });
            }
        }
        if network == self.network {
            return Ok(false);
        }
        self.network = network.normalized()?;
        Ok(true)
    }

    pub fn aws(&self) -> Option<&AwsPolicy> {
        self.aws.as_ref()
    }

    pub fn serves(&self) -> &[ServesCapability] {
        self.delegation.as_ref().map_or(&[], |d| &d.serves)
    }

    pub fn accepts_delegation(&self) -> bool {
        self.delegation.as_ref().is_some_and(|d| d.accepts)
    }

    /// As written in skill.md (`None` if the key was omitted).
    pub fn max_delegation_concurrent(&self) -> Option<u32> {
        self.delegation.as_ref().and_then(|d| d.max_concurrent)
    }

    /// Concurrent delegated tasks allowed, with the default applied
    pub fn delegation_limit(&self) -> u32 {
        self.max_delegation_concurrent()
            .unwrap_or(DEFAULT_MAX_DELEGATION_CONCURRENT)
    }
}

/// Which runtime tools the model is offered, and which need approval.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolPolicy {
    /// `None`: every runtime tool
    allowed: Option<Vec<String>>,
    /// `None`: `DEFAULT_REQUIRES_APPROVAL`
    requires_approval: Option<Vec<String>>,
}

impl ToolPolicy {
    pub fn new(allowed: Option<Vec<String>>, requires_approval: Option<Vec<String>>) -> Self {
        let clean = |tools: Vec<String>| -> Vec<String> {
            tools
                .into_iter()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        };
        Self {
            allowed: allowed.map(clean),
            requires_approval: requires_approval.map(clean),
        }
    }

    pub fn allows(&self, tool: &str) -> bool {
        self.allowed
            .as_ref()
            .is_none_or(|allowed| allowed.iter().any(|t| t == tool))
    }

    pub fn requires_approval_for(&self, tool: &str) -> bool {
        match &self.requires_approval {
            Some(tools) => tools.iter().any(|t| t == tool),
            None => DEFAULT_REQUIRES_APPROVAL.contains(&tool),
        }
    }

    /// The tools needing approval, with the default applied.
    pub fn approval_list(&self) -> Vec<String> {
        self.requires_approval.clone().unwrap_or_else(|| {
            DEFAULT_REQUIRES_APPROVAL
                .iter()
                .map(ToString::to_string)
                .collect()
        })
    }

    pub fn allowed(&self) -> Option<&[String]> {
        self.allowed.as_deref()
    }

    /// As written in skill.md (`None` if the key was omitted).
    pub fn declared_approvals(&self) -> Option<&[String]> {
        self.requires_approval.as_deref()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPolicy {
    #[serde(default)]
    pub allow: Vec<HostRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRule {
    /// `api.example.com`, `*.example.com` (subdomains only), or `*`; an
    /// optional `:port` allows a non-default port
    pub host: String,
    /// Empty means any method
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
}

impl NetworkPolicy {
    /// Normalizes case and rejects rules that can never match as intended.
    pub fn normalized(mut self) -> DomainResult<Self> {
        for rule in &mut self.allow {
            rule.host = rule.host.trim().trim_end_matches('.').to_ascii_lowercase();
            let (name, port) = split_port(&rule.host);
            let wildcard_ok = name == "*" || !name[name.starts_with("*.") as usize..].contains('*');
            if name.is_empty() || !wildcard_ok || port.is_some_and(|p| p.parse::<u16>().is_err()) {
                return Err(DomainError::invalid(
                    "capabilities",
                    "network.allow",
                    format!("has an invalid host '{}'", rule.host),
                ));
            }
            for method in &mut rule.methods {
                *method = method.trim().to_ascii_uppercase();
            }
        }
        Ok(self)
    }
}

/// `host:port` → (`host`, `Some("port")`). IPv6 literals aren't supported.
pub fn split_port(host: &str) -> (&str, Option<&str>) {
    match host.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwsPolicy {
    #[serde(default)]
    pub mode: AwsMode,
    #[serde(default)]
    pub services: Vec<AwsGrant>,
    /// The AWS profile the agent's calls run as; `None`: the access key in
    /// Settings, else the sign-in profile there
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// How an agent reaches AWS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AwsMode {
    /// Only through the host's `/aws` proxy; no credentials in the container
    #[default]
    Proxy,
    /// Short-lived credentials scoped to the grants are injected at start
    Sts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwsGrant {
    pub service: String,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub resources: Vec<String>,
}

impl AwsPolicy {
    pub fn normalized(mut self) -> DomainResult<Self> {
        self.profile = self
            .profile
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty());
        if self
            .profile
            .as_deref()
            .is_some_and(|p| p.chars().any(char::is_whitespace))
        {
            return Err(DomainError::invalid(
                "capabilities",
                "aws.profile",
                "must be a profile name without spaces",
            ));
        }
        for grant in &mut self.services {
            grant.service = grant.service.trim().to_ascii_lowercase();
            if grant.service.is_empty() {
                return Err(DomainError::invalid(
                    "capabilities",
                    "aws.services",
                    "has an entry without 'service'",
                ));
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(host: &str) -> HostRule {
        HostRule {
            host: host.into(),
            methods: vec!["get".into()],
        }
    }

    #[test]
    fn normalizes_and_validates_hosts() {
        let policy = NetworkPolicy {
            allow: vec![rule(" API.Example.com. "), rule("*.example.com:8443")],
        }
        .normalized()
        .unwrap();
        assert_eq!(policy.allow[0].host, "api.example.com");
        assert_eq!(policy.allow[0].methods, ["GET"]);
        for bad in ["a.*.com", "", "host:99999", "*x.com"] {
            let policy = NetworkPolicy {
                allow: vec![rule(bad)],
            };
            assert!(policy.normalized().is_err(), "{bad}");
        }
    }

    #[test]
    fn tool_policy_defaults() {
        let default = ToolPolicy::default();
        assert!(default.allows("anything"));
        assert!(default.requires_approval_for("shell"));
        assert!(!default.requires_approval_for("list_dir"));
        let explicit = ToolPolicy::new(Some(vec!["shell".into()]), Some(vec![]));
        assert!(explicit.allows("shell") && !explicit.allows("write_file"));
        assert!(!explicit.requires_approval_for("shell"));
    }

    #[test]
    fn rejects_duplicate_pilots_and_empty_aws_services() {
        let pilot = PilotRef::parse("browser").unwrap();
        let dup = CapabilitySet::new(
            ToolPolicy::default(),
            vec![pilot.clone(), pilot],
            NetworkPolicy::default(),
            None,
        );
        assert!(dup.is_err());
        let aws = AwsPolicy {
            mode: AwsMode::Sts,
            services: vec![AwsGrant {
                service: " ".into(),
                actions: vec![],
                resources: vec![],
            }],
            profile: None,
        };
        assert!(CapabilitySet::new(
            ToolPolicy::default(),
            vec![],
            NetworkPolicy::default(),
            Some(aws)
        )
        .is_err());
    }

    fn serves(name: &str) -> ServesCapability {
        ServesCapability {
            name: name.into(),
            description: " Does it ".into(),
            input_schema: None,
            output_schema: None,
        }
    }

    #[test]
    fn delegation_defaults_off_and_validates() {
        let base = CapabilitySet::default();
        assert!(!base.accepts_delegation() && base.serves().is_empty());
        assert_eq!(base.delegation_limit(), DEFAULT_MAX_DELEGATION_CONCURRENT);

        let set = base
            .clone()
            .with_delegation(vec![serves(" summarize ")], true, Some(3))
            .unwrap();
        assert_eq!(set.serves()[0].name, "summarize");
        assert_eq!(set.serves()[0].description, "Does it");
        assert_eq!(set.delegation_limit(), 3);

        for (list, max) in [
            (vec![serves("a"), serves("a")], None),
            (vec![serves("two words")], None),
            (vec![serves(" ")], None),
            (vec![], Some(0)),
        ] {
            assert!(base.clone().with_delegation(list, true, max).is_err());
        }
    }
}
