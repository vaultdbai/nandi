//! User preferences. Portable ones (theme, language) live in the repo's
//! `.nandi/settings.yaml`, so they follow the user's repo to other machines;
//! `<app data dir>/preferences.json` keeps a copy for the startup path that
//! runs before the repo loads.

use std::path::{Path, PathBuf};

use crate::services::AppState;
use crate::types::{validate_theme, AppPreferences};

/// Gets the path to the preferences file.
fn get_preferences_path(app_data_dir: &Path) -> Result<PathBuf, String> {
    // Ensure the directory exists
    std::fs::create_dir_all(app_data_dir)
        .map_err(|e| format!("Failed to create app data directory: {e}"))?;

    Ok(app_data_dir.join("preferences.json"))
}

/// Loads user preferences from disk.
/// Returns default preferences if the file doesn't exist.
pub async fn load_preferences(state: &AppState) -> Result<AppPreferences, String> {
    log::debug!("Loading preferences from disk");
    let prefs_path = get_preferences_path(state.app_data_dir())?;

    if !prefs_path.exists() {
        log::info!("Preferences file not found, using defaults");
        return Ok(with_portable_settings(state, AppPreferences::default()));
    }

    let contents = std::fs::read_to_string(&prefs_path).map_err(|e| {
        log::error!("Failed to read preferences file: {e}");
        format!("Failed to read preferences file: {e}")
    })?;

    let preferences: AppPreferences = serde_json::from_str(&contents).map_err(|e| {
        log::error!("Failed to parse preferences JSON: {e}");
        format!("Failed to parse preferences: {e}")
    })?;

    log::info!("Successfully loaded preferences");
    Ok(with_portable_settings(state, preferences))
}

/// Overlays theme, palette, and language from settings.yaml (the gold copy).
fn with_portable_settings(state: &AppState, mut preferences: AppPreferences) -> AppPreferences {
    if !state.store().layout().settings_path().is_file() {
        return preferences;
    }
    state.store().read(|repo| {
        preferences.theme = repo.settings().theme().as_str().to_string();
        preferences.palette = repo.settings().palette().map(str::to_string);
        if let Some(language) = repo.settings().language() {
            preferences.language = Some(language.to_string());
        }
    });
    preferences
}

/// Writes changed theme/palette/language to settings.yaml (staged) and commits them
/// to the aggregate and projection (`repo://settings.updated`).
async fn save_portable_settings(
    state: &AppState,
    preferences: &AppPreferences,
) -> Result<(), String> {
    use crate::services::domain::settings::Theme;
    use crate::services::domain::EntityKind;
    use crate::services::repos::SingletonRepository;

    let mut settings = state.store().read(|repo| repo.settings().clone());
    let before = settings.clone();
    settings.set_theme(Theme::parse(&preferences.theme)?);
    settings.set_palette(preferences.palette.clone())?;
    settings.set_language(preferences.language.clone())?;
    if settings == before && state.store().layout().settings_path().is_file() {
        return Ok(());
    }
    let store = state.store().clone();
    tokio::task::spawn_blocking(move || {
        store.repos().settings.save(&settings)?;
        store.commit(EntityKind::Settings, "settings")?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("Settings write failed: {e}"))?
}

/// Saves user preferences to disk.
/// Uses atomic write (temp file + rename) to prevent corruption.
pub async fn save_preferences(state: &AppState, preferences: AppPreferences) -> Result<(), String> {
    // Validate theme value
    validate_theme(&preferences.theme)?;
    save_portable_settings(state, &preferences).await?;

    log::debug!("Saving preferences to disk: {preferences:?}");
    let prefs_path = get_preferences_path(state.app_data_dir())?;

    let json_content = serde_json::to_string_pretty(&preferences).map_err(|e| {
        log::error!("Failed to serialize preferences: {e}");
        format!("Failed to serialize preferences: {e}")
    })?;

    // Write to a temporary file first, then rename (atomic operation)
    let temp_path = prefs_path.with_extension("tmp");

    std::fs::write(&temp_path, json_content).map_err(|e| {
        log::error!("Failed to write preferences file: {e}");
        format!("Failed to write preferences file: {e}")
    })?;

    if let Err(rename_err) = std::fs::rename(&temp_path, &prefs_path) {
        log::error!("Failed to finalize preferences file: {rename_err}");
        // Clean up the temp file to avoid leaving orphaned files on disk
        if let Err(remove_err) = std::fs::remove_file(&temp_path) {
            log::warn!("Failed to remove temp file after rename failure: {remove_err}");
        }
        return Err(format!("Failed to finalize preferences file: {rename_err}"));
    }

    log::info!("Successfully saved preferences to {prefs_path:?}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::RecordingSink;
    use std::sync::Arc;

    #[tokio::test]
    async fn theme_and_language_live_in_settings_yaml() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let preferences = AppPreferences {
            theme: "dark".into(),
            palette: Some("aurora".into()),
            language: Some("fr".into()),
        };
        save_preferences(&state, preferences.clone()).await.unwrap();

        let yaml = std::fs::read_to_string(state.store().layout().settings_path()).unwrap();
        assert!(
            yaml.contains("theme: dark")
                && yaml.contains("palette: aurora")
                && yaml.contains("language: fr"),
            "{yaml}"
        );
        assert!(recorder.names().contains(&"repo://settings.updated"));
        let loaded = load_preferences(&state).await.unwrap();
        assert_eq!(
            (loaded.theme.as_str(), loaded.language.as_deref()),
            ("dark", Some("fr"))
        );

        // An edit to settings.yaml (e.g. pulled from another machine) wins
        std::fs::write(
            state.store().layout().settings_path(),
            "theme: light
",
        )
        .unwrap();
        state.store().reload(
            crate::services::domain::EntityKind::Settings,
            "settings",
            crate::services::domain::ChangedBy::Git,
        );
        let loaded = load_preferences(&state).await.unwrap();
        assert_eq!(loaded.theme, "light");
        state.shutdown();
    }
}
