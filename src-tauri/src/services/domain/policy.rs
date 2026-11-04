//! Policies: rules over what agents may do, in `.nandi/policies/<id>.md`.
//!
//! ```yaml
//! ---
//! applies_to: [agent:report-*, workflow:month-end, tag:finance]   # or ["*"]
//! tools: { allow: [read_file, shell], deny: [shell], ask: [write_file] }
//! files: [{ path: "D:/Finance", mode: read }, { folder: reports, mode: read }]
//! network: { allow: [{ host: "*.amazonaws.com" }] }
//! aws: { services: [{ service: s3, actions: ["Get*", "List*"] }] }
//! models: { allow: ["claude:*"] }
//! connectors: { allow: [github], run_on_host: [] }
//! pilots: { allow: [] }
//! ---
//! Why this exists, in plain words. Never grants anything.
//! ```
//!
//! A policy only ever takes away. An agent gets what its skill.md asks for,
//! narrowed by every policy that applies to it, and a deny anywhere wins
//! ([`effective`]). The one exception is `connectors.run_on_host`, which
//! lets a trusted connector run on the host instead of in a container.
//!
//! A file without `applies_to` is prose: Studio and the ask box read it,
//! nothing enforces it, as before. Other top-level keys (`name`,
//! `description`, …) are notes; the rule blocks reject unknown keys.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::agent::split_frontmatter;
use super::capability::{split_port, AwsGrant, AwsPolicy, HostRule, NetworkPolicy};
use super::{DomainError, DomainResult};

/// Every key of the policy format, for checks of text that names them
pub const POLICY_KEYS: &[&str] = &[
    "applies_to",
    "tools",
    "files",
    "network",
    "aws",
    "models",
    "connectors",
    "run_on_host",
    "pilots",
];

/// The rule blocks a policy can carry (a file with any of them must say
/// who they're for)
const RULE_KEYS: &[&str] = &[
    "tools",
    "files",
    "network",
    "aws",
    "models",
    "connectors",
    "pilots",
];

/// One `.nandi/policies/<id>.md`
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    id: String,
    applies_to: Vec<Target>,
    rules: Rules,
}

/// Who a policy is for
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// `*`
    All,
    /// `agent:<id or glob>`
    Agent(glob::Pattern),
    /// `workflow:<id or glob>`: agents while they run a step of it
    Workflow(glob::Pattern),
    /// `tag:<tag>`: agents whose skill.md lists the tag
    Tag(String),
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRules {
    /// Only these tools (on top of what skill.md allows)
    #[serde(default)]
    pub allow: Option<Vec<String>>,
    #[serde(default)]
    pub deny: Vec<String>,
    /// Always ask first, whatever skill.md says
    #[serde(default)]
    pub ask: Vec<String>,
}

/// How much of a file or folder an agent may use
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileMode {
    #[default]
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRule {
    /// A folder on the host (absolute, or under `~`) and everything in it
    #[serde(default)]
    pub path: Option<String>,
    /// A shared folder by its id (`services::connectors::folders`)
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub mode: FileMode,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AwsRules {
    #[serde(default)]
    pub services: Vec<AwsServiceRule>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AwsServiceRule {
    pub service: String,
    /// IAM-style action patterns (`Get*`); empty: any action
    #[serde(default)]
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowList {
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorRules {
    /// Only these connectors; omitted: any the agent names
    #[serde(default)]
    pub allow: Option<Vec<String>>,
    /// Trusted connectors that run on the host rather than in a container
    #[serde(default)]
    pub run_on_host: Vec<String>,
}

/// The rule blocks; `None` leaves that area alone
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    pub tools: Option<ToolRules>,
    pub files: Option<Vec<FileRule>>,
    pub network: Option<NetworkPolicy>,
    pub aws: Option<AwsRules>,
    pub models: Option<AllowList>,
    pub connectors: Option<ConnectorRules>,
    pub pilots: Option<AllowList>,
}

#[derive(Deserialize)]
struct Frontmatter {
    #[serde(default)]
    applies_to: Option<Vec<String>>,
    #[serde(default)]
    tools: Option<ToolRules>,
    #[serde(default)]
    files: Option<Vec<FileRule>>,
    #[serde(default)]
    network: Option<NetworkPolicy>,
    #[serde(default)]
    aws: Option<AwsRules>,
    #[serde(default)]
    models: Option<AllowList>,
    #[serde(default)]
    connectors: Option<ConnectorRules>,
    #[serde(default)]
    pilots: Option<AllowList>,
    /// Notes (`name`, `description`, …): kept in the file, not enforced
    #[serde(flatten)]
    _notes: BTreeMap<String, serde_yaml::Value>,
}

impl Policy {
    /// Parses `.nandi/policies/<id>.md`. A file with no frontmatter, or none
    /// of the rule keys, is prose and applies nothing.
    pub fn parse(id: &str, content: &str) -> DomainResult<Self> {
        let what = format!("policy '{id}'");
        let (yaml, _body) = split_frontmatter(content).map_err(|e| DomainError::parse(&what, e))?;
        let front: Frontmatter = match yaml.map(str::trim).filter(|y| !y.is_empty()) {
            Some(yaml) => serde_yaml::from_str(yaml).map_err(|e| DomainError::parse(&what, e))?,
            None => {
                return Ok(Self {
                    id: id.to_string(),
                    applies_to: Vec::new(),
                    rules: Rules::default(),
                })
            }
        };
        let rules = Rules {
            tools: front.tools.map(clean_tools),
            files: front
                .files
                .map(|files| validate_files(id, files))
                .transpose()?,
            network: front.network.map(NetworkPolicy::normalized).transpose()?,
            aws: front.aws.map(|aws| validate_aws(id, aws)).transpose()?,
            models: front
                .models
                .map(|m| validate_patterns(id, "models.allow", m))
                .transpose()?,
            connectors: front.connectors,
            pilots: front.pilots,
        };
        let applies_to = front
            .applies_to
            .unwrap_or_default()
            .iter()
            .map(|target| Target::parse(id, target))
            .collect::<DomainResult<Vec<_>>>()?;
        if applies_to.is_empty() && rules != Rules::default() {
            return Err(DomainError::invalid(
                format!("policy '{id}'"),
                "applies_to",
                format!(
                    "is missing, so its rules ({}) would apply to nobody. Add applies_to: [\"*\"] \
                     or name the agents",
                    RULE_KEYS
                        .iter()
                        .filter(|key| rules.has(key))
                        .copied()
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        Ok(Self {
            id: id.to_string(),
            applies_to,
            rules,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    pub fn applies_to(&self) -> &[Target] {
        &self.applies_to
    }

    /// Whether it has rules to enforce (else it's prose)
    pub fn is_enforced(&self) -> bool {
        !self.applies_to.is_empty()
    }

    /// Whether it applies to `subject`
    pub fn applies(&self, subject: &Subject) -> bool {
        self.applies_to.iter().any(|target| target.matches(subject))
    }
}

impl Rules {
    fn has(&self, key: &str) -> bool {
        match key {
            "tools" => self.tools.is_some(),
            "files" => self.files.is_some(),
            "network" => self.network.is_some(),
            "aws" => self.aws.is_some(),
            "models" => self.models.is_some(),
            "connectors" => self.connectors.is_some(),
            "pilots" => self.pilots.is_some(),
            _ => false,
        }
    }
}

impl Target {
    fn parse(policy: &str, text: &str) -> DomainResult<Self> {
        let text = text.trim();
        let invalid = |reason: String| {
            DomainError::invalid(format!("policy '{policy}'"), "applies_to", reason)
        };
        let pattern = |value: &str| {
            glob::Pattern::new(value.trim())
                .map_err(|e| invalid(format!("has a bad pattern '{text}': {e}")))
        };
        if text == "*" {
            return Ok(Self::All);
        }
        match text.split_once(':') {
            Some(("agent", id)) if !id.trim().is_empty() => Ok(Self::Agent(pattern(id)?)),
            Some(("workflow", id)) if !id.trim().is_empty() => Ok(Self::Workflow(pattern(id)?)),
            Some(("tag", tag)) if !tag.trim().is_empty() => Ok(Self::Tag(tag.trim().to_string())),
            _ => Err(invalid(format!(
                "has '{text}'; use *, agent:<id>, workflow:<id> or tag:<tag>"
            ))),
        }
    }

    fn matches(&self, subject: &Subject) -> bool {
        match self {
            Self::All => true,
            Self::Agent(pattern) => pattern.matches(subject.agent),
            Self::Workflow(pattern) => subject.workflow.is_some_and(|w| pattern.matches(w)),
            Self::Tag(tag) => subject.tags.iter().any(|t| t == tag),
        }
    }
}

fn clean_tools(mut tools: ToolRules) -> ToolRules {
    let clean = |list: &mut Vec<String>| {
        list.iter_mut().for_each(|t| *t = t.trim().to_string());
        list.retain(|t| !t.is_empty());
    };
    if let Some(allow) = tools.allow.as_mut() {
        clean(allow);
    }
    clean(&mut tools.deny);
    clean(&mut tools.ask);
    tools
}

fn validate_files(policy: &str, files: Vec<FileRule>) -> DomainResult<Vec<FileRule>> {
    let invalid =
        |reason: String| DomainError::invalid(format!("policy '{policy}'"), "files", reason);
    files
        .into_iter()
        .map(|mut rule| match (&rule.path, &rule.folder) {
            (Some(_), Some(_)) | (None, None) => Err(invalid(
                "entries need either path or folder, not both".to_string(),
            )),
            (Some(path), None) => {
                let path = normalize_path(path);
                if !is_absolute(&path) {
                    return Err(invalid(format!(
                        "has '{path}'; paths must be absolute (D:/Data, /home/me/data) or under ~"
                    )));
                }
                rule.path = Some(path);
                Ok(rule)
            }
            (None, Some(folder)) => {
                rule.folder = Some(folder.trim().to_string());
                Ok(rule)
            }
        })
        .collect()
}

fn validate_aws(policy: &str, mut aws: AwsRules) -> DomainResult<AwsRules> {
    for rule in &mut aws.services {
        rule.service = rule.service.trim().to_ascii_lowercase();
        if rule.service.is_empty() {
            return Err(DomainError::invalid(
                format!("policy '{policy}'"),
                "aws.services",
                "has an entry without 'service'",
            ));
        }
    }
    Ok(aws)
}

fn validate_patterns(
    policy: &str,
    field: &'static str,
    list: AllowList,
) -> DomainResult<AllowList> {
    for pattern in &list.allow {
        glob::Pattern::new(pattern).map_err(|e| {
            DomainError::invalid(
                format!("policy '{policy}'"),
                field,
                format!("has a bad pattern '{pattern}': {e}"),
            )
        })?;
    }
    Ok(list)
}

/// Forward slashes, no trailing slash
fn normalize_path(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    match path.trim_end_matches('/') {
        "" => "/".to_string(),
        // `C:` alone means the drive
        trimmed if trimmed.len() == 2 && trimmed.ends_with(':') => format!("{trimmed}/"),
        trimmed => trimmed.to_string(),
    }
}

fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path == "~"
        || path.starts_with("~/")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/')
}

/// `~` / `~/…` under `home`; anything else as it is
fn expand_home(path: &str, home: Option<&str>) -> String {
    match (path.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{rest}", home.replace('\\', "/").trim_end_matches('/'))
        }
        _ => path.to_string(),
    }
}

/// Whether `path` is `folder` or inside it. Windows paths (with a drive
/// letter) compare without case.
fn within(path: &str, folder: &str) -> bool {
    let (path, folder) = (normalize_path(path), normalize_path(folder));
    let windows = folder.as_bytes().get(1) == Some(&b':');
    let (path, folder) = if windows {
        (path.to_lowercase(), folder.to_lowercase())
    } else {
        (path, folder)
    };
    path == folder
        || path
            .strip_prefix(folder.trim_end_matches('/'))
            .is_some_and(|rest| rest.starts_with('/'))
}

// ---------------------------------------------------------------------------
// The set of policies
// ---------------------------------------------------------------------------

/// A policy file that doesn't parse. It still blocks: the agents it names
/// when its `applies_to` can be read, else every agent (fail closed).
#[derive(Debug, Clone, PartialEq)]
pub struct BrokenPolicy {
    pub id: String,
    pub error: String,
    /// `None`: unreadable, so it blocks everyone
    targets: Option<Vec<Target>>,
}

impl BrokenPolicy {
    pub fn new(id: &str, content: &str, error: impl std::fmt::Display) -> Self {
        Self {
            id: id.to_string(),
            error: error.to_string(),
            targets: readable_targets(id, content),
        }
    }

    pub fn blocks(&self, subject: &Subject) -> bool {
        self.targets
            .as_ref()
            .is_none_or(|targets| targets.iter().any(|t| t.matches(subject)))
    }
}

/// `applies_to` from a file whose rules don't parse, if that much does
fn readable_targets(id: &str, content: &str) -> Option<Vec<Target>> {
    let front = readable_frontmatter(content)?;
    front
        .get("applies_to")?
        .as_sequence()?
        .iter()
        .map(|v| v.as_str().and_then(|text| Target::parse(id, text).ok()))
        .collect()
}

/// The frontmatter as plain YAML, whether or not its rules parse
fn readable_frontmatter(content: &str) -> Option<serde_yaml::Value> {
    let (yaml, _) = split_frontmatter(content).ok()?;
    serde_yaml::from_str(yaml?).ok()
}

/// One policy file as a picker lists it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PolicySummary {
    pub id: String,
    /// Its `name:` note, else its id
    pub name: String,
    /// Whether it parses; a broken one still opens, to be fixed
    pub valid: bool,
}

impl PolicySummary {
    /// From `.nandi/policies/<id>.md` as it is on disk
    pub fn of(id: &str, content: &str) -> Self {
        let name = readable_frontmatter(content)
            .and_then(|front| front.get("name")?.as_str().map(str::trim).map(String::from))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| id.to_string());
        Self {
            id: id.to_string(),
            name,
            valid: Policy::parse(id, content).is_ok(),
        }
    }
}

/// Every policy in the repo, parsed or not
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PolicySet {
    pub policies: Vec<Policy>,
    pub broken: Vec<BrokenPolicy>,
}

impl PolicySet {
    /// From `(id, file content)` pairs
    pub fn parse<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut set = Self::default();
        for (id, content) in files {
            match Policy::parse(id, content) {
                Ok(policy) => set.policies.push(policy),
                Err(error) => set.broken.push(BrokenPolicy::new(id, content, error)),
            }
        }
        set
    }

    /// The broken policy that stops `subject` from running, if any
    pub fn blocking(&self, subject: &Subject) -> Option<&BrokenPolicy> {
        self.broken.iter().find(|b| b.blocks(subject))
    }

    /// What `subject` may do, or why it may not run at all
    pub fn effective(&self, request: &Request, subject: &Subject) -> Result<Effective, String> {
        match self.blocking(subject) {
            Some(broken) => Err(format!(
                "The policy '{}' has an error, so agents it may cover don't run until it's fixed: {}",
                broken.id, broken.error
            )),
            None => Ok(effective(request, subject, &self.policies)),
        }
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Who policies are being applied to
#[derive(Debug, Clone, Copy)]
pub struct Subject<'a> {
    pub agent: &'a str,
    pub tags: &'a [String],
    /// The workflow it's running a step of, if any
    pub workflow: Option<&'a str>,
}

/// A shared folder an agent asks for
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRequest {
    pub id: String,
    pub path: String,
    pub mode: FileMode,
}

/// What an agent's skill.md asks for: the input to [`effective`]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    /// `None`: every runtime tool
    pub allowed_tools: Option<Vec<String>>,
    /// With the default applied (`ToolPolicy::approval_list`)
    pub approval: Vec<String>,
    pub network: Vec<HostRule>,
    pub aws: Option<AwsPolicy>,
    /// `provider:model`, when it names one
    pub model: Option<String>,
    pub folders: Vec<FolderRequest>,
    pub connectors: Vec<String>,
    pub pilots: Vec<String>,
    /// The user's home folder, for `~` in a policy's `files` paths
    pub home: Option<String>,
}

/// What a policy changed, for the agent page's "which policy decided" view
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// `tool shell`, `folder reports`, `host api.github.com`, …
    pub item: String,
    pub outcome: Outcome,
    /// The policy's id
    pub by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Taken away
    Removed,
    /// Kept, narrower (e.g. `read only`, `api.github.com`)
    Narrowed(String),
    /// Kept, but asks first
    MustAsk,
    /// Runs on the host (`connectors.run_on_host`)
    OnHost,
}

/// What an agent may actually do: the request, narrowed by every policy
/// that applies
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effective {
    /// The policies that applied, in id order
    pub policies: Vec<String>,
    /// `None`: every runtime tool not denied
    pub allowed_tools: Option<Vec<String>>,
    /// Tool → the policy that denied it
    pub denied_tools: BTreeMap<String, String>,
    pub approval: Vec<String>,
    pub network: Vec<HostRule>,
    pub aws: Option<AwsPolicy>,
    /// Every applying `models.allow`, with its policy; a model must match
    /// all of them
    pub model_rules: Vec<(Vec<String>, String)>,
    pub folders: Vec<FolderRequest>,
    pub connectors: Vec<String>,
    pub connectors_on_host: Vec<String>,
    pub pilots: Vec<String>,
    pub changes: Vec<Change>,
}

impl Effective {
    pub fn allows_tool(&self, tool: &str) -> bool {
        !self.denied_tools.contains_key(tool)
            && self
                .allowed_tools
                .as_ref()
                .is_none_or(|allowed| allowed.iter().any(|t| t == tool))
    }

    pub fn needs_approval(&self, tool: &str) -> bool {
        self.approval.iter().any(|t| t == tool)
    }

    /// `Err(policy)` names the policy whose `models.allow` refuses `model`
    pub fn allows_model(&self, model: &str) -> Result<(), String> {
        model_allowed(&self.model_rules, model)
    }
}

/// Whether `model` (`provider:model`) matches every `models.allow` in
/// `rules`; `Err(policy)` names the first that refuses it
pub fn model_allowed(rules: &[(Vec<String>, String)], model: &str) -> Result<(), String> {
    match rules
        .iter()
        .find(|(patterns, _)| !matches_any(patterns, model))
    {
        Some((_, by)) => Err(by.clone()),
        None => Ok(()),
    }
}

/// Narrows `request` by each of `policies` that applies to `subject`.
/// Pure, and the same whatever order the policies come in.
pub fn effective(request: &Request, subject: &Subject, policies: &[Policy]) -> Effective {
    let mut applying: Vec<&Policy> = policies
        .iter()
        .filter(|p| p.is_enforced() && p.applies(subject))
        .collect();
    applying.sort_by(|a, b| a.id.cmp(&b.id));

    let mut out = Effective {
        policies: applying.iter().map(|p| p.id.clone()).collect(),
        allowed_tools: request.allowed_tools.clone(),
        approval: request.approval.clone(),
        network: request.network.clone(),
        aws: request.aws.clone(),
        folders: request.folders.clone(),
        connectors: request.connectors.clone(),
        pilots: request.pilots.clone(),
        ..Effective::default()
    };
    for policy in applying {
        apply(&mut out, policy, request);
    }
    out
}

fn apply(out: &mut Effective, policy: &Policy, request: &Request) {
    let by = policy.id.clone();
    let rules = &policy.rules;
    let mut change = |item: String, outcome: Outcome| {
        out.changes.push(Change {
            item,
            outcome,
            by: by.clone(),
        })
    };

    if let Some(tools) = &rules.tools {
        if let Some(allow) = &tools.allow {
            out.allowed_tools = Some(match out.allowed_tools.take() {
                Some(current) => {
                    let (kept, removed): (Vec<_>, Vec<_>) =
                        current.into_iter().partition(|t| allow.contains(t));
                    for tool in removed {
                        change(format!("tool {tool}"), Outcome::Removed);
                    }
                    kept
                }
                None => allow.clone(),
            });
        }
        for tool in &tools.deny {
            if !out.denied_tools.contains_key(tool) {
                out.denied_tools.insert(tool.clone(), policy.id.clone());
                change(format!("tool {tool}"), Outcome::Removed);
            }
        }
        for tool in &tools.ask {
            if !out.approval.contains(tool) {
                out.approval.push(tool.clone());
                change(format!("tool {tool}"), Outcome::MustAsk);
            }
        }
    }

    if let Some(files) = &rules.files {
        let mut kept = Vec::new();
        for folder in std::mem::take(&mut out.folders) {
            let allowed = files
                .iter()
                .filter(|rule| match (&rule.folder, &rule.path) {
                    (Some(id), _) => *id == folder.id,
                    (_, Some(path)) => {
                        within(&folder.path, &expand_home(path, request.home.as_deref()))
                    }
                    _ => false,
                })
                .map(|rule| rule.mode)
                .max();
            match allowed {
                None => change(format!("folder {}", folder.id), Outcome::Removed),
                Some(mode) if mode < folder.mode => {
                    change(
                        format!("folder {}", folder.id),
                        Outcome::Narrowed("read only".to_string()),
                    );
                    kept.push(FolderRequest { mode, ..folder });
                }
                Some(_) => kept.push(folder),
            }
        }
        out.folders = kept;
    }

    if let Some(network) = &rules.network {
        let mut kept = Vec::new();
        for rule in std::mem::take(&mut out.network) {
            match network
                .allow
                .iter()
                .find_map(|allow| intersect_host(&rule, allow))
            {
                Some(narrowed) => {
                    if narrowed != rule {
                        change(
                            format!("host {}", rule.host),
                            Outcome::Narrowed(narrowed.host.clone()),
                        );
                    }
                    kept.push(narrowed);
                }
                None => change(format!("host {}", rule.host), Outcome::Removed),
            }
        }
        out.network = kept;
    }

    if let (Some(rules), Some(aws)) = (&rules.aws, out.aws.as_mut()) {
        let mut kept = Vec::new();
        for grant in std::mem::take(&mut aws.services) {
            let item = format!("aws {}", grant.service);
            let Some(rule) = rules.services.iter().find(|r| r.service == grant.service) else {
                change(item, Outcome::Removed);
                continue;
            };
            match narrow_actions(&grant, &rule.actions) {
                Some(narrowed) => {
                    if narrowed.actions != grant.actions {
                        change(item, Outcome::Narrowed(narrowed.actions.join(", ")));
                    }
                    kept.push(narrowed);
                }
                None => change(item, Outcome::Removed),
            }
        }
        aws.services = kept;
    }

    if let Some(models) = &rules.models {
        if let Some(model) = &request.model {
            if !matches_any(&models.allow, model) {
                change(format!("model {model}"), Outcome::Removed);
            }
        }
        out.model_rules
            .push((models.allow.clone(), policy.id.clone()));
    }

    if let Some(connectors) = &rules.connectors {
        if let Some(allow) = &connectors.allow {
            let (kept, removed): (Vec<_>, Vec<_>) = std::mem::take(&mut out.connectors)
                .into_iter()
                .partition(|c| allow.contains(c));
            for connector in removed {
                change(format!("connector {connector}"), Outcome::Removed);
            }
            out.connectors = kept;
        }
    }

    if let Some(pilots) = &rules.pilots {
        let (kept, removed): (Vec<_>, Vec<_>) = std::mem::take(&mut out.pilots)
            .into_iter()
            .partition(|p| pilots.allow.contains(p));
        for pilot in removed {
            change(format!("pilot {pilot}"), Outcome::Removed);
        }
        out.pilots = kept;
    }

    // Last: only connectors still allowed may run on the host
    if let Some(connectors) = &rules.connectors {
        for connector in &connectors.run_on_host {
            if out.connectors.contains(connector) && !out.connectors_on_host.contains(connector) {
                out.connectors_on_host.push(connector.clone());
                change(format!("connector {connector}"), Outcome::OnHost);
            }
        }
    }
    out.connectors_on_host
        .retain(|c| out.connectors.contains(c));
}

fn matches_any(patterns: &[String], text: &str) -> bool {
    patterns
        .iter()
        .any(|p| glob::Pattern::new(p).is_ok_and(|p| p.matches(text)))
}

/// The part of the agent's `rule` that `allow` permits, if any: the more
/// specific host of the two, with the methods both allow
fn intersect_host(rule: &HostRule, allow: &HostRule) -> Option<HostRule> {
    let (rule_name, rule_port) = split_port(&rule.host);
    let (allow_name, allow_port) = split_port(&allow.host);
    if rule_port != allow_port {
        return None;
    }
    let host = if covers(allow_name, rule_name) {
        rule.host.clone()
    } else if covers(rule_name, allow_name) {
        allow.host.clone()
    } else {
        return None;
    };
    let methods = match (rule.methods.is_empty(), allow.methods.is_empty()) {
        (true, _) => allow.methods.clone(),
        (_, true) => rule.methods.clone(),
        _ => {
            let both: Vec<String> = rule
                .methods
                .iter()
                .filter(|m| allow.methods.contains(m))
                .cloned()
                .collect();
            if both.is_empty() {
                return None;
            }
            both
        }
    };
    Some(HostRule { host, methods })
}

/// Whether host pattern `outer` matches everything `inner` does
fn covers(outer: &str, inner: &str) -> bool {
    if outer == "*" || outer == inner {
        return true;
    }
    // `*.x.com` covers `a.x.com` and `*.a.x.com`, not `x.com`
    outer.strip_prefix('*').is_some_and(|suffix| {
        suffix.starts_with('.') && inner.ends_with(suffix) && inner.len() > suffix.len()
    })
}

/// `grant` limited to `patterns` (empty: any action); `None` when nothing's
/// left
fn narrow_actions(grant: &AwsGrant, patterns: &[String]) -> Option<AwsGrant> {
    if patterns.is_empty() {
        return Some(grant.clone());
    }
    let actions: Vec<String> = if grant.actions.is_empty() {
        // Any action, narrowed to the policy's
        patterns.to_vec()
    } else {
        grant
            .actions
            .iter()
            .filter(|action| {
                patterns.iter().any(|p| {
                    glob::Pattern::new(&p.to_ascii_lowercase())
                        .is_ok_and(|p| p.matches(&action.to_ascii_lowercase()))
                })
            })
            .cloned()
            .collect()
    };
    (!actions.is_empty()).then(|| AwsGrant {
        actions,
        ..grant.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(id: &str, front: &str) -> Policy {
        Policy::parse(id, &format!("---\n{front}\n---\nWhy.\n")).unwrap()
    }

    fn subject(agent: &str) -> Subject<'_> {
        Subject {
            agent,
            tags: &[],
            workflow: None,
        }
    }

    fn host(host: &str, methods: &[&str]) -> HostRule {
        HostRule {
            host: host.to_string(),
            methods: methods.iter().map(|m| m.to_string()).collect(),
        }
    }

    fn request() -> Request {
        Request {
            allowed_tools: None,
            approval: vec!["write_file".into()],
            network: vec![host("*.github.com", &[]), host("example.com", &["GET"])],
            aws: Some(AwsPolicy {
                services: vec![
                    AwsGrant {
                        service: "s3".into(),
                        actions: vec!["GetObject".into(), "PutObject".into()],
                        resources: vec![],
                    },
                    AwsGrant {
                        service: "ec2".into(),
                        actions: vec![],
                        resources: vec![],
                    },
                ],
                ..AwsPolicy::default()
            }),
            model: Some("gpt:gpt-5".into()),
            folders: vec![
                FolderRequest {
                    id: "reports".into(),
                    path: "D:/Finance/reports".into(),
                    mode: FileMode::Write,
                },
                FolderRequest {
                    id: "photos".into(),
                    path: "C:/Users/me/Pictures".into(),
                    mode: FileMode::Read,
                },
            ],
            connectors: vec!["github".into(), "notes".into()],
            pilots: vec!["browser".into()],
            home: None,
        }
    }

    #[test]
    fn prose_policies_apply_nothing_and_notes_are_kept() {
        let prose = Policy::parse("style", "# Be kind\n").unwrap();
        assert!(!prose.is_enforced());
        let old = policy("spending", "name: spending\nrequire_approval: false");
        assert!(!old.is_enforced(), "today's files keep working");
        let effective = effective(&request(), &subject("a"), &[prose, old]);
        assert!(effective.policies.is_empty() && effective.changes.is_empty());
    }

    #[test]
    fn rules_without_a_target_are_refused() {
        let error = Policy::parse("x", "---\ntools: { deny: [shell] }\n---\n").unwrap_err();
        assert!(error.to_string().contains("applies_to"), "{error}");
        assert!(error.to_string().contains("tools"), "{error}");
    }

    #[test]
    fn bad_rules_are_refused() {
        for front in [
            "applies_to: [robot:x]",
            "applies_to: ['*']\ntools: { block: [shell] }",
            "applies_to: ['*']\nfiles: [{ path: relative/dir }]",
            "applies_to: ['*']\nfiles: [{ path: /a, folder: b }]",
            "applies_to: ['*']\nnetwork: { allow: [{ host: 'a.*.com' }] }",
            "applies_to: ['*']\nmodels: { allow: ['[oops'] }",
            "applies_to: ['*']\naws: { services: [{ service: '' }] }",
        ] {
            assert!(
                Policy::parse("bad", &format!("---\n{front}\n---\n")).is_err(),
                "{front}"
            );
        }
    }

    #[test]
    fn targets_pick_agents_workflows_and_tags() {
        let p = policy(
            "p",
            "applies_to: [agent:report-*, workflow:month-end, tag:finance]\ntools: { deny: [shell] }",
        );
        assert!(p.applies(&subject("report-weekly")));
        assert!(!p.applies(&subject("writer")));
        let tags = ["finance".to_string()];
        assert!(p.applies(&Subject {
            agent: "writer",
            tags: &tags,
            workflow: None
        }));
        assert!(p.applies(&Subject {
            agent: "writer",
            tags: &[],
            workflow: Some("month-end")
        }));
        let all = policy("all", "applies_to: ['*']\ntools: { deny: [shell] }");
        assert!(all.applies(&subject("anyone")));
    }

    #[test]
    fn deny_wins_and_ask_is_added() {
        let deny = policy("a-deny", "applies_to: ['*']\ntools: { deny: [shell] }");
        let allow = policy(
            "b-allow",
            "applies_to: ['*']\ntools: { allow: [shell, read_file], ask: [read_file] }",
        );
        let e = effective(&request(), &subject("a"), &[allow.clone(), deny.clone()]);
        assert!(!e.allows_tool("shell"), "an explicit deny beats an allow");
        assert!(e.allows_tool("read_file"));
        assert!(!e.allows_tool("write_file"), "outside the allow list");
        assert!(e.needs_approval("read_file") && e.needs_approval("write_file"));
        assert_eq!(e.policies, ["a-deny", "b-allow"]);
        assert_eq!(e.denied_tools["shell"], "a-deny");
        // Order doesn't matter
        assert_eq!(e, effective(&request(), &subject("a"), &[deny, allow]));
    }

    #[test]
    fn a_policy_never_widens() {
        let mut narrow = request();
        narrow.allowed_tools = Some(vec!["read_file".into()]);
        narrow.network = vec![host("api.github.com", &["GET"])];
        let wide = policy(
            "wide",
            "applies_to: ['*']\ntools: { allow: [shell, read_file, write_file] }\n\
             network: { allow: [{ host: '*' }] }",
        );
        let e = effective(&narrow, &subject("a"), &[wide]);
        assert_eq!(
            e.allowed_tools.as_deref(),
            Some(&["read_file".to_string()][..])
        );
        assert_eq!(e.network, narrow.network);
        assert!(e.changes.is_empty());
    }

    #[test]
    fn folders_are_dropped_or_made_read_only() {
        let p = policy(
            "files",
            "applies_to: ['*']\nfiles: [{ path: 'd:\\finance', mode: read }]",
        );
        let e = effective(&request(), &subject("a"), &[p]);
        assert_eq!(e.folders.len(), 1, "photos isn't under D:/Finance");
        assert_eq!(e.folders[0].id, "reports");
        assert_eq!(e.folders[0].mode, FileMode::Read);
        assert!(e
            .changes
            .iter()
            .any(|c| c.item == "folder photos" && c.outcome == Outcome::Removed));

        let by_id = policy("ids", "applies_to: ['*']\nfiles: [{ folder: photos }]");
        let e = effective(&request(), &subject("a"), &[by_id]);
        assert_eq!(
            e.folders.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            ["photos"]
        );
    }

    #[test]
    fn hosts_narrow_to_the_more_specific_and_shared_methods() {
        let p = policy(
            "net",
            "applies_to: ['*']\nnetwork: { allow: [{ host: api.github.com, methods: [GET, POST] }, { host: '*.example.com' }] }",
        );
        let e = effective(&request(), &subject("a"), &[p]);
        assert_eq!(e.network, [host("api.github.com", &["GET", "POST"])]);
        assert!(e
            .changes
            .iter()
            .any(|c| c.item == "host example.com" && c.outcome == Outcome::Removed));
        assert!(covers("*.x.com", "*.a.x.com") && !covers("*.x.com", "x.com"));
        assert!(intersect_host(&host("a.com:8443", &[]), &host("a.com", &[])).is_none());
    }

    #[test]
    fn aws_grants_keep_only_allowed_services_and_actions() {
        let p = policy(
            "aws",
            "applies_to: ['*']\naws: { services: [{ service: S3, actions: ['get*'] }] }",
        );
        let e = effective(&request(), &subject("a"), &[p]);
        let services = &e.aws.unwrap().services;
        assert_eq!(services.len(), 1, "ec2 isn't allowed");
        assert_eq!(services[0].actions, ["GetObject"]);
    }

    #[test]
    fn models_must_match_every_applying_policy() {
        let anthropic = policy(
            "only-claude",
            "applies_to: ['*']\nmodels: { allow: ['claude:*'] }",
        );
        let e = effective(&request(), &subject("a"), &[anthropic]);
        assert_eq!(e.allows_model("gpt:gpt-5"), Err("only-claude".to_string()));
        assert_eq!(e.allows_model("claude:claude-sonnet-5"), Ok(()));
        assert!(e.changes.iter().any(|c| c.item == "model gpt:gpt-5"));
        assert_eq!(
            effective(&request(), &subject("a"), &[]).allows_model("x:y"),
            Ok(())
        );
    }

    #[test]
    fn connectors_and_pilots_narrow_and_trusted_ones_run_on_the_host() {
        let p = policy(
            "conn",
            "applies_to: ['*']\nconnectors: { allow: [github, notes], run_on_host: [notes, gone] }\npilots: { allow: [] }",
        );
        let only_github = policy(
            "gh",
            "applies_to: [agent:b]\nconnectors: { allow: [github] }",
        );
        let e = effective(&request(), &subject("a"), &[p.clone(), only_github.clone()]);
        assert_eq!(e.connectors, ["github", "notes"]);
        assert_eq!(e.connectors_on_host, ["notes"], "only ones still allowed");
        assert!(e.pilots.is_empty());
        // Another applying policy that drops `notes` also takes back the host
        let e = effective(&request(), &subject("b"), &[p, only_github]);
        assert_eq!(e.connectors, ["github"]);
        assert!(e.connectors_on_host.is_empty());
    }

    #[test]
    fn a_summary_names_the_policy_and_says_whether_it_parses() {
        let named = PolicySummary::of(
            "no-shell",
            "---
name: No shell
applies_to: ['*']
tools: { deny: [shell] }
---
",
        );
        assert_eq!(
            named,
            PolicySummary {
                id: "no-shell".into(),
                name: "No shell".into(),
                valid: true,
            }
        );
        let prose = PolicySummary::of(
            "tone",
            "Be kind.
",
        );
        assert_eq!((prose.name.as_str(), prose.valid), ("tone", true));
        let broken = PolicySummary::of(
            "oops",
            "---
name: Oops
tools: { nope: 1 }
---
",
        );
        assert_eq!((broken.name.as_str(), broken.valid), ("Oops", false));
        let unreadable = PolicySummary::of(
            "mess",
            "---
name: [
---
",
        );
        assert_eq!(
            (unreadable.name.as_str(), unreadable.valid),
            ("mess", false)
        );
    }

    #[test]
    fn a_broken_policy_blocks_whom_it_names_or_everyone() {
        let set = PolicySet::parse([
            (
                "ok",
                "---
applies_to: ['*']
tools: { deny: [shell] }
---
",
            ),
            (
                "named",
                "---
applies_to: [agent:report-*]
tools: { block: [x] }
---
",
            ),
        ]);
        assert_eq!(set.policies.len(), 1);
        assert!(set.blocking(&subject("report-weekly")).is_some());
        assert!(set.blocking(&subject("writer")).is_none());
        let e = set.effective(&request(), &subject("writer")).unwrap();
        assert!(!e.allows_tool("shell"));
        let error = set
            .effective(&request(), &subject("report-weekly"))
            .unwrap_err();
        assert!(error.contains("'named'"), "{error}");

        let unreadable = PolicySet::parse([(
            "mess",
            "---
applies_to: [
---
",
        )]);
        assert!(
            unreadable.blocking(&subject("anyone")).is_some(),
            "fail closed"
        );
    }

    #[test]
    fn home_in_policy_paths_is_the_user_s() {
        let p = policy("home", "applies_to: ['*']\nfiles: [{ path: '~/Finance' }]");
        let mut r = request();
        r.folders[0].path = "C:/Users/me/Finance/reports".into();
        r.home = Some(r"C:\Users\me".into());
        let e = effective(&r, &subject("a"), &[p]);
        assert_eq!(
            e.folders.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            ["reports"]
        );
        assert_eq!(expand_home("~/x", None), "~/x");
    }

    #[test]
    fn paths_compare_by_folder_not_by_prefix() {
        assert!(within("D:/Finance/reports", "d:/finance"));
        assert!(within("D:\\Finance", "D:/Finance/"));
        assert!(!within("D:/Finances", "D:/Finance"));
        assert!(!within("/home/Me", "/home/me"), "case matters off Windows");
        assert!(within("/anything", "/"));
    }
}
