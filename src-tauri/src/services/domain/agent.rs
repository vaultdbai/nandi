//! The agent entity: `.nandi/agents/<id>/skill.md` (frontmatter + system
//! prompt) and the optional `instructions.md` (persistent user
//! instructions appended to it).
//!
//! ```markdown
//! ---
//! name: Research Agent
//! description: Finds papers
//! model: deepseek:deepseek-flash   # omitted: settings.yaml default_model
//! image: acme/tools:1.4               # omitted: the default runtime image
//! memory_mb: 2048
//! cpu: 1.5
//! shell_timeout_s: 300           # omitted: 30
//! allowed-tools: [shell, read_file]
//! requires_approval: [shell]
//! serves: [{name: find-papers, description: Finds papers on a topic}]
//! provides: [code-review.python]   # shorthand: bare serves entries
//! accepts_delegation: true
//! ---
//!
//! You are a research agent…
//! ```
//!
//! Unknown frontmatter keys are kept and written back, so saving an agent
//! never drops what a newer app version (or the user) put there.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::capability::{
    AwsPolicy, CapabilitySet, NetworkPolicy, PilotOptions, ServesCapability, ToolPolicy,
};
use super::{
    check_len, has_conflict_markers, AgentId, DomainError, DomainResult, PilotRef, WorkflowId,
    CONFLICT_MARKERS,
};

pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_DESCRIPTION_CHARS: usize = 2_000;
pub const MAX_INSTRUCTIONS_CHARS: usize = 200_000;

/// skill.md frontmatter a marketplace publisher may set to describe where
/// an agent came from. A fork drops them: it's the user's own agent.
const SOURCE_METADATA_KEYS: [&str; 8] = [
    "source",
    "publisher",
    "repo",
    "version",
    "signature",
    "marketplace",
    "min_nandi_version",
    "tags",
];

// ---------------------------------------------------------------------------
// Value objects
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    DeepSeek,
    Anthropic,
    OpenAi,
    Ollama,
}

impl Provider {
    pub const ALL: [Provider; 4] = [Self::DeepSeek, Self::Anthropic, Self::OpenAi, Self::Ollama];

    /// The prefix in `<prefix>:<model>` strings and `provider:` keys
    pub fn prefix(self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek",
            Self::Anthropic => "claude",
            Self::OpenAi => "gpt",
            Self::Ollama => "ollama",
        }
    }

    pub fn from_prefix(prefix: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.prefix() == prefix)
    }
}

/// An LLM: `deepseek:deepseek-flash` in skill.md, or
/// `{provider: deepseek, name: deepseek-flash}` in settings.yaml.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelRef {
    provider: Provider,
    name: String,
}

impl ModelRef {
    pub fn new(provider: Provider, name: impl Into<String>) -> DomainResult<Self> {
        let name = name.into().trim().to_string();
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Err(DomainError::invalid(
                "model",
                "name",
                format!("'{name}' must be a non-empty name without spaces"),
            ));
        }
        Ok(Self { provider, name })
    }

    /// Parses `<provider>:<model>`, e.g. `claude:claude-sonnet-5`.
    pub fn parse(value: &str) -> DomainResult<Self> {
        let (prefix, name) = value.trim().split_once(':').ok_or_else(|| {
            DomainError::invalid(
                "model",
                "value",
                format!("'{value}' should be '<provider>:<model>', e.g. 'claude:claude-sonnet-5'"),
            )
        })?;
        Self::new(Self::provider(prefix)?, name)
    }

    fn provider(prefix: &str) -> DomainResult<Provider> {
        Provider::from_prefix(prefix.trim()).ok_or_else(|| {
            DomainError::invalid(
                "model",
                "provider",
                format!("'{prefix}' is unknown: use 'deepseek', 'claude', 'gpt', or 'ollama'"),
            )
        })
    }

    pub fn provider_kind(&self) -> Provider {
        self.provider
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.provider.prefix(), self.name)
    }
}

/// Serialized as `<provider>:<model>`; also reads the mapping form.
impl Serialize for ModelRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ModelRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Text(String),
            Mapping { provider: String, name: String },
        }
        let parsed = match Form::deserialize(deserializer)? {
            Form::Text(text) => Self::parse(&text),
            Form::Mapping { provider, name } => {
                Self::provider(&provider).and_then(|p| Self::new(p, name))
            }
        };
        parsed.map_err(serde::de::Error::custom)
    }
}

/// The mapping form, for settings.yaml.
pub mod model_ref_mapping {
    use super::ModelRef;
    use serde::ser::SerializeMap;

    pub fn serialize<S: serde::Serializer>(model: &ModelRef, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("provider", model.provider.prefix())?;
        map.serialize_entry("name", &model.name)?;
        map.end()
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ModelRef, D::Error> {
        serde::Deserialize::deserialize(d)
    }
}

/// Where the agent's tools run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContainerSpec {
    /// `None`: the default runtime image
    image: Option<String>,
    memory_mb: Option<u32>,
    cpu: Option<f32>,
    /// How long one `shell` call may run; `None` is the runtime's 30s
    shell_timeout_s: Option<u32>,
}

/// The longest `shell_timeout_s` skill.md may ask for
pub const MAX_SHELL_TIMEOUT_S: u32 = 3_600;

impl ContainerSpec {
    pub fn new(
        image: Option<String>,
        memory_mb: Option<u32>,
        cpu: Option<f32>,
        shell_timeout_s: Option<u32>,
    ) -> DomainResult<Self> {
        let image = image
            .map(|i| i.trim().to_string())
            .filter(|i| !i.is_empty());
        if image
            .as_deref()
            .is_some_and(|i| i.chars().any(char::is_whitespace))
        {
            return Err(DomainError::invalid(
                "container",
                "image",
                "must not contain spaces",
            ));
        }
        if memory_mb.is_some_and(|m| !(16..=1_048_576).contains(&m)) {
            return Err(DomainError::invalid(
                "container",
                "memory_mb",
                "must be between 16 and 1048576",
            ));
        }
        if cpu.is_some_and(|c| !c.is_finite() || c <= 0.0 || c > 256.0) {
            return Err(DomainError::invalid(
                "container",
                "cpu",
                "must be more than 0 and at most 256",
            ));
        }
        if shell_timeout_s.is_some_and(|s| !(1..=MAX_SHELL_TIMEOUT_S).contains(&s)) {
            return Err(DomainError::invalid(
                "container",
                "shell_timeout_s",
                format!("must be between 1 and {MAX_SHELL_TIMEOUT_S}"),
            ));
        }
        Ok(Self {
            image,
            memory_mb,
            cpu,
            shell_timeout_s,
        })
    }

    pub fn image(&self) -> Option<&str> {
        self.image.as_deref()
    }

    pub fn memory_mb(&self) -> Option<u32> {
        self.memory_mb
    }

    pub fn cpu(&self) -> Option<f32> {
        self.cpu
    }

    pub fn shell_timeout_s(&self) -> Option<u32> {
        self.shell_timeout_s
    }
}

/// What runs the agent's turns: Nandi's own loop, or a CLI harness over ACP
/// (`runtime:` in skill.md; absent is `native`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum AgentRuntimeKind {
    #[default]
    Native,
    ClaudeCode,
    Codex,
    GeminiCli,
    /// A registered ACP executable, `custom:<id>`
    Custom(String),
}

impl AgentRuntimeKind {
    pub fn parse(value: &str) -> DomainResult<Self> {
        let value = value.trim();
        Ok(match value {
            "" | "native" => Self::Native,
            "claude-code" | "claude_code" => Self::ClaudeCode,
            "codex" => Self::Codex,
            "gemini-cli" | "gemini_cli" | "gemini" => Self::GeminiCli,
            other => match other.strip_prefix("custom:").map(str::trim) {
                Some(id)
                    if !id.is_empty()
                        && id
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) =>
                {
                    Self::Custom(id.to_string())
                }
                _ => {
                    return Err(DomainError::invalid(
                        "agent",
                        "runtime",
                        format!(
                            "'{other}' is unknown: use native, claude-code, codex, gemini-cli, or custom:<id>"
                        ),
                    ))
                }
            },
        })
    }

    /// As written in skill.md
    pub fn as_str(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Self::Native => "native".into(),
            Self::ClaudeCode => "claude-code".into(),
            Self::Codex => "codex".into(),
            Self::GeminiCli => "gemini-cli".into(),
            Self::Custom(id) => format!("custom:{id}").into(),
        }
    }

    pub fn is_native(&self) -> bool {
        *self == Self::Native
    }
}

/// `runtime_config:` in skill.md: settings for a harness runtime. Unknown
/// keys are kept, like the frontmatter's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeConfig {
    /// The harness's model ID (e.g. `sonnet`, a Bedrock model or inference
    /// profile ID); `None`: the harness default
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `bedrock` routes the session to Bedrock with the user's AWS identity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// `litellm`: through the LiteLLM gateway (third-party Bedrock models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "aws-profile")]
    pub aws_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "aws-region")]
    pub aws_region: Option<String>,
    /// `native`: run in Nandi's own loop when the harness can't start.
    /// Off by default: falling back would silently change the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

impl RuntimeConfig {
    fn check(&self) -> DomainResult<()> {
        let id_like = |field: &'static str, value: &Option<String>| match value {
            Some(v) if v.trim().is_empty() || v.chars().any(char::is_whitespace) => {
                Err(DomainError::invalid(
                    "runtime_config",
                    field,
                    "must be a non-empty value without spaces",
                ))
            }
            _ => Ok(()),
        };
        id_like("model", &self.model)?;
        id_like("aws_profile", &self.aws_profile)?;
        id_like("aws_region", &self.aws_region)?;
        if let Some(gateway) = &self.gateway {
            if gateway != "litellm" {
                return Err(DomainError::invalid(
                    "runtime_config",
                    "gateway",
                    format!("'{gateway}' is unknown: use litellm"),
                ));
            }
        }
        if let Some(fallback) = &self.fallback {
            if fallback != "native" {
                return Err(DomainError::invalid(
                    "runtime_config",
                    "fallback",
                    format!("'{fallback}' is unknown: use native"),
                ));
            }
        }
        Ok(())
    }

    pub fn falls_back_to_native(&self) -> bool {
        self.fallback.as_deref() == Some("native")
    }
}

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

/// Everything needed to create an agent. `Agent::new` validates it.
#[derive(Debug, Clone)]
pub struct AgentDraft {
    pub id: AgentId,
    pub name: String,
    pub description: String,
    pub skill_body: String,
    pub instructions: String,
    pub model: Option<ModelRef>,
    pub container: ContainerSpec,
    pub capabilities: CapabilitySet,
}

impl AgentDraft {
    /// A draft with defaults for everything but the ID and name.
    pub fn named(id: AgentId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: String::new(),
            skill_body: String::new(),
            instructions: String::new(),
            model: None,
            container: ContainerSpec::default(),
            capabilities: CapabilitySet::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    id: AgentId,
    name: String,
    description: String,
    /// skill.md without frontmatter: the agent's system prompt
    skill_body: String,
    /// instructions.md: persistent instructions from the user
    instructions: String,
    /// `None`: the settings default
    model: Option<ModelRef>,
    container: ContainerSpec,
    capabilities: CapabilitySet,
    /// `runtime:`; `Native` when absent
    runtime: AgentRuntimeKind,
    /// `runtime_config:` (boxed: most agents have none)
    runtime_config: Option<Box<RuntimeConfig>>,
    /// Agent-scoped workflows (`.nandi/agents/<id>/workflows/`), set by the
    /// aggregate
    workflows: Vec<WorkflowId>,
    /// Frontmatter keys this version doesn't know, written back unchanged
    extra: BTreeMap<String, serde_yaml::Value>,
    created_at: SystemTime,
    updated_at: SystemTime,
}

impl Agent {
    pub fn new(draft: AgentDraft) -> DomainResult<Self> {
        let now = SystemTime::now();
        let agent = Self {
            id: draft.id,
            name: draft.name.trim().to_string(),
            description: draft.description.trim().to_string(),
            skill_body: draft.skill_body.trim().to_string(),
            instructions: draft.instructions.trim().to_string(),
            model: draft.model,
            container: draft.container,
            capabilities: draft.capabilities,
            runtime: AgentRuntimeKind::Native,
            runtime_config: None,
            workflows: Vec::new(),
            extra: BTreeMap::new(),
            created_at: now,
            updated_at: now,
        };
        agent.check()?;
        Ok(agent)
    }

    fn label(&self) -> String {
        format!("Agent '{}'", self.id)
    }

    fn check(&self) -> DomainResult<()> {
        let label = self.label();
        if self.name.is_empty() {
            return Err(DomainError::invalid(label, "name", "is required"));
        }
        check_len(&label, "name", &self.name, MAX_NAME_CHARS)?;
        check_len(
            &label,
            "description",
            &self.description,
            MAX_DESCRIPTION_CHARS,
        )?;
        check_len(
            &label,
            "skill body",
            &self.skill_body,
            MAX_INSTRUCTIONS_CHARS,
        )?;
        check_len(
            &label,
            "instructions",
            &self.instructions,
            MAX_INSTRUCTIONS_CHARS,
        )?;
        if let Some(config) = &self.runtime_config {
            config.check()?;
        }
        Ok(())
    }

    /// Parses skill.md. The name defaults to the ID. Timestamps are the
    /// epoch until the caller sets them (`with_timestamps`).
    pub fn from_skill_md(id: AgentId, content: &str) -> DomainResult<Self> {
        let what = || format!("skill.md for agent '{id}'");
        if has_conflict_markers(content) {
            return Err(DomainError::parse(what(), CONFLICT_MARKERS));
        }
        let (yaml, body) = split_frontmatter(content).map_err(|e| DomainError::parse(what(), e))?;
        let fm: Frontmatter = match yaml.filter(|y| !y.trim().is_empty()) {
            Some(yaml) => serde_yaml::from_str(yaml).map_err(|e| DomainError::parse(what(), e))?,
            None => Frontmatter::default(),
        };
        let mut pilot_options = BTreeMap::new();
        let pilots = fm
            .pilots
            .into_iter()
            .map(|item| match item {
                PilotItem::Id(id) => PilotRef::parse(id),
                PilotItem::Entry(entry) => {
                    let pilot = PilotRef::parse(entry.id)?;
                    let options = PilotOptions {
                        allow: entry.allow,
                        values: entry.values,
                        secrets: entry.secrets,
                    };
                    pilot_options.insert(pilot.as_str().to_string(), options);
                    Ok(pilot)
                }
            })
            .collect::<DomainResult<Vec<_>>>()?;
        let capabilities = CapabilitySet::new(
            ToolPolicy::new(fm.allowed_tools, fm.requires_approval),
            pilots,
            fm.network.unwrap_or_default(),
            fm.aws,
        )?
        .with_delegation(
            fm.serves,
            fm.accepts_delegation.unwrap_or(false),
            fm.max_delegation_concurrent,
        )?
        .with_provides(fm.provides)?
        .with_pilot_options(pilot_options)?;
        let model = fm
            .model
            .filter(|m| !m.trim().is_empty())
            .map(|m| ModelRef::parse(&m))
            .transpose()?;
        let agent = Self {
            name: fm
                .name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.to_string()),
            description: fm.description.unwrap_or_default().trim().to_string(),
            skill_body: body.trim().to_string(),
            instructions: String::new(),
            model,
            container: ContainerSpec::new(fm.image, fm.memory_mb, fm.cpu, fm.shell_timeout_s)?,
            capabilities,
            runtime: fm
                .runtime
                .as_deref()
                .map(AgentRuntimeKind::parse)
                .transpose()?
                .unwrap_or_default(),
            runtime_config: fm.runtime_config.map(Box::new),
            workflows: Vec::new(),
            extra: fm.extra,
            created_at: UNIX_EPOCH,
            updated_at: UNIX_EPOCH,
            id,
        };
        agent.check()?;
        Ok(agent)
    }

    /// skill.md plus the optional instructions.md.
    pub fn from_files(
        id: AgentId,
        skill_md: &str,
        instructions_md: Option<&str>,
    ) -> DomainResult<Self> {
        if instructions_md.is_some_and(has_conflict_markers) {
            return Err(DomainError::parse(
                format!("instructions.md for agent '{id}'"),
                CONFLICT_MARKERS,
            ));
        }
        let mut agent = Self::from_skill_md(id, skill_md)?;
        // Loading isn't a change: set the field without touching timestamps
        agent.instructions = instructions_md.unwrap_or_default().trim().to_string();
        agent.check()?;
        Ok(agent)
    }

    pub fn with_timestamps(mut self, created_at: SystemTime, updated_at: SystemTime) -> Self {
        self.created_at = created_at;
        self.updated_at = updated_at;
        self
    }

    /// A copy owned by the user: a new ID and name, with the marketplace's
    /// source metadata dropped from the frontmatter. Everything else (model,
    /// capabilities, prompt, unknown keys) carries over.
    pub fn forked(&self, id: AgentId, name: &str) -> DomainResult<Self> {
        let mut fork = self.clone();
        fork.id = id;
        fork.name = name.trim().to_string();
        fork.workflows = Vec::new();
        fork.extra
            .retain(|key, _| !SOURCE_METADATA_KEYS.contains(&key.as_str()));
        let now = SystemTime::now();
        (fork.created_at, fork.updated_at) = (now, now);
        fork.check()?;
        Ok(fork)
    }

    pub fn to_skill_md(&self) -> String {
        let tools = self.capabilities.tools();
        let pilots: Vec<PilotItemOut> = self
            .capabilities
            .pilots()
            .iter()
            .map(
                |pilot| match self.capabilities.pilot_options(pilot.as_str()) {
                    Some(options) => PilotItemOut::Entry {
                        id: pilot.as_str(),
                        options,
                    },
                    None => PilotItemOut::Id(pilot.as_str()),
                },
            )
            .collect();
        let fm = FrontmatterOut {
            name: &self.name,
            description: (!self.description.is_empty()).then_some(self.description.as_str()),
            model: self.model.as_ref().map(ToString::to_string),
            image: self.container.image(),
            memory_mb: self.container.memory_mb(),
            cpu: self.container.cpu(),
            shell_timeout_s: self.container.shell_timeout_s(),
            allowed_tools: tools.allowed(),
            requires_approval: tools.declared_approvals(),
            pilots,
            aws: self.capabilities.aws(),
            network: Some(self.capabilities.network()).filter(|n| !n.allow.is_empty()),
            serves: self.capabilities.declared_serves(),
            provides: self.capabilities.provides(),
            accepts_delegation: self.capabilities.accepts_delegation(),
            max_delegation_concurrent: self.capabilities.max_delegation_concurrent(),
            // Absent means native, so a native agent's file doesn't say it
            runtime: (!self.runtime.is_native()).then(|| self.runtime.as_str().into_owned()),
            runtime_config: self.runtime_config.as_deref(),
            extra: &self.extra,
        };
        // Serializing plain data can't fail
        let yaml = serde_yaml::to_string(&fm).unwrap_or_default();
        if self.skill_body.is_empty() {
            format!("---\n{yaml}---\n")
        } else {
            format!("---\n{yaml}---\n\n{}\n", self.skill_body)
        }
    }

    /// instructions.md, or `""` if there are none (the file is then removed).
    pub fn to_instructions_md(&self) -> String {
        if self.instructions.is_empty() {
            String::new()
        } else {
            format!("{}\n", self.instructions)
        }
    }

    // -- Behavior -----------------------------------------------------------

    pub fn update_instructions(&mut self, text: &str) -> DomainResult<()> {
        let text = text.trim();
        check_len(&self.label(), "instructions", text, MAX_INSTRUCTIONS_CHARS)?;
        if self.instructions != text {
            self.instructions = text.to_string();
            self.updated_at = SystemTime::now();
        }
        Ok(())
    }

    pub fn update_profile(&mut self, name: &str, description: &str) -> DomainResult<()> {
        let previous = (self.name.clone(), self.description.clone());
        self.name = name.trim().to_string();
        self.description = description.trim().to_string();
        if let Err(e) = self.check() {
            (self.name, self.description) = previous;
            return Err(e);
        }
        self.updated_at = SystemTime::now();
        Ok(())
    }

    /// Lets the agent reach `hosts` (`network.allow`), as when a person
    /// always allows a site its browser asked for. `false`: it could already
    pub fn allow_sites(&mut self, hosts: &[String]) -> DomainResult<bool> {
        let added = self.capabilities.allow_hosts(hosts)?;
        if added {
            self.updated_at = SystemTime::now();
        }
        Ok(added)
    }

    pub fn set_model(&mut self, model: Option<ModelRef>) {
        self.model = model;
        self.updated_at = SystemTime::now();
    }

    pub fn set_runtime(
        &mut self,
        runtime: AgentRuntimeKind,
        config: Option<RuntimeConfig>,
    ) -> DomainResult<()> {
        if let Some(config) = &config {
            config.check()?;
        }
        self.runtime = runtime;
        self.runtime_config = config.map(Box::new);
        self.updated_at = SystemTime::now();
        Ok(())
    }

    pub fn runtime(&self) -> &AgentRuntimeKind {
        &self.runtime
    }

    pub fn runtime_config(&self) -> Option<&RuntimeConfig> {
        self.runtime_config.as_deref()
    }

    pub fn requires_approval_for(&self, tool: &str) -> bool {
        self.capabilities.tools().requires_approval_for(tool)
    }

    pub fn allows_tool(&self, tool: &str) -> bool {
        self.capabilities.tools().allows(tool)
    }

    /// The agent's part of the system prompt: skill body, then instructions.
    pub fn system_prompt(&self) -> String {
        match (self.skill_body.is_empty(), self.instructions.is_empty()) {
            (_, true) => self.skill_body.clone(),
            (true, false) => self.instructions.clone(),
            (false, false) => format!("{}\n\n{}", self.skill_body, self.instructions),
        }
    }

    /// The model to run with: the agent's own, else `default`.
    pub fn effective_model(&self, default: &ModelRef) -> ModelRef {
        self.model.clone().unwrap_or_else(|| default.clone())
    }

    pub(super) fn set_workflows(&mut self, workflows: Vec<WorkflowId>) {
        self.workflows = workflows;
    }

    // -- Accessors ----------------------------------------------------------

    /// skill.md keys Nandi doesn't model, preserved on save (e.g. the wave
    /// final turn policy: `middlewares`, `max_turns`, `memory_disabled`)
    pub fn extra(&self) -> &BTreeMap<String, serde_yaml::Value> {
        &self.extra
    }

    /// `tags:`, the labels policies aim at with `tag:<label>`
    pub fn tags(&self) -> Vec<String> {
        match self.extra.get("tags") {
            Some(serde_yaml::Value::Sequence(items)) => items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|tag| tag.trim().to_string())
                .filter(|tag| !tag.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Sets (or with `None` removes) one of those keys
    pub fn set_extra(&mut self, key: &str, value: Option<serde_yaml::Value>) {
        match value {
            Some(value) => {
                self.extra.insert(key.to_string(), value);
            }
            None => {
                self.extra.remove(key);
            }
        }
        self.updated_at = SystemTime::now();
    }

    pub fn id(&self) -> &AgentId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn skill_body(&self) -> &str {
        &self.skill_body
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn model(&self) -> Option<&ModelRef> {
        self.model.as_ref()
    }

    pub fn container(&self) -> &ContainerSpec {
        &self.container
    }

    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    pub fn workflows(&self) -> &[WorkflowId] {
        &self.workflows
    }

    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub fn updated_at(&self) -> SystemTime {
        self.updated_at
    }
}

// ---------------------------------------------------------------------------
// skill.md format
// ---------------------------------------------------------------------------

/// A `pilots:` item: its ID, or a mapping with its options
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PilotItem {
    Id(String),
    Entry(PilotEntry),
}

/// A `pilots:` item written as a mapping (unknown keys are an error, so a
/// typo in `allow` can't quietly ask less)
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PilotEntry {
    id: String,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    values: BTreeMap<String, String>,
    #[serde(default)]
    secrets: BTreeMap<String, super::settings::SecretRef>,
}

/// A `pilots:` item as written back: the bare ID unless it has options
#[derive(Serialize)]
#[serde(untagged)]
enum PilotItemOut<'a> {
    Id(&'a str),
    Entry {
        id: &'a str,
        #[serde(flatten)]
        options: &'a PilotOptions,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    model: Option<String>,
    image: Option<String>,
    #[serde(alias = "memory-mb")]
    memory_mb: Option<u32>,
    cpu: Option<f32>,
    shell_timeout_s: Option<u32>,
    #[serde(rename = "allowed-tools", alias = "allowed_tools")]
    allowed_tools: Option<Vec<String>>,
    #[serde(alias = "requires-approval")]
    requires_approval: Option<Vec<String>>,
    pilots: Vec<PilotItem>,
    aws: Option<AwsPolicy>,
    network: Option<NetworkPolicy>,
    serves: Vec<ServesCapability>,
    /// Shorthand: capability IDs served without a description or schema
    provides: Vec<String>,
    #[serde(alias = "accepts-delegation")]
    accepts_delegation: Option<bool>,
    #[serde(alias = "max-delegation-concurrent")]
    max_delegation_concurrent: Option<u32>,
    runtime: Option<String>,
    #[serde(alias = "runtime-config")]
    runtime_config: Option<RuntimeConfig>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml::Value>,
}

/// Every skill.md frontmatter key [`Frontmatter`] reads, with what it means:
/// the schema Studio's prompt shows. `every_frontmatter_key_is_documented`
/// fails when the struct gains a key this list lacks.
pub const SKILL_FRONTMATTER_KEYS: &[(&str, &str)] = &[
    ("name", "display name (defaults to the agent ID)"),
    ("description", "one line on what the agent does"),
    (
        "model",
        "`<provider>:<model>`, e.g. `claude:claude-sonnet-5`; omit to use the default model",
    ),
    ("image", "container image (default: the Nandi runtime)"),
    ("memory_mb", "container memory in MB"),
    ("cpu", "container CPUs"),
    (
        "shell_timeout_s",
        "seconds one `shell` call may run (default 30, at most 3600)",
    ),
    (
        "allowed-tools",
        "list of tool IDs the agent may use (omit: every runtime tool)",
    ),
    (
        "requires_approval",
        "list of tool IDs that wait for the user's approval",
    ),
    (
        "pilots",
        "list of host pilots (OS control, the browser) the agent may use: an ID, or `{id, allow, values, secrets}`",
    ),
    ("aws", "AWS access: `services:` list and optional `region:`"),
    ("network", "internet access: `allow:` list of hosts"),
    (
        "serves",
        "capabilities it offers others: list of `{name, description}`",
    ),
    ("provides", "shorthand: list of capability IDs it serves"),
    (
        "accepts_delegation",
        "true to accept requests addressed to this agent directly",
    ),
    (
        "max_delegation_concurrent",
        "how many delegated tasks it runs at once",
    ),
    (
        "runtime",
        "where the model runs: `native` (Nandi itself, the default), `claude-code`, `codex`, `gemini-cli`",
    ),
    (
        "runtime_config",
        "settings for a CLI runtime, e.g. `model:` for that CLI",
    ),
    (
        "connectors",
        "list of installed connector IDs whose tools the agent gets, e.g. `[filesystem]`",
    ),
    (
        "tags",
        "labels policies can aim at with `tag:<label>`, e.g. `[finance]`",
    ),
];

/// Documented keys the connector service reads from `extra` (kept as
/// written), not the parser
pub const EXTRA_FRONTMATTER_KEYS: &[&str] = &["connectors", "tags"];

#[derive(Serialize)]
struct FrontmatterOut<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell_timeout_s: Option<u32>,
    #[serde(rename = "allowed-tools", skip_serializing_if = "Option::is_none")]
    allowed_tools: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requires_approval: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pilots: Vec<PilotItemOut<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aws: Option<&'a AwsPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<&'a NetworkPolicy>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    serves: Vec<&'a ServesCapability>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    provides: &'a [String],
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    accepts_delegation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_delegation_concurrent: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_config: Option<&'a RuntimeConfig>,
    #[serde(flatten)]
    extra: &'a BTreeMap<String, serde_yaml::Value>,
}

/// Splits `---` delimited YAML frontmatter from the body. No opening
/// `---` means no frontmatter; an opening without a close is an error.
/// Handles a BOM and CRLF line endings.
pub fn split_frontmatter(content: &str) -> Result<(Option<&str>, &str), String> {
    let content = content.trim_start_matches('\u{feff}');
    let mut offset = 0;
    let mut yaml_start = 0;
    for (index, line) in content.split_inclusive('\n').enumerate() {
        let start = offset;
        offset += line.len();
        let is_delimiter = line.trim_end() == "---";
        if index == 0 {
            if !is_delimiter {
                return Ok((None, content));
            }
            yaml_start = offset;
        } else if is_delimiter {
            return Ok((Some(&content[yaml_start..start]), &content[offset..]));
        }
    }
    if offset > 0 && content.lines().next().map(str::trim_end) == Some("---") {
        return Err("missing closing '---'".to_string());
    }
    Ok((None, content))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> AgentId {
        AgentId::parse(s).unwrap()
    }

    const FULL: &str = r#"---
name: Research Agent
description: Finds papers
model: claude:claude-sonnet-5
image: nandi/research:latest
memory_mb: 2048
cpu: 1.5
shell_timeout_s: 600
allowed-tools: [shell, read_file]
requires_approval: [shell]
pilots: [browser]
command: run.sh
network:
  allow:
    - host: API.example.com
      methods: [get]
aws:
  mode: sts
  services:
    - service: s3
      actions: [GetObject]
---

# Researcher
Find papers.
"#;

    #[test]
    fn parses_every_field() {
        let agent = Agent::from_skill_md(id("research"), FULL).unwrap();
        assert_eq!(agent.name(), "Research Agent");
        assert_eq!(agent.description(), "Finds papers");
        assert_eq!(agent.model().unwrap().to_string(), "claude:claude-sonnet-5");
        assert_eq!(agent.model().unwrap().provider_kind(), Provider::Anthropic);
        assert_eq!(agent.container().image(), Some("nandi/research:latest"));
        assert_eq!(agent.container().memory_mb(), Some(2048));
        assert_eq!(agent.container().cpu(), Some(1.5));
        assert_eq!(agent.container().shell_timeout_s(), Some(600));
        assert!(agent.allows_tool("read_file") && !agent.allows_tool("write_file"));
        assert!(agent.requires_approval_for("shell"));
        assert!(!agent.requires_approval_for("read_file"));
        assert_eq!(agent.capabilities().pilots()[0].as_str(), "browser");
        assert_eq!(
            agent.capabilities().network().allow[0].host,
            "api.example.com"
        );
        assert_eq!(agent.skill_body(), "# Researcher\nFind papers.");
        assert!(agent.extra.contains_key("command"));
    }

    #[test]
    fn round_trips_through_skill_md() {
        let agent = Agent::from_files(id("research"), FULL, Some("Cite sources.")).unwrap();
        let skill_md = agent.to_skill_md();
        let again = Agent::from_files(id("research"), &skill_md, Some(&agent.to_instructions_md()))
            .unwrap();
        assert_eq!(again, agent);
        // Rendering is stable
        assert_eq!(again.to_skill_md(), skill_md);
        assert!(skill_md.contains("command: run.sh"), "{skill_md}");

        let minimal = Agent::from_skill_md(id("plain"), "Just a prompt").unwrap();
        let again = Agent::from_skill_md(id("plain"), &minimal.to_skill_md()).unwrap();
        assert_eq!(again, minimal);
        assert_eq!(minimal.name(), "plain");
        assert!(minimal.model().is_none());
    }

    #[test]
    fn handles_crlf_bom_and_empty_frontmatter() {
        let agent =
            Agent::from_skill_md(id("w"), "\u{feff}---\r\nname: Win\r\n---\r\nBody\r\n").unwrap();
        assert_eq!(agent.name(), "Win");
        assert_eq!(agent.skill_body(), "Body");
        assert_eq!(
            Agent::from_skill_md(id("e"), "---\n---\nBody")
                .unwrap()
                .skill_body(),
            "Body"
        );
        assert!(Agent::from_skill_md(id("e"), "").is_ok());
    }

    #[test]
    fn parses_delegation_fields_and_defaults_them_off() {
        let content = "---
name: R
serves:
  - name: find-papers
    description: Finds papers
    input_schema: {type: object}
accepts_delegation: true
max_delegation_concurrent: 2
---
Body";
        let agent = Agent::from_skill_md(id("r"), content).unwrap();
        let caps = agent.capabilities();
        assert!(caps.accepts_delegation());
        assert_eq!(caps.delegation_limit(), 2);
        assert_eq!(caps.serves()[0].name, "find-papers");
        assert_eq!(
            caps.serves()[0].input_schema,
            Some(serde_json::json!({"type": "object"}))
        );
        let again = Agent::from_skill_md(id("r"), &agent.to_skill_md()).unwrap();
        assert_eq!(again, agent);

        let plain = Agent::from_skill_md(
            id("p"),
            "---
name: P
---
",
        )
        .unwrap();
        assert!(!plain.capabilities().accepts_delegation());
        assert!(plain.capabilities().serves().is_empty());
        let md = plain.to_skill_md();
        assert!(!md.contains("serves") && !md.contains("delegation"), "{md}");
        assert!(Agent::from_skill_md(
            id("z"),
            "---
max_delegation_concurrent: 0
---
"
        )
        .is_err());
    }

    #[test]
    fn a_half_merged_file_isnt_an_agent() {
        let merged =
            "---\nname: A\n---\n<<<<<<< HEAD\nBe brief.\n=======\nBe thorough.\n>>>>>>> 8a5fc95\n";
        let error = Agent::from_skill_md(id("a"), merged)
            .unwrap_err()
            .to_string();
        assert!(error.contains("merge conflict"), "{error}");
        assert!(Agent::from_files(id("a"), "---\nname: A\n---\nHi\n", Some(merged)).is_err());
        // A Markdown heading underline isn't a conflict
        let heading = "---\nname: A\n---\nRules\n=======\nBe brief.\n";
        assert!(Agent::from_skill_md(id("a"), heading).is_ok());
    }

    #[test]
    fn allowing_sites_adds_them_to_network_allow_once() {
        let id = AgentId::parse("browser").unwrap();
        let mut agent = Agent::from_skill_md(
            id.clone(),
            "---
name: Browser
network:
  allow:
    - host: example.com
---
Body
",
        )
        .unwrap();
        assert!(agent
            .allow_sites(&["Shop.Example.org".into(), "*.shop.example.org".into()])
            .unwrap());
        let hosts: Vec<&str> = agent
            .capabilities()
            .network()
            .allow
            .iter()
            .map(|r| r.host.as_str())
            .collect();
        assert_eq!(
            hosts,
            ["example.com", "shop.example.org", "*.shop.example.org"]
        );
        assert!(
            !agent.allow_sites(&["example.com".into()]).unwrap(),
            "already there"
        );
        // Written back, and read again the same
        let again = Agent::from_skill_md(id, &agent.to_skill_md()).unwrap();
        assert_eq!(
            again.capabilities().network(),
            agent.capabilities().network()
        );
        assert!(agent.allow_sites(&["a.*.com".into()]).is_err());
    }

    #[test]
    fn provides_is_shorthand_for_bare_serves_entries() {
        let content = "---
name: R
serves:
  - name: find-papers
    description: Finds papers
provides: [test-cap, find-papers]
---
Body";
        let agent = Agent::from_skill_md(id("r"), content).unwrap();
        let names: Vec<&str> = agent
            .capabilities()
            .serves()
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["find-papers", "test-cap"]);
        assert_eq!(agent.capabilities().provides(), ["test-cap"]);
        // Written back as it was: test-cap under provides, not serves
        let md = agent.to_skill_md();
        assert!(md.contains("provides:\n- test-cap"), "{md}");
        assert_eq!(md.matches("name: test-cap").count(), 0, "{md}");
        let again = Agent::from_skill_md(id("r"), &md).unwrap();
        assert_eq!(again, agent);

        assert!(Agent::from_skill_md(id("z"), "---\nprovides: [a, a]\n---\n").is_err());
        assert!(Agent::from_skill_md(id("z"), "---\nprovides: ['a b']\n---\n").is_err());
    }

    #[test]
    fn every_frontmatter_key_is_documented() {
        let keys: Vec<&str> = SKILL_FRONTMATTER_KEYS.iter().map(|(k, _)| *k).collect();
        // Every documented key is one the parser reads (none land in `extra`)
        let yaml = "name: A\ndescription: d\nmodel: claude:claude-sonnet-5\nimage: img\n\
            memory_mb: 512\ncpu: 1\nshell_timeout_s: 120\nallowed-tools: [shell]\nrequires_approval: [shell]\n\
            pilots: []\naws: {services: []}\nnetwork: {allow: []}\n\
            serves: [{name: x, description: y}]\nprovides: [z]\naccepts_delegation: true\n\
            max_delegation_concurrent: 2\nruntime: claude-code\nruntime_config: {model: sonnet}\n\
            connectors: [filesystem]\ntags: [finance]\n";
        let fm: Frontmatter = serde_yaml::from_str(yaml).unwrap();
        let tagged = Agent::from_skill_md(id("a"), &format!("---\n{yaml}---\nBody\n")).unwrap();
        assert_eq!(tagged.tags(), ["finance"]);
        let undocumented: Vec<&String> = fm
            .extra
            .keys()
            .filter(|k| !EXTRA_FRONTMATTER_KEYS.contains(&k.as_str()))
            .collect();
        assert!(undocumented.is_empty(), "undocumented: {undocumented:?}");
        let written: Vec<String> = serde_yaml::from_str::<serde_yaml::Mapping>(yaml)
            .unwrap()
            .keys()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect();
        assert_eq!(written, keys, "the fixture covers every documented key");

        // Every key the writer emits is documented
        let agent = Agent::from_skill_md(id("a"), &format!("---\n{yaml}---\nBody\n")).unwrap();
        let skill_md = agent.to_skill_md();
        let (out, _) = split_frontmatter(&skill_md).unwrap();
        let out: serde_yaml::Mapping = serde_yaml::from_str(out.unwrap()).unwrap();
        for key in out.keys().filter_map(|k| k.as_str()) {
            assert!(
                keys.contains(&key),
                "'{key}' missing from SKILL_FRONTMATTER_KEYS"
            );
        }
    }

    #[test]
    fn rejects_invalid_files() {
        for (content, expected) in [
            ("---\nname: [unclosed\n---\n", "Invalid skill.md"),
            ("---\nname: No end\n", "missing closing"),
            ("---\nmodel: gemini:pro\n---\n", "unknown"),
            ("---\nmodel: claude\n---\n", "provider"),
            ("---\ncpu: -1\n---\n", "cpu"),
            ("---\nshell_timeout_s: 0\n---\n", "shell_timeout_s"),
            ("---\nshell_timeout_s: 7200\n---\n", "shell_timeout_s"),
            (
                "---\nnetwork:\n  allow:\n    - host: \"a.*.com\"\n---\n",
                "invalid host",
            ),
            ("---\naws:\n  mode: root\n---\n", "Invalid skill.md"),
            ("---\npilots: [\"../x\"]\n---\n", "pilot"),
        ] {
            let error = Agent::from_skill_md(id("a"), content)
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{content:?}: {error}");
        }
    }

    #[test]
    fn new_enforces_invariants() {
        assert!(Agent::new(AgentDraft::named(id("a"), "  ")).is_err());
        let mut draft = AgentDraft::named(id("a"), "A");
        draft.skill_body = "x".repeat(MAX_INSTRUCTIONS_CHARS + 1);
        assert!(Agent::new(draft).is_err());
        assert!(
            AgentId::parse("Not_Valid").is_err(),
            "IDs are checked by their type"
        );

        let mut agent = Agent::new(AgentDraft::named(id("a"), "A")).unwrap();
        assert!(agent.update_profile("", "x").is_err());
        assert_eq!(agent.name(), "A", "a failed update changes nothing");
    }

    #[test]
    fn forks_drop_source_metadata_and_keep_the_rest() {
        let original = Agent::from_skill_md(
            id("arxiv"),
            "---\nname: ArXiv\nmodel: claude:claude-sonnet-5\nsource: github.com/a/b\npublisher: alice\nversion: 1.2.0\ncustom: kept\nallowed-tools: [shell]\n---\nSearch arXiv.",
        )
        .unwrap();
        let fork = original.forked(id("my-arxiv"), " My ArXiv ").unwrap();
        assert_eq!(fork.id().as_str(), "my-arxiv");
        assert_eq!(fork.name(), "My ArXiv");
        assert_eq!(fork.skill_body(), "Search arXiv.");
        assert_eq!(fork.model(), original.model());
        let md = fork.to_skill_md();
        for dropped in ["source:", "publisher:", "version:"] {
            assert!(!md.contains(dropped), "{dropped} in {md}");
        }
        assert!(md.contains("custom: kept"));
        assert!(md.contains("allowed-tools"));
        assert!(original.forked(id("x"), "  ").is_err());
    }

    #[test]
    fn instructions_extend_the_system_prompt() {
        let mut agent = Agent::from_skill_md(id("a"), "---\nname: A\n---\nBe kind.").unwrap();
        assert_eq!(agent.system_prompt(), "Be kind.");
        agent.update_instructions("  Cite sources.\n").unwrap();
        assert_eq!(agent.system_prompt(), "Be kind.\n\nCite sources.");
        assert_eq!(agent.to_instructions_md(), "Cite sources.\n");
        assert!(agent
            .update_instructions(&"x".repeat(MAX_INSTRUCTIONS_CHARS + 1))
            .is_err());
    }

    #[test]
    fn defaults_to_approving_mutating_tools_and_the_settings_model() {
        let agent = Agent::from_skill_md(id("a"), "---\nname: A\n---\n").unwrap();
        assert!(agent.allows_tool("anything"));
        assert!(agent.requires_approval_for("shell") && agent.requires_approval_for("write_file"));
        let default = ModelRef::parse("deepseek:deepseek-flash").unwrap();
        assert_eq!(agent.effective_model(&default), default);
    }

    #[test]
    fn model_refs_parse_both_forms() {
        let text: ModelRef = serde_yaml::from_str("ollama:llama3.2").unwrap();
        let mapping: ModelRef = serde_yaml::from_str("{provider: ollama, name: llama3.2}").unwrap();
        assert_eq!(text, mapping);
        assert_eq!(
            serde_yaml::to_string(&text).unwrap().trim(),
            "ollama:llama3.2"
        );
        assert!(ModelRef::parse("ollama:").is_err());
    }

    #[test]
    fn pilot_options_round_trip_and_typos_are_errors() {
        let id = AgentId::parse("shopper").unwrap();
        let skill = "---
name: Shopper
pilots:
  - opendesk
  - id: browser-headless
    allow: [interact, submit]
    values: {email: dev@acme.test}
    secrets: {SITE_PASSWORD: nandi.pilot.shopper.site}
---
Buy things.
";
        let agent = Agent::from_skill_md(id.clone(), skill).unwrap();
        let options = agent
            .capabilities()
            .pilot_options("browser-headless")
            .unwrap();
        assert!(options.allows("submit") && !options.allows("pay"));
        assert_eq!(options.values["email"], "dev@acme.test");
        assert_eq!(
            options.secrets["SITE_PASSWORD"].as_str(),
            "nandi.pilot.shopper.site"
        );
        assert!(agent.capabilities().pilot_options("opendesk").is_none());
        // Written back the same way: a bare ID, and a mapping
        let again = Agent::from_skill_md(id.clone(), &agent.to_skill_md()).unwrap();
        assert_eq!(again.capabilities(), agent.capabilities());
        assert!(agent.to_skill_md().contains("- opendesk"));

        for bad in [
            "pilots:
  - id: browser
    alow: [submit]",
            "pilots:
  - id: browser
    allow: [everything]",
            "pilots:
  - id: browser
    secrets: {password: nandi.pilot.x}",
            "pilots:
  - id: browser
    secrets: {PASSWORD: hunter2}",
        ] {
            let skill = format!(
                "---
{bad}
---
"
            );
            assert!(Agent::from_skill_md(id.clone(), &skill).is_err(), "{bad}");
        }
    }
}
