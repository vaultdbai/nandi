//! `marketplace://` events, through the existing bus. Installs, uninstalls,
//! forks, and updates also change repo files, so the usual `repo://` and
//! list-reload events (`agent://agents_changed`, …) follow from the
//! re-scan; these say what the marketplace did.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::events::NandiEvent;

macro_rules! marketplace_event {
    ($ty:ident, $name:literal) => {
        impl NandiEvent for $ty {
            const NAME: &'static str = $name;
        }
    };
}

/// A pack or agent was installed.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceInstalled {
    /// pack | agent
    pub kind: String,
    pub id: String,
    pub source: String,
    pub version: String,
}
marketplace_event!(MarketplaceInstalled, "marketplace://installed");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceUninstalled {
    pub kind: String,
    pub id: String,
}
marketplace_event!(MarketplaceUninstalled, "marketplace://uninstalled");

/// A marketplace agent became the user's own.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceForked {
    pub source_id: String,
    pub new_id: String,
}
marketplace_event!(MarketplaceForked, "marketplace://forked");

/// Found by an explicit "Check for updates"; nothing is applied.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceUpdateAvailable {
    pub kind: String,
    pub id: String,
    pub current_version: String,
    pub latest_version: String,
}
marketplace_event!(MarketplaceUpdateAvailable, "marketplace://update_available");

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceUpdateApplied {
    pub kind: String,
    pub id: String,
    pub from_version: String,
    pub to_version: String,
}
marketplace_event!(MarketplaceUpdateApplied, "marketplace://update_applied");

/// A marketplace action failed.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceError {
    /// install | uninstall | fork | check_updates | update | registry
    pub action: String,
    pub id: String,
    pub error: String,
}
marketplace_event!(MarketplaceError, "marketplace://error");

/// Download progress of an install or update (throttled). Not recorded.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceProgress {
    pub kind: String,
    pub id: String,
    pub received_objects: u32,
    pub total_objects: u32,
}
marketplace_event!(MarketplaceProgress, "marketplace://progress");
