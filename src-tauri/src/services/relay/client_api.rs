//! The client side of the relay: what the app needs from a relay
//! connection (`RelayClient`), whatever carries it. The wire messages are
//! in `nandi_protocol` (re-exported by `protocol`).

use futures_util::future::BoxFuture;

use super::protocol::{
    CapabilityPresence, DutyDelivery, DutyState, InstancePresence, MemberRecord, PeerDelivery,
    PeerMessage, PeopleMatch, ResealRequest, RoutePlan, RoutingQuery, TaskAssigned, TaskCompleted,
    TaskEvent, TaskFailed, TaskRefused,
};
use super::RelayStatus;
use crate::services::a2a::{Task, TaskResult};
use crate::services::domain::AgentCard;
use crate::services::gateway::GatewayCard;
use nandi_protocol::{GroupChange, GroupOp, GroupOutcome, PublicKeys};

pub type Handler<T> = Box<dyn Fn(T) + Send + Sync>;

/// A handler that takes a delivery over. Its future answers once the
/// delivery is safe here (stored, or known already): only then does the
/// client ack it. `false` leaves it at the relay, to come again.
pub type StoringHandler<T> = Box<dyn Fn(T) -> BoxFuture<'static, bool> + Send + Sync>;

/// A [`StoringHandler`] for a test that only watches deliveries: each is
/// safe as soon as it's seen
#[cfg(test)]
pub fn stored_at_once<T>(watch: impl Fn(T) + Send + Sync + 'static) -> StoringHandler<T> {
    Box::new(move |delivery| {
        watch(delivery);
        Box::pin(async { true })
    })
}

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
/// with, before the relay's words
pub const REFUSED_BY_RELAY: &str = "The relay refused it: ";

/// What a submission that went out answers with when the relay's answer
/// didn't come (it timed out, or the connection dropped): the relay may
/// have it, so it's sent again, never given up. Any other error means it
/// didn't leave.
pub const UNANSWERED_BY_RELAY: &str = "The relay hasn't answered: ";

/// The relay took a submission
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submitted {
    /// Its provider is offline: it waits at the relay for them
    pub held: bool,
}

/// What the relay tells us about groups (protocol 7)
#[derive(Debug, Clone, PartialEq)]
pub enum GroupSignal {
    /// We (re)registered: anything may have changed while we were away,
    /// so list again
    Registered,
    /// A group we're in (or were invited to) changed
    Changed {
        group_id: String,
        change: GroupChange,
    },
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

    #[cfg(test)]
    fn heartbeat(
        &self,
        load: std::collections::HashMap<String, u32>,
    ) -> BoxFuture<'_, Result<(), String>>;

    /// `CapabilityResult` for `capability`, or an error after 5s
    fn query_capability<'a>(
        &'a self,
        capability: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AgentCard>, String>>;

    /// Sends a signed `SubmitTask`: `task` for its target, then (should
    /// the relay fail it over) each of `alternates` (card IDs) in turn, its
    /// input sealed for each. Answers once the relay took it (held for an
    /// offline provider counts), with the relay's refusal
    /// ([`REFUSED_BY_RELAY`]), or with no answer ([`UNANSWERED_BY_RELAY`]).
    fn delegate_task<'a>(
        &'a self,
        task: &'a Task,
        alternates: &'a [String],
    ) -> BoxFuture<'a, Result<Submitted, String>>;

    /// Provider side: sends a finished task's result back, once
    /// registered. `Ok` means only that it went out: the relay answers a
    /// result it won't take with a refusal ([`Self::on_task_refused`]).
    fn send_result(&self, result: TaskResult) -> BoxFuture<'_, Result<(), String>>;

    /// Provider side: tasks for us. One is acked once the handler stored
    /// it.
    fn on_task_assigned(&self, handler: StoringHandler<TaskAssigned>) -> BoxFuture<'_, ()>;

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

    /// The relay refused something we sent about a task
    fn on_task_refused(&self, handler: Handler<TaskRefused>) -> BoxFuture<'_, ()> {
        let _ = handler;
        Box::pin(async {})
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

    // Groups (protocol 7)

    /// One group call (`GroupRequest` → `GroupReply`), or an error after
    /// 5s. Not connected: `RelayUnavailable`. A refusal is an `Ok`
    /// (`GroupOutcome::Refused`) for the caller to explain.
    fn group_request(&self, op: GroupOp) -> BoxFuture<'_, Result<GroupOutcome, String>> {
        let _ = op;
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }

    /// Group news: after every registration, and every `GroupChanged`
    fn on_groups(&self, handler: Handler<GroupSignal>) -> BoxFuture<'_, ()> {
        let _ = handler;
        Box::pin(async {})
    }

    /// `instance`'s keys: the pinned ones, else the relay's (pinned now).
    /// `Err` while their key changed and the user hasn't accepted it.
    fn public_keys_of<'a>(
        &'a self,
        instance: &'a str,
    ) -> BoxFuture<'a, Result<PublicKeys, String>> {
        let _ = instance;
        Box::pin(async { Err("Not supported by this relay client".to_string()) })
    }
}
