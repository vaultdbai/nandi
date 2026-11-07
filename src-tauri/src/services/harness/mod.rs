//! Harness adapters (wave B): agents whose `runtime:` is a CLI harness
//! (Claude Code, Codex, Gemini CLI, or a custom ACP executable) run in that
//! CLI instead of Nandi's own loop. Nandi spawns the CLI (in the harness
//! container by default), speaks ACP over its stdio, and translates the
//! session's updates into the same run events, rows, and evidence a native
//! run produces. The CLI owns LLM auth (Anthropic, Bedrock, Azure, Vertex,
//! OpenAI) through its own config.
//!
//! - `acp`: sessions on the official ACP SDK; `adapter`: the frozen contract
//! - `claude_code`, `codex`, `gemini_cli`, `custom`: concrete adapters
//! - `claude_sdk`: Nandi's own `claude:` model calls through Claude Code's
//!   sign-in (the Claude Agent SDK; tool-less completions, never agent code)
//! - `detection`, `health_monitor`: which CLIs are installed and signed in
//! - `event_translation`, `evidence`: ACP updates → run events and rows
//! - `aws_auth`, `aws_config_parser`, `bedrock_models`, `gateway`: Bedrock

pub mod acp;
pub mod adapter;
pub mod approvals;
pub mod aws_auth;
pub mod aws_config_parser;
pub mod bedrock_models;
pub mod claude_code;
pub mod claude_sdk;
pub mod codex;
pub mod custom;
pub mod detection;
pub mod event_translation;
pub mod evidence;
pub mod gateway;
pub mod gemini_cli;
pub mod health_monitor;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use tokio_util::sync::CancellationToken;

pub use adapter::{
    HarnessAdapter, HarnessAuth, HarnessAuthKind, HarnessConfig, HarnessError, HarnessHealth,
    HarnessRuntime, HarnessSession, ModelRef,
};

use crate::db::{now_ms, ts, Db};
use crate::model::Timestamp;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::AppState;
use detection::{DetectEnv, Detection, HarnessDetector};

pub const HARNESS_DETECTED: &str = "harness://detected";
pub const HARNESS_HEALTH_CHANGED: &str = "harness://health_changed";
pub const HARNESS_SESSION_STARTED: &str = "harness://session_started";
pub const HARNESS_SESSION_ENDED: &str = "harness://session_ended";
pub const HARNESS_TURN_DELTA: &str = "harness://turn_delta";
pub const HARNESS_TURN_FINISHED: &str = "harness://turn_finished";
pub const HARNESS_PERMISSION_REQUEST: &str = "harness://permission_request";
pub const HARNESS_ERROR: &str = "harness://error";
pub const AWS_PROFILE_CHANGED: &str = "aws://profile_changed";
pub const AWS_CREDENTIALS_EXPIRED: &str = "aws://credentials_expired";
pub const BEDROCK_MODELS_REFRESHED: &str = "bedrock://models_refreshed";
pub const BEDROCK_GATEWAY_STATUS: &str = "bedrock://gateway_status";

pub use crate::runtime::HARNESS_IMAGE;
/// Sessions listed when the caller gives no limit
const DEFAULT_SESSION_LIMIT: u32 = 100;

/// A harness as Settings → Harnesses shows it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessInfo {
    /// `claude-code`, `codex`, `gemini-cli`, `custom:<id>`
    pub id: String,
    pub display_name: String,
    /// On the host (where the user signs in)
    pub executable: Option<String>,
    /// e.g. `~/.claude`
    pub config_dir: Option<String>,
    pub health: HarnessHealth,
    pub is_default: bool,
    pub default_model: Option<String>,
    pub supported_models: Vec<ModelRef>,
    pub detected_at: Option<Timestamp>,
    /// The harness image is here. When it isn't, the first run downloads
    /// it (nothing is ever built)
    pub image_available: bool,
}

/// One end-to-end check: start the CLI, open a session, close it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessTestResult {
    pub harness_id: String,
    pub ok: bool,
    pub latency_ms: Option<u32>,
    pub version: Option<String>,
    pub configured_provider: Option<String>,
    pub message: String,
    pub error: Option<String>,
}

/// The harness new agents get, with its default model (machine-local)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessDefault {
    pub harness_id: String,
    pub model: Option<String>,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum HarnessSessionStatus {
    Starting,
    Running,
    Finished,
    Failed,
    Killed,
}

impl HarnessSessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Finished => "finished",
            Self::Failed => "failed",
            Self::Killed => "killed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "starting" => Self::Starting,
            "running" => Self::Running,
            "finished" => Self::Finished,
            "killed" => Self::Killed,
            _ => Self::Failed,
        }
    }

    pub fn is_live(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

/// A `harness_sessions` row
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSessionSummary {
    pub id: String,
    pub adapter_id: String,
    pub agent_id: Option<String>,
    pub run_id: Option<String>,
    pub status: HarnessSessionStatus,
    pub child_pid: Option<u32>,
    pub container_name: Option<String>,
    pub working_dir: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub stop_reason: Option<String>,
    pub error: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub started_at: Timestamp,
    pub ended_at: Option<Timestamp>,
}

/// A `harness_events` row: one translated ACP update
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessEventRecord {
    pub session_id: String,
    pub seq: u32,
    /// The `AcpSessionUpdate` kind, e.g. `messageDelta`
    pub kind: String,
    pub payload: Value,
    pub created_at: Timestamp,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// `harness://detected`: detection ran; refetch the list
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessDetected {
    pub harness_ids: Vec<String>,
}

impl NandiEvent for HarnessDetected {
    const NAME: &'static str = HARNESS_DETECTED;
}

/// `harness://health_changed`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessHealthChanged {
    pub harness_id: String,
    pub health: HarnessHealth,
}

impl NandiEvent for HarnessHealthChanged {
    const NAME: &'static str = HARNESS_HEALTH_CHANGED;
}

/// `harness://session_started`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSessionStarted {
    pub session_id: String,
    pub adapter_id: String,
    pub agent_id: Option<String>,
    pub run_id: Option<String>,
}

impl NandiEvent for HarnessSessionStarted {
    const NAME: &'static str = HARNESS_SESSION_STARTED;
}

/// `harness://session_ended`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSessionEnded {
    pub session_id: String,
    pub status: HarnessSessionStatus,
    pub error: Option<String>,
}

impl NandiEvent for HarnessSessionEnded {
    const NAME: &'static str = HARNESS_SESSION_ENDED;
}

/// `harness://turn_finished`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessTurnFinished {
    pub session_id: String,
    pub run_id: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<acp::AcpUsage>,
}

impl NandiEvent for HarnessTurnFinished {
    const NAME: &'static str = HARNESS_TURN_FINISHED;
}

/// `harness://error`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessErrorEvent {
    pub harness_id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub message: String,
}

impl NandiEvent for HarnessErrorEvent {
    const NAME: &'static str = HARNESS_ERROR;
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// A live session's kill switch
struct LiveHandle {
    kill: CancellationToken,
    container_name: Option<String>,
}

/// Harness sessions, the adapters behind them, and detection
pub struct HarnessService {
    db: Db,
    bus: EventBus,
    detector: Arc<HarnessDetector>,
    live: Mutex<HashMap<String, LiveHandle>>,
}

/// What a session row starts with besides the adapter's session
#[derive(Debug, Clone, Default)]
pub struct SessionMeta {
    pub agent_id: Option<String>,
    pub run_id: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub container_name: Option<String>,
}

impl HarnessService {
    pub fn new(db: Db, bus: EventBus, detector: Arc<HarnessDetector>) -> Self {
        Self {
            db,
            bus,
            detector,
            live: Mutex::default(),
        }
    }

    pub fn detector(&self) -> &Arc<HarnessDetector> {
        &self.detector
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Built-in harnesses, then registered custom ones
    pub async fn list_adapters(&self) -> Result<Vec<HarnessRuntime>, String> {
        let custom = self
            .db
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT harness_id FROM harness_preferences
                      WHERE harness_id LIKE 'custom:%' AND command IS NOT NULL
                      ORDER BY harness_id",
                )?;
                let ids = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(ids)
            })
            .await?;
        let mut adapters = HarnessRuntime::BUILT_IN.to_vec();
        adapters.extend(custom.iter().filter_map(|id| HarnessRuntime::parse(id)));
        Ok(adapters)
    }

    /// A fresh adapter for `harness_id`, ready to `start`
    pub async fn get_adapter(&self, harness_id: &str) -> Result<Box<dyn HarnessAdapter>, String> {
        let runtime = HarnessRuntime::parse(harness_id)
            .ok_or_else(|| format!("Unknown harness '{harness_id}'"))?;
        let custom = match &runtime {
            HarnessRuntime::Custom(_) => Some(self.custom_command(harness_id).await?),
            _ => None,
        };
        new_adapter(&runtime, self.detector.env().clone(), custom)
    }

    async fn custom_command(&self, harness_id: &str) -> Result<adapter::Invocation, String> {
        let id = harness_id.to_string();
        let row = self
            .db
            .call(move |conn| {
                conn.query_row(
                    "SELECT command, args FROM harness_preferences WHERE harness_id = ?1",
                    [&id],
                    |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
            })
            .await?;
        match row {
            Some((Some(program), args)) => Ok(adapter::Invocation {
                program,
                args: serde_json::from_str(&args).unwrap_or_default(),
            }),
            _ => Err(format!(
                "Custom harness '{harness_id}' has no command: register it first"
            )),
        }
    }

    /// Starts `adapter` and records the session. The token is cancelled by
    /// `kill_session`; whoever drives the session stops the adapter then.
    pub async fn start_session(
        &self,
        adapter: &dyn HarnessAdapter,
        config: HarnessConfig,
        meta: SessionMeta,
    ) -> Result<(HarnessSession, CancellationToken), HarnessError> {
        let session = match adapter.start(config).await {
            Ok(session) => session,
            Err(error) => {
                events::emit(
                    &self.bus,
                    &HarnessErrorEvent {
                        harness_id: adapter.id().to_string(),
                        session_id: None,
                        run_id: meta.run_id.clone(),
                        message: error.to_string(),
                    },
                );
                return Err(error);
            }
        };
        evidence::session_started(
            &self.db,
            evidence::SessionStart {
                id: session.id.clone(),
                adapter_id: session.adapter_id.clone(),
                agent_id: meta.agent_id.clone(),
                run_id: meta.run_id.clone(),
                acp_session_id: None,
                child_pid: Some(session.child_pid),
                container_name: meta.container_name.clone(),
                working_dir: session.working_dir.display().to_string(),
                model: meta.model,
                provider: meta.provider,
            },
        )
        .await
        .map_err(HarnessError::Other)?;
        let kill = CancellationToken::new();
        if let Ok(mut live) = self.live.lock() {
            live.insert(
                session.id.clone(),
                LiveHandle {
                    kill: kill.clone(),
                    container_name: meta.container_name,
                },
            );
        }
        events::emit(
            &self.bus,
            &HarnessSessionStarted {
                session_id: session.id.clone(),
                adapter_id: session.adapter_id.clone(),
                agent_id: meta.agent_id,
                run_id: meta.run_id,
            },
        );
        Ok((session, kill))
    }

    /// Records one update of a live session
    pub async fn record(&self, session_id: &str, update: &acp::AcpSessionUpdate) {
        if let Err(e) = evidence::record_event(&self.db, session_id, update).await {
            log::warn!("Could not record harness event for session {session_id}: {e}");
        }
    }

    /// Closes the session row and forgets the kill switch
    pub async fn end_session(
        &self,
        session_id: &str,
        status: HarnessSessionStatus,
        stop_reason: Option<String>,
        error: Option<String>,
        usage: acp::AcpUsage,
    ) {
        if let Ok(mut live) = self.live.lock() {
            live.remove(session_id);
        }
        if let Err(e) = evidence::session_ended(
            &self.db,
            session_id,
            status,
            stop_reason,
            error.clone(),
            usage,
        )
        .await
        {
            log::warn!("Could not close harness session {session_id}: {e}");
        }
        events::emit(
            &self.bus,
            &HarnessSessionEnded {
                session_id: session_id.to_string(),
                status,
                error,
            },
        );
    }

    pub async fn session_by_id(&self, session_id: &str) -> Result<HarnessSessionSummary, String> {
        let id = session_id.to_string();
        self.db
            .call(move |conn| evidence::get_session(conn, &id))
            .await?
            .ok_or_else(|| format!("Harness session '{session_id}' not found"))
    }

    pub fn is_live(&self, session_id: &str) -> bool {
        self.live
            .lock()
            .is_ok_and(|live| live.contains_key(session_id))
    }

    /// Stops a live session: its driver sees the token and stops the CLI;
    /// the container goes too, in case the driver is stuck
    pub async fn kill_session(&self, session_id: &str) -> Result<(), String> {
        let handle = self
            .live
            .lock()
            .ok()
            .and_then(|mut live| live.remove(session_id));
        let Some(handle) = handle else {
            let session = self.session_by_id(session_id).await?;
            return if session.status.is_live() {
                // Orphaned: nothing drives it any more
                self.end_session(
                    session_id,
                    HarnessSessionStatus::Killed,
                    None,
                    Some("Killed".to_string()),
                    acp::AcpUsage::default(),
                )
                .await;
                Ok(())
            } else {
                Err(format!("Harness session '{session_id}' is not running"))
            };
        };
        handle.kill.cancel();
        if let Some(name) = &handle.container_name {
            crate::runtime::stop_container(name).await;
        }
        self.end_session(
            session_id,
            HarnessSessionStatus::Killed,
            None,
            Some("Killed by the user".to_string()),
            acp::AcpUsage::default(),
        )
        .await;
        Ok(())
    }

    /// Re-runs detection; emits `harness://health_changed` for every
    /// harness whose health changed, then `harness://detected`
    pub async fn detect(&self) -> Result<Vec<Detection>, String> {
        let before = self.detector.stored().await.unwrap_or_default();
        let detected = self.detector.detect_all().await?;
        claude_sdk::invalidate();
        for detection in &detected {
            let previous = before.iter().find(|b| b.harness_id == detection.harness_id);
            if previous.is_none_or(|p| !same_health(&p.health, &detection.health)) {
                events::emit(
                    &self.bus,
                    &HarnessHealthChanged {
                        harness_id: detection.harness_id.clone(),
                        health: detection.health.clone(),
                    },
                );
            }
        }
        events::emit(
            &self.bus,
            &HarnessDetected {
                harness_ids: detected.iter().map(|d| d.harness_id.clone()).collect(),
            },
        );
        Ok(detected)
    }

    /// Every harness with its stored detection (detecting first if none)
    pub async fn list(&self) -> Result<Vec<HarnessInfo>, String> {
        let mut detections = self.detector.stored().await?;
        if detections.is_empty() {
            detections = self.detect().await?;
        }
        self.infos(detections).await
    }

    async fn infos(&self, detections: Vec<Detection>) -> Result<Vec<HarnessInfo>, String> {
        let default = self.default().await?;
        let image_available = crate::runtime::image_present(HARNESS_IMAGE)
            .await
            .unwrap_or(false);
        let mut infos = Vec::new();
        for runtime in self.list_adapters().await? {
            let id = runtime.id();
            let detection = detections.iter().find(|d| d.harness_id == id);
            let models = self
                .get_adapter(&id)
                .await
                .map(|a| a.supported_models())
                .unwrap_or_default();
            infos.push(HarnessInfo {
                display_name: runtime.display_name(),
                executable: detection
                    .and_then(|d| d.executable.as_ref())
                    .map(|p| p.display().to_string()),
                config_dir: detection
                    .and_then(|d| d.config_dir.as_ref())
                    .map(|p| p.display().to_string()),
                health: detection.map(|d| d.health.clone()).unwrap_or_default(),
                is_default: default.as_ref().is_some_and(|d| d.harness_id == id),
                default_model: default
                    .as_ref()
                    .filter(|d| d.harness_id == id)
                    .and_then(|d| d.model.clone()),
                supported_models: models,
                detected_at: detection.map(|d| d.health.last_check),
                image_available,
                id,
            });
        }
        Ok(infos)
    }

    pub async fn health(&self, harness_id: &str) -> Result<HarnessHealth, String> {
        if HarnessRuntime::parse(harness_id).is_none() {
            return Err(format!("Unknown harness '{harness_id}'"));
        }
        match self.detector.stored_one(harness_id).await? {
            Some(detection) => Ok(detection.health),
            None => {
                let detected = self.detect().await?;
                Ok(detected
                    .into_iter()
                    .find(|d| d.harness_id == harness_id)
                    .map(|d| d.health)
                    .unwrap_or_default())
            }
        }
    }

    pub async fn set_default(
        &self,
        harness_id: &str,
        model: Option<String>,
    ) -> Result<HarnessDefault, String> {
        if HarnessRuntime::parse(harness_id).is_none() {
            return Err(format!("Unknown harness '{harness_id}'"));
        }
        let (id, now) = (harness_id.to_string(), now_ms());
        let model = model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        let saved = model.clone();
        self.db
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute("UPDATE harness_preferences SET is_default = 0", [])?;
                tx.execute(
                    "INSERT INTO harness_preferences (harness_id, is_default, default_model, updated_at)
                     VALUES (?1, 1, ?2, ?3)
                     ON CONFLICT(harness_id) DO UPDATE SET is_default = 1,
                         default_model = ?2, updated_at = ?3",
                    params![id, saved, now],
                )?;
                tx.commit()
            })
            .await?;
        Ok(HarnessDefault {
            harness_id: harness_id.to_string(),
            model,
            updated_at: ts(now),
        })
    }

    pub async fn clear_default(&self) -> Result<(), String> {
        self.db
            .call(|conn| {
                conn.execute(
                    "UPDATE harness_preferences SET is_default = 0, updated_at = ?1
                      WHERE is_default = 1",
                    [now_ms()],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn default(&self) -> Result<Option<HarnessDefault>, String> {
        self.db
            .call(|conn| {
                conn.query_row(
                    "SELECT harness_id, default_model, updated_at FROM harness_preferences
                      WHERE is_default = 1",
                    [],
                    |row| {
                        Ok(HarnessDefault {
                            harness_id: row.get(0)?,
                            model: row.get(1)?,
                            updated_at: ts(row.get(2)?),
                        })
                    },
                )
                .optional()
            })
            .await
    }

    pub async fn sessions(
        &self,
        agent_id: Option<String>,
        limit: Option<u32>,
    ) -> Result<Vec<HarnessSessionSummary>, String> {
        let limit = limit.unwrap_or(DEFAULT_SESSION_LIMIT).clamp(1, 1_000);
        self.db
            .call(move |conn| evidence::list_sessions(conn, agent_id.as_deref(), limit))
            .await
    }

    pub async fn events(
        &self,
        session_id: &str,
        after_seq: Option<u32>,
    ) -> Result<Vec<HarnessEventRecord>, String> {
        self.session_by_id(session_id).await?;
        let id = session_id.to_string();
        self.db
            .call(move |conn| evidence::list_events(conn, &id, after_seq.unwrap_or(0)))
            .await
    }

    /// Whether a run can start here: the harness's detection, then a
    /// session that opens and closes on its adapter
    pub async fn test(&self, harness_id: &str) -> Result<HarnessTestResult, String> {
        let runtime = HarnessRuntime::parse(harness_id)
            .ok_or_else(|| format!("Unknown harness '{harness_id}'"))?;
        let started = std::time::Instant::now();
        let health = self.health(harness_id).await?;
        let outcome = match self.get_adapter(harness_id).await {
            Ok(adapter) if health.installed => match self.test_config(harness_id).await {
                Ok(config) => adapter::test_session(adapter, config).await,
                Err(error) => Err(HarnessError::Other(error)),
            },
            Ok(_) => Err(HarnessError::NotInstalled(runtime.display_name())),
            Err(error) => Err(HarnessError::Other(error)),
        };
        let latency_ms = Some(started.elapsed().as_millis().min(u32::MAX as u128) as u32);
        let (ok, message, error) = match outcome {
            Ok(message) => (true, message, None),
            Err(error) => (
                false,
                format!("{} could not start a session", runtime.display_name()),
                Some(error.to_string()),
            ),
        };
        Ok(HarnessTestResult {
            harness_id: harness_id.to_string(),
            ok,
            latency_ms,
            version: health.version,
            configured_provider: health.configured_provider,
            message,
            error,
        })
    }
}

impl HarnessService {
    /// A throwaway session config: an empty temp folder, in the harness
    /// container (its image and tools fetched first, if they aren't here)
    async fn test_config(&self, harness_id: &str) -> Result<HarnessConfig, String> {
        use crate::services::images::{self, tools::ToolBundle, SandboxImage};
        let tools = match ToolBundle::for_harness(harness_id) {
            Some(bundle) => vec![images::tools::ensure(bundle).await?],
            None => {
                images::ensure(SandboxImage::Harness).await?;
                Vec::new()
            }
        };
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let dir = std::env::temp_dir().join(format!("nandi-harness-test-{suffix}"));
        let _ = tokio::fs::create_dir_all(&dir).await;
        Ok(HarnessConfig {
            working_dir: dir.clone(),
            max_turns: 1,
            placement: adapter::Placement::Container {
                image: HARNESS_IMAGE.to_string(),
                name: format!("nandi-harness-test-{}", &suffix[..12]),
                host_workdir: dir,
                memory_mb: None,
                cpu: None,
                network: None,
                folders: Vec::new(),
                tools,
            },
            request_timeout: Some(std::time::Duration::from_secs(90)),
            ..HarnessConfig::default()
        })
    }
}

fn same_health(a: &HarnessHealth, b: &HarnessHealth) -> bool {
    a.installed == b.installed
        && a.version == b.version
        && a.host_authenticated == b.host_authenticated
        && a.container_authenticated == b.container_authenticated
        && a.configured_provider == b.configured_provider
        && a.error == b.error
}

/// The adapter for `runtime` (`custom`: the registered command)
pub fn new_adapter(
    runtime: &HarnessRuntime,
    env: DetectEnv,
    custom: Option<adapter::Invocation>,
) -> Result<Box<dyn HarnessAdapter>, String> {
    Ok(match runtime {
        HarnessRuntime::ClaudeCode => Box::new(claude_code::ClaudeCodeAdapter::with_env(env)),
        HarnessRuntime::Codex => Box::new(codex::CodexAdapter::with_env(env)),
        HarnessRuntime::GeminiCli => Box::new(gemini_cli::GeminiCliAdapter::with_env(env)),
        HarnessRuntime::Custom(id) => {
            let invocation =
                custom.ok_or_else(|| format!("Custom harness '{id}' has no command"))?;
            Box::new(custom::CustomAdapter::with_command(invocation, env))
        }
    })
}

/// Builds the service, closes sessions a previous run left open, and (once
/// per database, outside tests) runs the first detection and starts the
/// health monitor. Must not block.
pub fn init(state: &AppState) -> Option<(Arc<HarnessService>, Arc<HarnessDetector>)> {
    // Tests never probe the real machine: an empty PATH and home
    let env = if cfg!(test) {
        let empty = state.app_data_dir().join("harness-test-home");
        DetectEnv {
            path: Some(empty.clone().into_os_string()),
            home: empty,
            vars: HashMap::new(),
        }
    } else {
        DetectEnv::current()
    };
    let store = state.store().clone();
    claude_sdk::set_bedrock_route_source(Box::new(move || {
        let repo = crate::services::config::repo_config::harness_defaults(&store);
        (repo.provider.as_deref() == Some("bedrock")).then_some(claude_sdk::BedrockRoute {
            aws_profile: repo.aws_profile,
            aws_region: repo.aws_region,
        })
    }));
    let detector = Arc::new(HarnessDetector::new(state.db().clone(), env));
    let service = Arc::new(HarnessService::new(
        state.db().clone(),
        state.bus().clone(),
        detector.clone(),
    ));
    let db = state.db().clone();
    let background = service.clone();
    state.runtime().spawn(async move {
        if let Err(e) = evidence::close_orphaned_sessions(&db).await {
            log::warn!("Could not close orphaned harness sessions: {e}");
        }
        if cfg!(test) {
            return;
        }
        // First startup after v13: populate `detected_harnesses`
        if background
            .detector
            .stored()
            .await
            .is_ok_and(|d| d.is_empty())
        {
            if let Err(e) = background.detect().await {
                log::warn!("Harness detection failed: {e}");
            }
        }
        health_monitor::run(Arc::downgrade(&background)).await;
    });
    Some((service, detector))
}

// ---------------------------------------------------------------------------
// App-facing API (commands and routes)
// ---------------------------------------------------------------------------

fn service(state: &AppState) -> Result<&Arc<HarnessService>, String> {
    state
        .harnesses()
        .ok_or_else(|| "Harnesses are not available".to_string())
}

pub async fn list_harnesses(state: &AppState) -> Result<Vec<HarnessInfo>, String> {
    let mut infos = service(state)?.list().await?;
    infos
        .iter_mut()
        .for_each(|i| with_nandi_keys(state, &i.id, &mut i.health));
    Ok(infos)
}

pub async fn detect_harnesses(state: &AppState) -> Result<Vec<HarnessInfo>, String> {
    let service = service(state)?;
    let detected = service.detect().await?;
    let mut infos = service.infos(detected).await?;
    infos
        .iter_mut()
        .for_each(|i| with_nandi_keys(state, &i.id, &mut i.health));
    Ok(infos)
}

pub async fn get_harness_health(
    state: &AppState,
    harness_id: &str,
) -> Result<HarnessHealth, String> {
    let mut health = service(state)?.health(harness_id).await?;
    with_nandi_keys(state, harness_id, &mut health);
    Ok(health)
}

/// Nandi's own key for the harness's provider also signs a container in
/// (`runtime_router` passes it as `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`)
fn with_nandi_keys(state: &AppState, harness_id: &str, health: &mut HarnessHealth) {
    use crate::llm::Provider;
    let provider = match harness_id {
        "claude-code" => Provider::Anthropic,
        "codex" => Provider::OpenAi,
        _ => return,
    };
    if !health.container_authenticated
        && crate::services::llm_keys::require_api_key(state.secrets(), provider).is_ok()
    {
        health.container_authenticated = true;
    }
}

pub async fn test_harness(state: &AppState, harness_id: &str) -> Result<HarnessTestResult, String> {
    service(state)?.test(harness_id).await
}

pub async fn set_harness_default(
    state: &AppState,
    harness_id: &str,
    model: Option<String>,
) -> Result<HarnessDefault, String> {
    service(state)?.set_default(harness_id, model).await
}

pub async fn clear_harness_default(state: &AppState) -> Result<(), String> {
    service(state)?.clear_default().await
}

pub async fn get_harness_default(state: &AppState) -> Result<Option<HarnessDefault>, String> {
    service(state)?.default().await
}

pub async fn list_harness_sessions(
    state: &AppState,
    agent_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<HarnessSessionSummary>, String> {
    service(state)?.sessions(agent_id, limit).await
}

pub async fn get_harness_session(
    state: &AppState,
    session_id: &str,
) -> Result<HarnessSessionSummary, String> {
    service(state)?.session_by_id(session_id).await
}

pub async fn get_harness_events(
    state: &AppState,
    session_id: &str,
    after_seq: Option<u32>,
) -> Result<Vec<HarnessEventRecord>, String> {
    service(state)?.events(session_id, after_seq).await
}

pub async fn kill_harness_session(state: &AppState, session_id: &str) -> Result<(), String> {
    service(state)?.kill_session(session_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::RecordingSink;

    fn service() -> (HarnessService, Arc<RecordingSink>) {
        let db = Db::in_memory();
        let sink = Arc::new(RecordingSink::default());
        let bus = EventBus::new(db.clone(), sink.clone());
        let empty = tempfile::tempdir().unwrap().keep();
        let detector = Arc::new(HarnessDetector::new(
            db.clone(),
            DetectEnv {
                home: empty.clone(),
                path: Some(empty.into_os_string()),
                vars: HashMap::new(),
            },
        ));
        (HarnessService::new(db, bus, detector), sink)
    }

    #[tokio::test]
    async fn the_default_harness_is_one_row() {
        let (service, _) = service();
        assert_eq!(service.default().await.unwrap(), None);
        service.set_default("codex", None).await.unwrap();
        let set = service
            .set_default("claude-code", Some(" claude-sonnet-5 ".into()))
            .await
            .unwrap();
        assert_eq!(set.model.as_deref(), Some("claude-sonnet-5"));
        let default = service.default().await.unwrap().unwrap();
        assert_eq!(default.harness_id, "claude-code");
        service.clear_default().await.unwrap();
        assert_eq!(service.default().await.unwrap(), None);
        assert!(service.set_default("nope", None).await.is_err());
    }

    #[tokio::test]
    async fn detection_emits_health_changes_once() {
        let (service, sink) = service();
        service.detect().await.unwrap();
        let first = sink.take();
        assert_eq!(
            first
                .iter()
                .filter(|(n, _)| *n == HARNESS_HEALTH_CHANGED)
                .count(),
            3
        );
        service.detect().await.unwrap();
        let names: Vec<_> = sink.take().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec![HARNESS_DETECTED], "nothing changed");
    }

    #[tokio::test]
    async fn killing_an_orphaned_session_closes_it() {
        let (service, sink) = service();
        evidence::session_started(
            service.db(),
            evidence::SessionStart {
                id: "s1".into(),
                adapter_id: "codex".into(),
                agent_id: None,
                run_id: None,
                acp_session_id: None,
                child_pid: None,
                container_name: None,
                working_dir: ".".into(),
                model: None,
                provider: None,
            },
        )
        .await
        .unwrap();
        service.kill_session("s1").await.unwrap();
        let session = service.session_by_id("s1").await.unwrap();
        assert_eq!(session.status, HarnessSessionStatus::Killed);
        assert!(sink.names().contains(&HARNESS_SESSION_ENDED));
        assert!(service.kill_session("s1").await.is_err(), "not running");
        assert!(service.kill_session("missing").await.is_err());
    }

    #[tokio::test]
    async fn custom_harnesses_need_a_command() {
        let (service, _) = service();
        let error = service.get_adapter("custom:mine").await.err().unwrap();
        assert!(error.contains("no command"), "{error}");
        assert!(service.get_adapter("nope").await.is_err());
    }
}
