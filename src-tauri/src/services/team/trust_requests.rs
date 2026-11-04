//! Trust requests (task 12): people another Nandi's request was refused for
//! because this Nandi doesn't trust them yet. Nothing new is stored for
//! them: the provider row of every refused task is already kept, stepped
//! to `rejected` with `reason_kind = notTrusted` (task 5). A request is
//! pending while the person still isn't trusted, paused or blocked, the
//! owner didn't deny them outright, and nothing was refused since the owner
//! last said "not now" (a dismissal time in `local_settings`).
//!
//! **Trust** marks them a trusted peer, so their next request goes through
//! without asking. **Deny** dismisses what's there (and can block them).
//! There's no "trust once": a refused task isn't held, so there's nothing
//! to resume.

use std::collections::BTreeMap;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{self, now_ms, Db};
use crate::model::Timestamp;
use crate::services::events::{emit, NandiEvent};
use crate::services::trust::{self as policies, TrustPolicy};
use crate::services::{network, AppState};

use super::trust::{self, TrustUpdate, ALL_AGENTS};

/// `trust://request_received`: someone was refused for trust, or their
/// request was answered; the payload names who
pub const TRUST_REQUEST_RECEIVED: &str = "trust://request_received";

/// `local_settings` key: `{instance_id: dismissed_at_ms}`
const DISMISSED_KEY: &str = "trust_requests.dismissed";

/// One person waiting to be trusted, with what they last asked for
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrustRequest {
    pub instance_id: String,
    /// Their display name, when the directory knows them
    pub member_name: Option<String>,
    pub handle: Option<String>,
    /// The agent of ours they asked last
    pub agent_id: String,
    pub agent_name: String,
    pub capability: String,
    /// Refusals since the owner last answered
    pub attempts: u32,
    pub first_at: Timestamp,
    pub last_at: Timestamp,
    /// The latest refused task (its Request page)
    pub last_task_id: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrustRequestReceived {
    pub instance_id: String,
}

impl NandiEvent for TrustRequestReceived {
    const NAME: &'static str = TRUST_REQUEST_RECEIVED;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TrustAnswer {
    /// Trust them from now on (a trusted peer)
    Trust,
    /// Not now: hide what's there; a new refusal brings them back
    Deny,
}

/// A refused task's provider row, as read back
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub task_id: String,
    pub requester_instance: String,
    /// The requester's card ID (`<instance>/<agent>`)
    pub requester_agent: String,
    /// Our agent's card ID or agent ID
    pub provider_agent: String,
    pub capability: String,
    pub at: i64,
}

/// What decides whether a refusal still waits on the owner
pub trait Standing {
    fn is_trusted(&self, instance_id: &str) -> bool;
    fn is_paused_or_blocked(&self, instance_id: &str, requester_agent: &str) -> bool;
    /// The owner set "never" for this requester on this agent
    fn is_denied(&self, requester_agent: &str, agent_id: &str) -> bool;
    fn dismissed_at(&self, instance_id: &str) -> Option<i64>;
    fn member(&self, instance_id: &str) -> Option<(String, Option<String>)>;
    fn agent_name(&self, agent_id: &str) -> String;
}

/// Our agent's ID from `provider_agent` (a card ID or a bare ID)
fn local_agent_id(provider_agent: &str) -> &str {
    provider_agent
        .rsplit_once('/')
        .map_or(provider_agent, |(_, id)| id)
}

/// One request per person, newest first: refusals (any order) folded under
/// what still stands
pub fn pending_requests(refusals: &[Refusal], standing: &impl Standing) -> Vec<TrustRequest> {
    let mut by_member: BTreeMap<&str, Vec<&Refusal>> = BTreeMap::new();
    for refusal in refusals {
        by_member
            .entry(refusal.requester_instance.as_str())
            .or_default()
            .push(refusal);
    }
    let mut requests: Vec<TrustRequest> = by_member
        .into_iter()
        .filter(|(instance, _)| !standing.is_trusted(instance))
        .filter_map(|(instance, mut refusals)| {
            let since = standing.dismissed_at(instance).unwrap_or(i64::MIN);
            refusals.retain(|r| {
                r.at > since
                    && !standing.is_paused_or_blocked(instance, &r.requester_agent)
                    && !standing.is_denied(&r.requester_agent, local_agent_id(&r.provider_agent))
            });
            refusals.sort_by_key(|r| r.at);
            let (first, last) = (refusals.first()?, refusals.last()?);
            let agent_id = local_agent_id(&last.provider_agent).to_string();
            let member = standing.member(instance);
            Some(TrustRequest {
                instance_id: instance.to_string(),
                member_name: member.as_ref().map(|(name, _)| name.clone()),
                handle: member.and_then(|(_, handle)| handle),
                agent_name: standing.agent_name(&agent_id),
                agent_id,
                capability: last.capability.clone(),
                attempts: refusals.len() as u32,
                first_at: first.at as Timestamp,
                last_at: last.at as Timestamp,
                last_task_id: last.task_id.clone(),
            })
        })
        .collect();
    requests.sort_by(|a, b| b.last_at.total_cmp(&a.last_at));
    requests
}

/// Every task refused for trust that another Nandi sent this one
pub fn refusals(conn: &Connection, local_instance: &str) -> rusqlite::Result<Vec<Refusal>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.requester_instance, t.requester_agent, t.provider_agent,
                t.capability, e.at
           FROM tasks t JOIN task_events e ON e.task_id = t.id
          WHERE t.state = 'rejected' AND e.to_state = 'rejected'
            AND e.reason_kind = 'notTrusted'
            AND t.provider_instance = ?1 AND t.requester_instance != ?1",
    )?;
    let rows = stmt.query_map(params![local_instance], |row| {
        Ok(Refusal {
            task_id: row.get(0)?,
            requester_instance: row.get(1)?,
            requester_agent: row.get(2)?,
            provider_agent: row.get(3)?,
            capability: row.get(4)?,
            at: row.get(5)?,
        })
    })?;
    rows.collect()
}

fn dismissals(conn: &Connection) -> rusqlite::Result<BTreeMap<String, i64>> {
    Ok(db::get_setting(conn, DISMISSED_KEY)?
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default())
}

fn save_dismissals(conn: &Connection, dismissed: &BTreeMap<String, i64>) -> rusqlite::Result<()> {
    let value = serde_json::to_string(dismissed).unwrap_or_else(|_| "{}".to_string());
    conn.execute(
        "INSERT INTO local_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![DISMISSED_KEY, value],
    )?;
    Ok(())
}

/// The live answers to `Standing`: trust rows, policies, the directory
struct Live<'a> {
    db: &'a Db,
    state: &'a AppState,
    dismissed: BTreeMap<String, i64>,
    members: Vec<network::NetworkMember>,
}

impl Standing for Live<'_> {
    fn is_trusted(&self, instance_id: &str) -> bool {
        trust::is_trusted(self.db, instance_id)
    }

    fn is_paused_or_blocked(&self, instance_id: &str, requester_agent: &str) -> bool {
        trust::is_paused(self.db, instance_id)
            || trust::is_blocked(self.db, instance_id, requester_agent)
            || trust::is_blocked(self.db, instance_id, ALL_AGENTS)
    }

    fn is_denied(&self, requester_agent: &str, agent_id: &str) -> bool {
        policies::policy_for(self.db, requester_agent, agent_id)
            .is_ok_and(|policy| policy == TrustPolicy::Deny)
    }

    fn dismissed_at(&self, instance_id: &str) -> Option<i64> {
        self.dismissed.get(instance_id).copied()
    }

    fn member(&self, instance_id: &str) -> Option<(String, Option<String>)> {
        self.members
            .iter()
            .find(|m| m.instance_id == instance_id)
            .map(|m| (m.display_name.clone(), m.handle.clone()))
    }

    fn agent_name(&self, agent_id: &str) -> String {
        self.state.store().display_name(agent_id)
    }
}

/// Everyone waiting to be trusted, newest first
pub async fn list_trust_requests(state: &AppState) -> Result<Vec<TrustRequest>, String> {
    let local = network::local_instance_id(state.db())?;
    let (refused, dismissed) = state
        .db()
        .call(move |conn| Ok((refusals(conn, &local)?, dismissals(conn)?)))
        .await?;
    if refused.is_empty() {
        return Ok(Vec::new());
    }
    let live = Live {
        db: state.db(),
        state,
        dismissed,
        members: network::known_members(state.db()),
    };
    Ok(pending_requests(&refused, &live))
}

/// Trust: a trusted peer from now on. Deny: hide what's waiting, and with
/// `block`, refuse everything from them.
pub async fn respond_trust_request(
    state: &AppState,
    instance_id: &str,
    answer: TrustAnswer,
    block: bool,
) -> Result<(), String> {
    let instance_id = instance_id.trim();
    if instance_id.is_empty() {
        return Err("Choose a person".to_string());
    }
    match answer {
        TrustAnswer::Trust => {
            trust::set_trust_relationship(
                state,
                instance_id,
                TrustUpdate {
                    added_as_trusted_peer: Some(true),
                    delegations_paused: None,
                },
            )
            .await?;
        }
        TrustAnswer::Deny => {
            let id = instance_id.to_string();
            state
                .db()
                .call(move |conn| {
                    let mut dismissed = dismissals(conn)?;
                    dismissed.insert(id, now_ms());
                    save_dismissals(conn, &dismissed)
                })
                .await?;
            if block {
                let relationship = trust::set_blocked(state.db(), instance_id, ALL_AGENTS, true)?;
                emit(state.bus(), &relationship);
            }
        }
    }
    emit(
        state.bus(),
        &TrustRequestReceived {
            instance_id: instance_id.to_string(),
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    struct Fixed {
        trusted: HashSet<String>,
        paused: HashSet<String>,
        denied: HashSet<(String, String)>,
        dismissed: HashMap<String, i64>,
        names: HashMap<String, (String, Option<String>)>,
    }

    impl Standing for Fixed {
        fn is_trusted(&self, instance_id: &str) -> bool {
            self.trusted.contains(instance_id)
        }
        fn is_paused_or_blocked(&self, instance_id: &str, _: &str) -> bool {
            self.paused.contains(instance_id)
        }
        fn is_denied(&self, requester_agent: &str, agent_id: &str) -> bool {
            self.denied
                .contains(&(requester_agent.to_string(), agent_id.to_string()))
        }
        fn dismissed_at(&self, instance_id: &str) -> Option<i64> {
            self.dismissed.get(instance_id).copied()
        }
        fn member(&self, instance_id: &str) -> Option<(String, Option<String>)> {
            self.names.get(instance_id).cloned()
        }
        fn agent_name(&self, agent_id: &str) -> String {
            format!("Agent {agent_id}")
        }
    }

    fn refusal(task: &str, instance: &str, agent: &str, at: i64) -> Refusal {
        Refusal {
            task_id: task.into(),
            requester_instance: instance.into(),
            requester_agent: format!("{instance}/asker"),
            provider_agent: format!("me/{agent}"),
            capability: format!("{agent}-cap"),
            at,
        }
    }

    #[test]
    fn refusals_fold_into_one_request_per_person() {
        let mut standing = Fixed::default();
        standing
            .names
            .insert("sam".into(), ("Sam K.".into(), Some("sam".into())));
        let rows = [
            refusal("t2", "sam", "papers", 20),
            refusal("t1", "sam", "papers", 10),
            refusal("t3", "sam", "notes", 30),
            refusal("t4", "jo", "papers", 15),
        ];
        let requests = pending_requests(&rows, &standing);
        assert_eq!(requests.len(), 2);
        // Newest first
        let sam = &requests[0];
        assert_eq!(sam.instance_id, "sam");
        assert_eq!(sam.member_name.as_deref(), Some("Sam K."));
        assert_eq!(sam.handle.as_deref(), Some("sam"));
        assert_eq!(sam.attempts, 3);
        assert_eq!((sam.first_at, sam.last_at), (10.0, 30.0));
        // What they asked last
        assert_eq!(sam.agent_id, "notes");
        assert_eq!(sam.agent_name, "Agent notes");
        assert_eq!(sam.capability, "notes-cap");
        assert_eq!(sam.last_task_id, "t3");
        // Unknown to the directory: no name, never the device ID as one
        assert_eq!(requests[1].member_name, None);
    }

    #[test]
    fn trusted_people_are_not_waiting() {
        let mut standing = Fixed::default();
        standing.trusted.insert("sam".into());
        assert!(pending_requests(&[refusal("t", "sam", "a", 1)], &standing).is_empty());
    }

    #[test]
    fn paused_or_blocked_people_are_not_waiting() {
        let mut standing = Fixed::default();
        standing.paused.insert("sam".into());
        assert!(pending_requests(&[refusal("t", "sam", "a", 1)], &standing).is_empty());
    }

    #[test]
    fn an_explicit_never_is_not_a_request() {
        let mut standing = Fixed::default();
        standing
            .denied
            .insert(("sam/asker".into(), "papers".into()));
        let rows = [
            refusal("t1", "sam", "papers", 1),
            refusal("t2", "sam", "notes", 2),
        ];
        let requests = pending_requests(&rows, &standing);
        assert_eq!(requests.len(), 1);
        assert_eq!(
            (requests[0].attempts, requests[0].agent_id.as_str()),
            (1, "notes")
        );
    }

    #[test]
    fn not_now_hides_until_they_ask_again() {
        let mut standing = Fixed::default();
        standing.dismissed.insert("sam".into(), 20);
        assert!(pending_requests(&[refusal("t1", "sam", "a", 10)], &standing).is_empty());
        let again = pending_requests(
            &[refusal("t1", "sam", "a", 10), refusal("t2", "sam", "a", 30)],
            &standing,
        );
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].attempts, 1, "only what came after the answer");
    }

    #[test]
    fn agent_ids_come_from_card_ids_or_bare_ids() {
        assert_eq!(local_agent_id("desktop-1/papers"), "papers");
        assert_eq!(local_agent_id("papers"), "papers");
    }

    fn seed(conn: &Connection, id: &str, requester: &str, state: &str, kind: &str, at: i64) {
        conn.execute(
            "INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance,
                                provider_agent, capability, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'me', 'me/papers', 'summarize', ?4, ?5, ?5)",
            params![id, requester, format!("{requester}/asker"), state, at],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO task_events (task_id, to_state, reason, reason_kind, at)
             VALUES (?1, ?2, 'refused', ?3, ?4)",
            params![id, state, kind, at],
        )
        .unwrap();
    }

    #[test]
    fn only_trust_refusals_of_incoming_tasks_are_read() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        seed(&conn, "t1", "sam", "rejected", "notTrusted", 10);
        seed(&conn, "t2", "sam", "rejected", "notAccepting", 11);
        seed(&conn, "t3", "jo", "completed", "notTrusted", 12);
        // Our own task to ourselves isn't a request from someone else
        seed(&conn, "t4", "me", "rejected", "notTrusted", 13);
        let rows = refusals(&conn, "me").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task_id, "t1");
        assert_eq!(rows[0].provider_agent, "me/papers");
    }

    #[tokio::test]
    async fn trust_answers_the_request_and_deny_hides_it() {
        use crate::services::events::tests::RecordingSink;
        let sink = std::sync::Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![sink.clone()]);
        let me = network::local_instance_id(state.db()).unwrap();
        {
            let conn = state.db().conn().unwrap();
            for (id, who, at) in [("t1", "sam", 10), ("t2", "jo", 20)] {
                conn.execute(
                    "INSERT INTO tasks (id, requester_instance, requester_agent,
                         provider_instance, provider_agent, capability, state,
                         created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'summarize', 'rejected', ?6, ?6)",
                    params![
                        id,
                        who,
                        format!("{who}/asker"),
                        me,
                        format!("{me}/papers"),
                        at
                    ],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO task_events (task_id, to_state, reason, reason_kind, at)
                     VALUES (?1, 'rejected', 'refused', 'notTrusted', ?2)",
                    params![id, at],
                )
                .unwrap();
            }
        }
        let waiting = list_trust_requests(&state).await.unwrap();
        assert_eq!(waiting.len(), 2);

        respond_trust_request(&state, "sam", TrustAnswer::Trust, false)
            .await
            .unwrap();
        assert!(trust::is_trusted_peer(state.db(), "sam"));
        respond_trust_request(&state, "jo", TrustAnswer::Deny, true)
            .await
            .unwrap();
        assert!(trust::is_instance_blocked(state.db(), "jo"));
        assert!(list_trust_requests(&state).await.unwrap().is_empty());
        assert!(sink.names().contains(&TRUST_REQUEST_RECEIVED));
        assert!(respond_trust_request(&state, " ", TrustAnswer::Deny, false)
            .await
            .is_err());
        state.shutdown();
    }
}
