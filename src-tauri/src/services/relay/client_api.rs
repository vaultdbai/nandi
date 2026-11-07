//! The client side of the relay: what the app needs from a relay
//! connection (`RelayClient`), whatever carries it. The wire messages are
//! in `nandi_protocol` (re-exported by `protocol`).

use std::collections::HashMap;

use futures_util::future::BoxFuture;

use super::protocol::{
    CapabilityPresence, DutyDelivery, DutyState, InstancePresence, MemberRecord, PeerDelivery,
    PeerMessage, PeopleMatch, ResealRequest, RoutePlan, RoutingQuery, TaskAssigned, TaskCompleted,
    TaskEvent, TaskFailed,
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

/// What a submission the relay refused (`TaskEvent::Refused`) answers
/// with, before the relay's words; any other error means it didn't get there
pub const REFUSED_BY_RELAY: &str = "The relay refused it: ";

/// The relay took a submission
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submitted {
    /// Its provider is offline: it waits at the relay for them
    pub held: bool,
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

    /// Sends a signed `SubmitTask`: `task` for its target, then (should
    /// the relay fail it over) each of `alternates` (card IDs) in turn, its
    /// input sealed for each. Answers once the relay took it (held for an
    /// offline provider counts) or with the relay's refusal.
    fn delegate_task<'a>(
        &'a self,
        task: &'a Task,
        alternates: &'a [String],
    ) -> BoxFuture<'a, Result<Submitted, String>>;

    /// Provider side: sends a finished task's result back
    fn send_result(&self, result: TaskResult) -> BoxFuture<'_, Result<(), String>>;

    fn on_task_assigned(&self, handler: Handler<TaskAssigned>) -> BoxFuture<'_, ()>;

    fn on_task_completed(&self, handler: Handler<TaskCompleted>) -> BoxFuture<'_, ()>;

    fn on_task_failed(&self, handler: Handler<TaskFailed>) -> BoxFuture<'_, ()>;

    /// What the relay did with a task we're a party to (failovers, moves,
    /// endings, and its answers to our submissions)
    fn on_task_event(&self, handler: Handler<TaskEvent>) -> BoxFuture<'_, ()> {
        let _ = handler;
        Box::pin(async {})
    }

    /// Provider side: hands `task_id` (asked for by `requester`) to the
    /// card `target` through the relay, with the request resealed for the
    /// new provider (when we still hold it) and `reason` sealed for the
    /// requester. Answers once the relay moved it, or why not.
    fn reassign<'a>(
        &'a self,
        task_id: &'a str,
        requester: &'a str,
        target: &'a str,
        by: &'a str,
        reason: Option<String>,
    ) -> BoxFuture<'a, Result<(), String>> {
        let _ = (task_id, requester, target, by, reason);
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// Requester side: seals our own `task` again for `target` (the relay
    /// asked: its provider no longer held the request)
    fn reseal_as_author<'a>(
        &'a self,
        task: &'a Task,
        target: &'a str,
        by: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        let _ = (task, target, by);
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// The relay asks us to reseal a task for a new provider
    fn on_reseal(&self, handler: Handler<ResealRequest>) -> BoxFuture<'_, ()> {
        let _ = handler;
        Box::pin(async {})
    }

    /// As a calendar trigger's backup: asks the relay to watch
    /// `primary_instance`'s `duty_id`, due at `due_at` (epoch ms)
    fn duty_watch<'a>(
        &'a self,
        duty_id: &'a str,
        primary_instance: &'a str,
        due_at: i64,
        after_minutes: Option<u32>,
    ) -> BoxFuture<'a, Result<(), String>> {
        let _ = (duty_id, primary_instance, due_at, after_minutes);
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// As a primary: how `duty_id` is going, for the relay (a failure's
    /// error sealed for `backup_instance`)
    fn duty_report<'a>(
        &'a self,
        duty_id: &'a str,
        backup_instance: &'a str,
        state: DutyState,
    ) -> BoxFuture<'a, Result<(), String>> {
        let _ = (duty_id, backup_instance, state);
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// Duty questions (as a primary) and decisions (as a backup)
    fn on_duty(&self, handler: Handler<DutyDelivery>) -> BoxFuture<'_, ()> {
        let _ = handler;
        Box::pin(async {})
    }

    fn status(&self) -> RelayStatus;

    // Wave final

    /// Sends a lifecycle message to the other side of a task through the
    /// relay, which keeps it for them if they're offline (live-only kinds
    /// go now or not at all). `Err` when we aren't connected.
    fn send_peer(
        &self,
        target_instance: String,
        body: PeerMessage,
    ) -> BoxFuture<'_, Result<(), String>> {
        let _ = (target_instance, body);
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// Every lifecycle message from the other side of a task
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
