//! The client side of the relay: what the app needs from a relay
//! connection (`RelayClient`), whatever carries it. The wire messages are
//! in `nandi_protocol` (re-exported by `protocol`).

use std::collections::HashMap;

use futures_util::future::BoxFuture;

use super::protocol::{
    CapabilityPresence, InstancePresence, MemberRecord, PeerDelivery, PeerMessage, PeopleMatch,
    RoutePlan, RoutingQuery, TaskAssigned, TaskCompleted, TaskFailed,
};
use super::RelayStatus;
use crate::services::a2a::{Task, TaskResult};
use crate::services::domain::AgentCard;
use crate::services::gateway::GatewayCard;

pub type Handler<T> = Box<dyn Fn(T) + Send + Sync>;

/// The relay's answer to a people search
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PeopleSearch {
    /// Best first, at most the limit asked for
    pub matches: Vec<PeopleMatch>,
    /// Members searched
    pub searched: u32,
    /// Members searched who list no responsibilities
    pub unstated: u32,
    /// Matches before the limit
    pub total: u32,
}

/// One instance's connection to a relay.
pub trait RelayClient: Send + Sync {
    /// Opens the outbound WebSocket; reconnects with backoff after drops
    /// (and re-registers)
    fn connect<'a>(
        &'a self,
        url: &'a str,
        instance_id: &'a str,
    ) -> BoxFuture<'a, Result<(), String>>;

    /// Sends `RegisterInstance` and waits for `Registered`
    fn register(&self, cards: Vec<AgentCard>) -> BoxFuture<'_, Result<(), String>>;

    fn heartbeat(&self, load: HashMap<String, u32>) -> BoxFuture<'_, Result<(), String>>;

    /// `CapabilityResult` for `capability`, or an error after 5s
    fn query_capability<'a>(
        &'a self,
        capability: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AgentCard>, String>>;

    /// Sends a signed `DelegateTask`, its input sealed for the provider;
    /// returns the task ID
    fn delegate_task<'a>(&'a self, task: &'a Task) -> BoxFuture<'a, Result<String, String>>;

    /// Sends `task` (a reassigned task's next attempt) with the request
    /// its last provider resealed for the new one, the requester's
    /// signature kept (`ReassignTask.request`). Clients that don't seal
    /// send it as any task.
    fn delegate_resealed<'a>(
        &'a self,
        task: &'a Task,
        request: nandi_protocol::Envelope,
    ) -> BoxFuture<'a, Result<String, String>> {
        let _ = request;
        self.delegate_task(task)
    }

    /// Provider side: sends a finished task's result back
    fn send_result(&self, result: TaskResult) -> BoxFuture<'_, Result<(), String>>;

    fn on_task_assigned(&self, handler: Handler<TaskAssigned>) -> BoxFuture<'_, ()>;

    fn on_task_completed(&self, handler: Handler<TaskCompleted>) -> BoxFuture<'_, ()>;

    fn on_task_failed(&self, handler: Handler<TaskFailed>) -> BoxFuture<'_, ()>;

    fn status(&self) -> RelayStatus;

    // Wave final

    /// Sends a lifecycle message to another instance through the relay.
    /// `Err` only when we aren't connected; an offline target comes back as
    /// an undeliverable `PeerDelivery`.
    fn send_peer(
        &self,
        target_instance: String,
        body: PeerMessage,
    ) -> BoxFuture<'_, Result<(), String>> {
        let _ = (target_instance, body);
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// Every `Peer` / `Undeliverable` message that arrives
    fn on_peer(&self, handler: Handler<PeerDelivery>) -> BoxFuture<'_, ()> {
        let _ = handler;
        Box::pin(async {})
    }

    /// Registers (or replaces) our gateway card; re-sent on reconnect
    fn register_gateway(&self, card: GatewayCard) -> BoxFuture<'_, Result<(), String>> {
        let _ = card;
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// The relay's member directory (online or not), or an error after 5s
    fn query_directory(&self) -> BoxFuture<'_, Result<Vec<MemberRecord>, String>> {
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// Who the relay finds responsible for `topic` (empty: everyone), at
    /// most `limit`, or an error after 5s
    fn find_people<'a>(
        &'a self,
        topic: &'a str,
        limit: u32,
    ) -> BoxFuture<'a, Result<PeopleSearch, String>> {
        let _ = (topic, limit);
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// Who should get a network request: the relay's plan (`RouteQuery`),
    /// best first, or an error after 5s. Not connected: `RelayUnavailable`
    /// (the caller runs local providers without asking).
    fn route<'a>(&'a self, query: &'a RoutingQuery) -> BoxFuture<'a, Result<RoutePlan, String>> {
        let _ = query;
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    // Wave A

    /// Sets what this instance provides: carried by every later
    /// `RegisterInstance` (reconnects too) and sent now as `UpdatePresence`
    /// when registered
    fn set_presence(
        &self,
        capabilities: Vec<CapabilityPresence>,
        tools: Vec<String>,
    ) -> BoxFuture<'_, Result<(), String>> {
        let _ = (capabilities, tools);
        Box::pin(async { Ok(()) })
    }

    /// The relay's providers of `capability_id` (`*`: every capability),
    /// or an error after 5s
    fn query_capability_presence<'a>(
        &'a self,
        capability_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<InstancePresence>, String>> {
        let _ = capability_id;
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }
}
