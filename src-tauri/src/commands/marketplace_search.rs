//! Marketplace search command (Tauri adapter for `services::marketplace::search`).

use tauri::State;

pub use crate::services::marketplace::search::MarketplaceSearchHit;

use crate::services::marketplace::search;
use crate::services::AppState;

/// Ranked catalog matches (installed and available)
#[tauri::command]
#[specta::specta]
pub async fn search_marketplace(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<MarketplaceSearchHit>, String> {
    search::search(&state, &query).await
}
