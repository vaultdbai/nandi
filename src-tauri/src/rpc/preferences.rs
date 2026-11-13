//! Preferences commands (Tauri adapter for `services::preferences`).

use crate::services::preferences;
use crate::types::AppPreferences;

super::commands! {
    fn load_preferences(state) -> AppPreferences => preferences::load_preferences(&state).await;

    fn save_preferences(state, preferences: AppPreferences) -> () => preferences::save_preferences(&state, preferences).await;
}
