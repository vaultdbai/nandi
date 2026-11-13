//! Global run commands (Tauri adapter for `services::runs`).

use crate::services::runs;
use crate::services::runs::{DateRange, RunFilters, RunPage, RunStats};

super::commands! {
    /// Runs across every agent, newest first
    fn list_all_runs(state, filters: RunFilters, limit: u32, offset: u32) -> RunPage => runs::list_all_runs(state.db(), filters, limit, offset).await;

    fn get_run_stats(state, range: DateRange) -> RunStats => runs::get_run_stats(state.db(), range).await;
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::services::AppState;

    async fn call(name: &str, args: Value) -> Result<Value, String> {
        let state = AppState::for_tests(vec![]);
        let result = crate::rpc::call(state.clone(), name, args).await.unwrap();
        state.shutdown();
        result
    }

    #[tokio::test]
    async fn list_all_runs_takes_filters_and_paging() {
        let filters = json!({
            "status": "failed", "sourceType": "trigger", "agentId": "writer", "search": "x",
            "dateFrom": 0, "dateTo": 1_700_000_000_000_i64,
        });
        let page = json!({ "filters": filters, "limit": 5, "offset": 0 });
        assert_eq!(
            call("list_all_runs", page).await,
            Ok(json!({ "rows": [], "total": 0 }))
        );

        let bogus = json!({ "filters": { "status": "bogus" }, "limit": 5, "offset": 0 });
        assert!(call("list_all_runs", bogus)
            .await
            .unwrap_err()
            .contains("bogus"));
    }

    #[tokio::test]
    async fn get_run_stats_checks_the_range() {
        let stats = call(
            "get_run_stats",
            json!({ "range": { "from": 0, "to": 1_700_000_000_000_i64 } }),
        )
        .await
        .unwrap();
        assert_eq!(stats["totalRuns"], 0);

        let backwards = json!({ "range": { "from": 2, "to": 1 } });
        assert!(call("get_run_stats", backwards).await.is_err());
    }
}
