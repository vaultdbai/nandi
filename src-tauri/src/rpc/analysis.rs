//! Run analysis commands (`services::run_analysis`).

use crate::services::run_analysis;
use crate::services::run_analysis::AnalysisResponse;

super::commands! {
    /// Answers `question` from the run's own record, citing sources
    fn analyze_run(state, run_id: String, question: String) -> AnalysisResponse => run_analysis::analyze_run(&state, &run_id, &question).await;

    /// A short markdown summary of the run (cached)
    fn summarize_run(state, run_id: String) -> String => run_analysis::summarize_run(&state, &run_id).await;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::services::AppState;

    #[tokio::test]
    async fn unknown_runs_are_errors() {
        let state = AppState::for_tests(vec![]);
        for (name, args) in [
            (
                "analyze_run",
                json!({ "runId": "nope", "question": "Why?" }),
            ),
            ("summarize_run", json!({ "runId": "nope" })),
        ] {
            let result = crate::rpc::call(state.clone(), name, args).await.unwrap();
            assert!(result.unwrap_err().contains("not found"), "{name}");
        }
        state.shutdown();
    }
}
