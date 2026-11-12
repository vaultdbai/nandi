//! Marketplace search command (`services::marketplace::search`).

use crate::services::marketplace::search;
use crate::services::marketplace::search::MarketplaceSearchHit;

super::commands! {
    /// Ranked catalog matches (installed and available)
    fn search_marketplace(state, query: String) -> Vec<MarketplaceSearchHit> => search::search(&state, &query).await;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::services::AppState;

    #[tokio::test]
    async fn a_blank_query_answers_no_hits() {
        let state = AppState::for_tests(vec![]);
        let hits =
            crate::rpc::call(state.clone(), "search_marketplace", json!({ "query": " " })).await;
        assert_eq!(hits, Some(Ok(json!([]))));
        state.shutdown();
    }
}
