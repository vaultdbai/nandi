//! Gateway commands (Tauri adapter for `services::gateway`).

use crate::services::gateway;
use crate::services::gateway::{
    CallerPolicy, GatewayConfig, GatewayManifest, GatewayStatus, GatewayUsage,
};

super::commands! {
    fn get_gateway_status(state) -> GatewayStatus => gateway::get_gateway_status(&state).await;

    /// Requests waiting for the owner's approval, oldest first (the Inbox)
    fn list_gateway_holds(state) -> Vec<gateway::holds::GatewayHold> => gateway::list_gateway_holds(&state).await;

    /// Approve (routing goes on) or decline (it's rejected) a held request;
    /// `note` goes back to the requester (a refusal's reason, a condition)
    fn decide_gateway_hold(state, task_id: String, approve: bool, note: Option<String>) -> gateway::holds::GatewayHold => gateway::decide_gateway_hold(&state, task_id, approve, note).await;

    /// Work assigned to the owner that's still open, soonest due first (the
    /// Inbox)
    fn list_gateway_assignments(state) -> Vec<gateway::assignments::GatewayAssignment> => gateway::list_gateway_assignments(&state).await;

    /// Mark an assignment Done (the requester gets the note) or Decline it
    fn close_gateway_assignment(state, task_id: String, done: bool, note: Option<String>) -> gateway::assignments::GatewayAssignment => gateway::close_gateway_assignment(&state, task_id, done, note).await;

    fn make_gateway_reachable(state) -> GatewayStatus => gateway::make_gateway_reachable(&state).await;

    fn get_gateway_config(state) -> GatewayConfig => gateway::get_gateway_config(&state).await;

    fn set_gateway_config(state, config: GatewayConfig) -> GatewayConfig => gateway::set_gateway_config(&state, config).await;

    fn list_gateway_callers(state) -> Vec<CallerPolicy> => gateway::list_gateway_callers(&state).await;

    fn set_caller_policy(state, policy: CallerPolicy) -> CallerPolicy => gateway::set_caller_policy(&state, policy).await;

    fn remove_caller_policy(state, caller_instance: String) -> () => gateway::remove_caller_policy(&state, &caller_instance).await;

    fn get_gateway_usage(state, caller_instance: String) -> GatewayUsage => gateway::get_gateway_usage(&state, &caller_instance).await;

    fn get_gateway_manifest(state) -> GatewayManifest => gateway::get_gateway_manifest(&state).await;

    fn set_gateway_manifest(state, manifest: GatewayManifest) -> GatewayManifest => gateway::set_gateway_manifest(&state, manifest).await;

    /// Ask the gateway about this instance (capabilities, queue, who to ask).
    /// It answers; it never runs work.
    fn gateway_chat(state, message: String) -> String => gateway::gateway_chat(&state, &message).await;
}
