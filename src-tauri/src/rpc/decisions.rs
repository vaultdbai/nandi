//! Decisions: what was decided, month by month.

use crate::services::domain::decision::{DecisionFilter, DecisionRecord};
use crate::services::inbox::decisions;

super::commands! {
    /// The months that have decisions (`2026-11`), newest first
    fn list_decision_months(state) -> Vec<String> => decisions::months(&state);

    /// One month's decisions, newest first, narrowed by the filter
    fn list_decisions(state, month: String, filter: Option<DecisionFilter>) -> Vec<DecisionRecord> => decisions::list(&state, month, filter).await;
}
