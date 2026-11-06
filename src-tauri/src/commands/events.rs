//! Tauri side of the event bus: a sink that emits to the webview, and
//! `tauri_specta::Event` mirrors of every `NandiEvent` so the generated
//! TypeScript bindings keep typed listeners.

use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::services::a2a::reassign::TaskReassigned;
use crate::services::a2a::returns::RequestReturned;
use crate::services::a2a::{DelegationTaskCompleted, DelegationTaskFailed, DelegationTaskStarted};
use crate::services::ask::{AskCompleted, AskStreamingDelta};
use crate::services::config::repo_config::RepoLlmConfigStatus;
use crate::services::connectors::{ConnectorChanged, ConnectorsChanged};
use crate::services::email::EmailChanged;
use crate::services::events::*;
use crate::services::live::LiveChange;
use crate::services::marketplace::events::*;
use crate::services::orchestrator::compaction::AgentRunCompacted;
use crate::services::orchestrator::middleware::AgentTurnAborted;
use crate::services::orchestrator::tool_search::AgentToolsLoaded;
use crate::services::pilots::{PilotChanged, PilotConsentNeeded};
use crate::services::relay::{
    MemberKeyChanged, RelayAgentOffline, RelayAgentOnline, RelaySealRefused, RelayStatus,
};
use crate::services::repo_service::{RepoRebuildCompleted, RepoRebuildStarted};
use crate::services::requests::local::RequestUpdated;
use crate::services::studio::{StudioArtifactLoaded, StudioArtifactSaved, StudioEditPreviewed};
use crate::services::team::TeamChanged;
use crate::services::triggers::{TriggerError, TriggerFired, TriggerRateLimited, TriggersChanged};
use crate::services::work::{WorkItemAdded, WorkItemRemoved, WorkItemUpdated};
use crate::services::workflows::manual_steps::{ManualStepCompleted, PendingManualStep};
use crate::services::workflows::{
    WorkflowDone, WorkflowFailed, WorkflowStepAwaitingApproval, WorkflowStepDone,
    WorkflowStepStarted, WorkflowsChanged,
};

pub struct TauriSink(pub AppHandle);

impl EventSink for TauriSink {
    fn emit_json(&self, name: &'static str, payload: Value) {
        if let Err(e) = self.0.emit(name, payload) {
            log::warn!("Failed to emit {name}: {e}");
        }
    }
}

macro_rules! tauri_events {
    ($($ty:ident),* $(,)?) => {
        $(impl tauri_specta::Event for $ty {
            const NAME: &'static str = <$ty as NandiEvent>::NAME;
        })*

        /// Every event name the webview can receive
        #[cfg(test)]
        const EVENT_NAMES: &[&str] = &[$(<$ty as NandiEvent>::NAME),*];
    };
}

tauri_events!(
    AgentRunUpdated,
    AgentMessageAdded,
    AgentToken,
    AgentToolCall,
    AgentToolResult,
    AgentApprovalRequest,
    AgentApprovalResolved,
    AgentTurnDone,
    AgentsChanged,
    AgentWarning,
    AgentQueueChanged,
    DashboardAttention,
    DashboardActivity,
    WorkflowStepStarted,
    WorkflowStepAwaitingApproval,
    WorkflowStepDone,
    WorkflowDone,
    WorkflowFailed,
    WorkflowsChanged,
    TriggerFired,
    TriggerRateLimited,
    TriggerError,
    TriggersChanged,
    RepoValidationFailed,
    RepoFileChanged,
    RepoRebuildStarted,
    RepoRebuildCompleted,
    MarketplaceInstalled,
    MarketplaceUninstalled,
    MarketplaceForked,
    MarketplaceUpdateAvailable,
    MarketplaceUpdateApplied,
    MarketplaceError,
    MarketplaceProgress,
    PilotChanged,
    PilotConsentNeeded,
    ConnectorChanged,
    ConnectorsChanged,
    EmailChanged,
    RelayStatus,
    RelayAgentOnline,
    RelayAgentOffline,
    RelaySealRefused,
    MemberKeyChanged,
    TeamChanged,
    DelegationTaskStarted,
    DelegationTaskCompleted,
    DelegationTaskFailed,
    RequestReturned,
    // Wave final
    AgentTurnAborted,
    AgentRunCompacted,
    AgentToolsLoaded,
    // Wave CD
    AskStreamingDelta,
    AskCompleted,
    StudioArtifactLoaded,
    StudioArtifactSaved,
    StudioEditPreviewed,
    WorkItemUpdated,
    WorkItemAdded,
    WorkItemRemoved,
    // Wave EF
    TaskReassigned,
    PendingManualStep,
    ManualStepCompleted,
    RepoLlmConfigStatus,
    // Task 14
    RequestUpdated,
    // Live UI
    LiveChange,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_name_is_one_tauri_accepts() {
        for name in EVENT_NAMES {
            assert!(crate::services::events::is_valid_event_name(name), "{name}");
        }
    }
}
