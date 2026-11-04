//! The triggers behind S3 Backup, kept in step with its Setup: once a
//! bucket and folders are set (and the Backup Keeper agent is installed),
//! saving the connector's settings writes
//!
//! - `s3-backup-<folder>`: a file trigger on each folder, so a change is
//!   backed up within `RATE_LIMIT_SECONDS` (a burst of changes is one run:
//!   the rest are rate-limited, and the nightly run catches anything left)
//! - `s3-backup-nightly`: a cron trigger that backs up every folder
//!
//! and removes the `s3-backup-*` triggers of folders that are gone. They are
//! ordinary trigger files (`.nandi/triggers/`), so the user can see, pause
//! or edit them in Automations; a trigger the user paused stays paused.

use serde_json::json;

use super::s3_backup::BackupSettings;
use crate::services::triggers;
use crate::services::triggers::schema::{
    Trigger, TriggerInput, TriggerSourceType, TriggerTargetType,
};
use crate::services::AppState;

/// The pack agent that runs the backups
pub const AGENT: &str = "backup-keeper";
const PREFIX: &str = "s3-backup-";
const NIGHTLY: &str = "s3-backup-nightly";
const NIGHTLY_SCHEDULE: &str = "0 2 * * *";
/// At most one backup per folder this often while it keeps changing
pub const RATE_LIMIT_SECONDS: u32 = 15 * 60;
/// Wait for a file to settle before it counts as changed
const DEBOUNCE_MS: u64 = 10_000;

/// The triggers `settings` call for (none without a bucket and folders)
pub fn desired(settings: &BackupSettings) -> Vec<TriggerInput> {
    if settings.bucket.is_none() || settings.folders.is_empty() {
        return Vec::new();
    }
    let mut wanted: Vec<TriggerInput> = settings
        .folders
        .iter()
        .map(|folder| TriggerInput {
            id: Some(format!("{PREFIX}{}", folder.id)),
            name: format!("Back up {} to S3", folder.id),
            enabled: Some(true),
            source_type: TriggerSourceType::File,
            source_config: json!({
                "path": folder.path,
                "recursive": true,
                "events": ["create", "modify"],
                "debounceMs": DEBOUNCE_MS,
            }),
            target_type: TriggerTargetType::Agent,
            target_id: AGENT.to_string(),
            input_template: Some(format!(
                "Files changed in the backup folder \"{id}\" (latest: {{{{payload.path}}}}). \
                 Back it up now with s3_backup__backup_folder (folder: \"{id}\") and reply \
                 with one line: how many files were uploaded, or the error.",
                id = folder.id
            )),
            priority: None,
            rate_limit_seconds: Some(RATE_LIMIT_SECONDS),
        })
        .collect();
    wanted.push(TriggerInput {
        id: Some(NIGHTLY.to_string()),
        name: "Nightly S3 backup".to_string(),
        enabled: Some(true),
        source_type: TriggerSourceType::Cron,
        source_config: json!({"schedule": NIGHTLY_SCHEDULE}),
        target_type: TriggerTargetType::Agent,
        target_id: AGENT.to_string(),
        input_template: Some(
            "Nightly backup: run s3_backup__backup_all and reply with one line per folder: \
             files uploaded, or the error."
                .to_string(),
        ),
        priority: None,
        rate_limit_seconds: None,
    });
    wanted
}

/// Writes the triggers `settings` call for and removes stale ones
pub async fn sync(state: &AppState, settings: &BackupSettings) -> Result<(), String> {
    let mut wanted = desired(settings);
    if !wanted.is_empty() {
        let agents = crate::services::agents::list_agents(state).await?;
        if !agents.iter().any(|agent| agent.id == AGENT) {
            log::info!(
                "S3 Backup is set up, but the Backup Keeper agent isn't installed: no triggers yet"
            );
            wanted.clear();
        }
    }
    let existing: Vec<Trigger> = triggers::list_triggers(state)
        .await?
        .into_iter()
        .filter(|t| t.id.starts_with(PREFIX))
        .collect();
    for mut input in wanted.iter().cloned() {
        let id = input.id.clone().unwrap_or_default();
        match existing.iter().find(|t| t.id == id) {
            Some(current) => {
                input.enabled = Some(current.enabled);
                triggers::update_trigger(state, id, input).await?;
            }
            None => {
                triggers::create_trigger(state, input).await?;
            }
        }
    }
    for stale in existing.iter().filter(|t| {
        !wanted
            .iter()
            .any(|w| w.id.as_deref() == Some(t.id.as_str()))
    }) {
        triggers::delete_trigger(state, stale.id.clone()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::connectors::folders::{Access, SharedFolder};

    fn settings(bucket: Option<&str>, folders: &[&str]) -> BackupSettings {
        BackupSettings {
            profile: None,
            upload_profile: "nandi-backup".into(),
            region: None,
            bucket: bucket.map(str::to_string),
            folders: folders
                .iter()
                .map(|id| SharedFolder {
                    id: (*id).to_string(),
                    path: format!("~/{id}"),
                    access: Access::Read,
                })
                .collect(),
        }
    }

    #[test]
    fn each_folder_gets_a_change_trigger_plus_one_nightly_run() {
        let wanted = desired(&settings(Some("b"), &["photos", "docs"]));
        let ids: Vec<_> = wanted.iter().filter_map(|t| t.id.as_deref()).collect();
        assert_eq!(
            ids,
            ["s3-backup-photos", "s3-backup-docs", "s3-backup-nightly"]
        );
        let photos = &wanted[0];
        assert_eq!(photos.source_type, TriggerSourceType::File);
        assert_eq!(photos.source_config["path"], "~/photos");
        assert_eq!(photos.source_config["recursive"], true);
        assert_eq!(photos.rate_limit_seconds, Some(RATE_LIMIT_SECONDS));
        let template = photos.input_template.as_deref().unwrap();
        assert!(template.contains("{{payload.path}}"), "{template}");
        assert!(template.contains("folder: \"photos\""), "{template}");
        assert_eq!(wanted[2].source_type, TriggerSourceType::Cron);
        assert!(wanted.iter().all(|t| t.target_id == AGENT));
    }

    #[test]
    fn nothing_until_there_is_a_bucket_and_a_folder() {
        assert!(desired(&settings(None, &["photos"])).is_empty());
        assert!(desired(&settings(Some("b"), &[])).is_empty());
    }
}
