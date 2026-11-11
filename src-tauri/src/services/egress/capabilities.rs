//! Per-agent egress capabilities, declared in skill.md frontmatter:
//!
//! ```yaml
//! aws:
//!   mode: proxy            # or `sts`: the container gets scoped credentials
//!   profile: work          # optional: the AWS profile its calls run as
//!   services:
//!     - service: s3
//!       actions: [GetObject, ListObjects]
//!       resources: ["arn:aws:s3:::reports", "arn:aws:s3:::reports/*"]
//! network:
//!   allow:
//!     - host: api.github.com
//!       methods: [GET]
//!     - host: "*.example.com"
//! ```
//!
//! Everything not listed is denied. Omitting `methods` allows any method;
//! omitting `resources` allows any resource of the listed actions.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub aws: Option<AwsCapabilities>,
    #[serde(default)]
    pub network: NetworkCapabilities,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AwsCapabilities {
    #[serde(default)]
    pub mode: AwsMode,
    #[serde(default)]
    pub services: Vec<AwsServiceGrant>,
    /// The AWS profile its calls run as; `None`: Settings' access key,
    /// else the sign-in profile
    #[serde(default)]
    pub profile: Option<String>,
}

/// How an agent reaches AWS.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AwsMode {
    /// Only through the host's `/aws` proxy; no credentials in the container
    #[default]
    Proxy,
    /// Short-lived credentials scoped to the grants are injected at start
    Sts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AwsServiceGrant {
    /// e.g. `s3`
    pub service: String,
    /// API action names, e.g. `GetObject`; `*` for all
    #[serde(default)]
    pub actions: Vec<String>,
    /// ARN patterns (`*` wildcards); empty means any resource
    #[serde(default)]
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NetworkCapabilities {
    #[serde(default)]
    pub allow: Vec<HostRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HostRule {
    /// `api.example.com`, `*.example.com` (subdomains only), or `*`; an
    /// optional `:port` allows a non-default port
    pub host: String,
    /// Empty means any method
    #[serde(default)]
    pub methods: Vec<String>,
}

/// An agent's declared capabilities (validated by the domain model) as the
/// policy the proxy enforces.
impl From<&crate::services::domain::CapabilitySet> for Capabilities {
    fn from(set: &crate::services::domain::CapabilitySet) -> Self {
        use crate::services::domain::capability as domain;
        Self {
            aws: set.aws().map(|aws| AwsCapabilities {
                mode: match aws.mode {
                    domain::AwsMode::Proxy => AwsMode::Proxy,
                    domain::AwsMode::Sts => AwsMode::Sts,
                },
                services: aws
                    .services
                    .iter()
                    .map(|grant| AwsServiceGrant {
                        service: grant.service.clone(),
                        actions: grant.actions.clone(),
                        resources: grant.resources.clone(),
                    })
                    .collect(),
                profile: aws.profile.clone(),
            }),
            network: NetworkCapabilities {
                allow: set
                    .network()
                    .allow
                    .iter()
                    .map(|rule| HostRule {
                        host: rule.host.clone(),
                        methods: rule.methods.clone(),
                    })
                    .collect(),
            },
        }
    }
}

impl Capabilities {
    /// Normalizes case and rejects rules that can never match as intended
    /// (the rules live in `domain::capability`).
    #[cfg(test)]
    pub fn validate(self) -> Result<Self, String> {
        use crate::services::domain::capability as domain;
        let network = domain::NetworkPolicy {
            allow: self
                .network
                .allow
                .into_iter()
                .map(|rule| domain::HostRule {
                    host: rule.host,
                    methods: rule.methods,
                })
                .collect(),
        };
        let aws = self.aws.map(|aws| domain::AwsPolicy {
            mode: match aws.mode {
                AwsMode::Proxy => domain::AwsMode::Proxy,
                AwsMode::Sts => domain::AwsMode::Sts,
            },
            services: aws
                .services
                .into_iter()
                .map(|grant| domain::AwsGrant {
                    service: grant.service,
                    actions: grant.actions,
                    resources: grant.resources,
                })
                .collect(),
            profile: aws.profile,
        });
        let set = domain::CapabilitySet::new(domain::ToolPolicy::default(), vec![], network, aws)?;
        Ok(Self::from(&set))
    }

    /// Whether a request to `host:port` with `method` is allowed. `port` is
    /// `None` for the scheme's default port.
    pub fn allows_request(&self, host: &str, port: Option<u16>, method: &str) -> bool {
        self.matching_rules(host, port).any(|rule| {
            rule.methods.is_empty() || rule.methods.iter().any(|m| m.eq_ignore_ascii_case(method))
        })
    }

    /// Whether a CONNECT tunnel to `host:port` is allowed. Methods inside a
    /// TLS tunnel can't be checked, so only rules without a `methods` limit
    /// allow tunnels; method-limited hosts must go through `/fetch`.
    pub fn allows_tunnel(&self, host: &str, port: u16) -> bool {
        let port = Self::tunnel_port(port);
        // STS agents call AWS directly with their scoped credentials
        let sts_endpoint = port.is_none()
            && self
                .aws
                .as_ref()
                .is_some_and(|aws| aws.mode == AwsMode::Sts)
            && host
                .trim_end_matches('.')
                .to_ascii_lowercase()
                .ends_with(".amazonaws.com");
        sts_endpoint
            || self
                .matching_rules(host, port)
                .any(|rule| rule.methods.is_empty())
    }

    /// A tunnel's port as rules name it: `None` for 443, which they leave out
    pub fn tunnel_port(port: u16) -> Option<u16> {
        (port != 443).then_some(port)
    }

    /// Whether a rule names `ip` itself for `port` (`None`: the default
    /// port), not by a name or a wildcard. The only way an internal address
    /// is reached (`address::reachable`).
    pub fn names_address(&self, ip: std::net::IpAddr, port: Option<u16>) -> bool {
        let literal = ip.to_string();
        self.matching_rules(&literal, port)
            .any(|rule| split_port(&rule.host).0 == literal)
    }

    fn matching_rules<'a>(
        &'a self,
        host: &str,
        port: Option<u16>,
    ) -> impl Iterator<Item = &'a HostRule> + 'a {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.network.allow.iter().filter(move |rule| {
            let (pattern, rule_port) = split_port(&rule.host);
            let port_ok = rule_port.and_then(|p| p.parse().ok()) == port;
            port_ok
                && (pattern == "*"
                    || pattern == host
                    || pattern
                        .strip_prefix("*.")
                        .is_some_and(|domain| host.ends_with(&format!(".{domain}"))))
        })
    }

    /// Whether the agent may call `service:action` on `resource` (an ARN).
    pub fn allows_aws(&self, service: &str, action: &str, resource: &str) -> bool {
        self.aws.as_ref().is_some_and(|aws| {
            aws.services.iter().any(|grant| {
                grant.service.eq_ignore_ascii_case(service)
                    && grant
                        .actions
                        .iter()
                        .any(|a| a == "*" || a.eq_ignore_ascii_case(action))
                    && (grant.resources.is_empty()
                        || grant.resources.iter().any(|r| glob_match(r, resource)))
            })
        })
    }

    /// An IAM session policy granting exactly the AWS grants, or `None` if
    /// there are none. Used to scope STS credentials.
    pub fn session_policy(&self) -> Option<String> {
        let aws = self.aws.as_ref()?;
        let statements: Vec<Value> = aws
            .services
            .iter()
            .filter(|grant| !grant.actions.is_empty())
            .map(|grant| {
                let mut actions: Vec<String> = grant
                    .actions
                    .iter()
                    .map(|action| iam_action(&grant.service, action))
                    .collect();
                actions.sort();
                actions.dedup();
                let resources = if grant.resources.is_empty() {
                    vec!["*".to_string()]
                } else {
                    grant.resources.clone()
                };
                json!({ "Effect": "Allow", "Action": actions, "Resource": resources })
            })
            .collect();
        (!statements.is_empty())
            .then(|| json!({ "Version": "2012-10-17", "Statement": statements }).to_string())
    }
}

/// The IAM action that authorizes an API action. Most share the name;
/// listing objects is authorized by `s3:ListBucket`.
fn iam_action(service: &str, action: &str) -> String {
    match (service, action) {
        ("s3", "ListObjects" | "ListObjectsV2") => "s3:ListBucket".to_string(),
        _ => format!("{service}:{action}"),
    }
}

/// `host[:port]` → `(host, port)`
fn split_port(host: &str) -> (&str, Option<&str>) {
    match host.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    }
}

/// `*` matches any run of characters (including `/`), like IAM ARNs.
fn glob_match(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Capabilities {
        serde_yaml::from_str::<Capabilities>(yaml)
            .unwrap()
            .validate()
            .unwrap()
    }

    const SAMPLE: &str = r#"
aws:
  mode: sts
  services:
    - service: S3
      actions: [GetObject, ListObjects]
      resources: ["arn:aws:s3:::reports", "arn:aws:s3:::reports/*"]
network:
  allow:
    - host: API.GitHub.com
      methods: [get]
    - host: "*.example.com"
    - host: "localhost:8080"
"#;

    #[test]
    fn parses_and_normalizes() {
        let caps = parse(SAMPLE);
        let aws = caps.aws.as_ref().unwrap();
        assert_eq!(aws.mode, AwsMode::Sts);
        assert_eq!(aws.services[0].service, "s3");
        assert_eq!(caps.network.allow[0].host, "api.github.com");
        assert_eq!(caps.network.allow[0].methods, ["GET"]);
    }

    #[test]
    fn defaults_to_nothing_allowed_and_proxy_mode() {
        let caps = Capabilities::default();
        assert!(!caps.allows_request("example.com", None, "GET"));
        assert!(!caps.allows_aws("s3", "GetObject", "arn:aws:s3:::b/k"));
        assert!(caps.session_policy().is_none());

        let caps = parse("aws:\n  services: []\n");
        assert_eq!(caps.aws.unwrap().mode, AwsMode::Proxy);
    }

    #[test]
    fn matches_hosts_methods_and_ports() {
        let caps = parse(SAMPLE);
        assert!(caps.allows_request("api.github.com", None, "GET"));
        assert!(caps.allows_request("API.GITHUB.COM.", None, "get"));
        assert!(!caps.allows_request("api.github.com", None, "POST"));
        assert!(!caps.allows_request("api.github.com", Some(8443), "GET"));
        assert!(!caps.allows_request("github.com", None, "GET"));

        assert!(caps.allows_request("a.b.example.com", None, "DELETE"));
        assert!(!caps.allows_request("example.com", None, "GET"));
        assert!(!caps.allows_request("evilexample.com", None, "GET"));

        assert!(caps.allows_request("localhost", Some(8080), "GET"));
        assert!(!caps.allows_request("localhost", None, "GET"));
    }

    #[test]
    fn tunnels_need_unrestricted_methods() {
        let caps = parse(SAMPLE);
        assert!(caps.allows_tunnel("www.example.com", 443));
        assert!(!caps.allows_tunnel("www.example.com", 22));
        assert!(
            !caps.allows_tunnel("api.github.com", 443),
            "method-limited hosts go through /fetch"
        );

        // SAMPLE is an STS agent: AWS endpoints are reachable directly
        assert!(caps.allows_tunnel("s3.us-east-1.amazonaws.com", 443));
        assert!(!caps.allows_tunnel("s3.us-east-1.amazonaws.com", 80));
        let proxy_only = parse("aws:\n  mode: proxy\n");
        assert!(!proxy_only.allows_tunnel("s3.amazonaws.com", 443));
    }

    #[test]
    fn rejects_bad_hosts() {
        for host in ["", "a.*.com", "ex*.com", "host:port"] {
            let caps = Capabilities {
                network: NetworkCapabilities {
                    allow: vec![HostRule {
                        host: host.to_string(),
                        methods: vec![],
                    }],
                },
                ..Default::default()
            };
            assert!(caps.validate().is_err(), "{host:?}");
        }
    }

    #[test]
    fn checks_aws_actions_and_resources() {
        let caps = parse(SAMPLE);
        assert!(caps.allows_aws("s3", "GetObject", "arn:aws:s3:::reports/2026/q3.csv"));
        assert!(caps.allows_aws("s3", "listobjects", "arn:aws:s3:::reports"));
        assert!(!caps.allows_aws("s3", "PutObject", "arn:aws:s3:::reports/x"));
        assert!(!caps.allows_aws("s3", "GetObject", "arn:aws:s3:::reports-private/x"));
        assert!(!caps.allows_aws("sqs", "GetObject", "arn:aws:s3:::reports/x"));
    }

    #[test]
    fn builds_session_policy() {
        let policy: Value = serde_json::from_str(&parse(SAMPLE).session_policy().unwrap()).unwrap();
        let statement = &policy["Statement"][0];
        assert_eq!(
            statement["Action"],
            json!(["s3:GetObject", "s3:ListBucket"])
        );
        assert_eq!(statement["Resource"][1], "arn:aws:s3:::reports/*");
    }

    #[test]
    fn globs_like_iam() {
        assert!(glob_match("arn:aws:s3:::b/*", "arn:aws:s3:::b/a/c"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*b*c", "a-b-b-c"));
        assert!(!glob_match("a*b", "a-b-c"));
        assert!(!glob_match("abc", "abcd"));
        assert!(!glob_match("ab*ba", "aba"));
    }
}
