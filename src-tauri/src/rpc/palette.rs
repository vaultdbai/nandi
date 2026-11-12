//! Command palette commands (`services::palette`).

use crate::services::palette;
use crate::services::palette::{PaletteActionOutcome, PaletteResult};
use serde_json::Value;

super::commands! {
    /// Ranked results across agents, workflows, runs, packages, tools, and
    /// actions; `>`-prefixed queries search actions only
    fn search_palette(state, query: String) -> Vec<PaletteResult> => palette::search_palette(&state, &query).await;

    /// Runs a built-in action by ID with optional arguments
    fn execute_palette_action(state, action_id: String, args: Value) -> PaletteActionOutcome => palette::execute_palette_action(&state, &action_id, args).await;
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
    async fn searches_actions() {
        let body = call("search_palette", json!({ "query": ">home" }))
            .await
            .unwrap();
        let results = body.as_array().unwrap();
        assert_eq!(results[0]["name"], "Go to Home");
        assert!(results.iter().all(|r| r["kind"] == "action"));
    }

    #[tokio::test]
    async fn executes_ui_actions() {
        let args = json!({ "actionId": "open_view", "args": { "view": { "kind": "network", "tab": "people" } } });
        let body = call("execute_palette_action", args).await.unwrap();
        assert_eq!(body["navigate"]["kind"], "network");
    }

    #[tokio::test]
    async fn unknown_actions_and_bad_arguments_are_errors() {
        let unknown = call(
            "execute_palette_action",
            json!({ "actionId": "nope", "args": {} }),
        )
        .await;
        assert!(unknown.unwrap_err().contains("Unknown"));
        let bad = call(
            "execute_palette_action",
            json!({ "actionId": "open_view", "args": "x" }),
        )
        .await;
        assert!(bad.unwrap_err().contains("Invalid"));
    }
}
