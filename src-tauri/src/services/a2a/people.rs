//! People tools for an agent that works for its owner on a team (the
//! first-party "Chief of Staff" agent), read on the host and never gated:
//!
//! - `find_people` `{topic?, limit?}`: who on the user's network is
//!   responsible for a topic. The relay searches and ranks (`FindPeople`),
//!   from what each person publishes on their gateway card
//!   (`responsibilities`, description, capabilities, the NANDI.md body
//!   when they share it) and their online agents' cards. Without a
//!   connected relay it answers `RelayUnavailable`: nothing is matched
//!   from a cached copy.
//! - `my_requests` `{direction?, open_only?, limit?}`: what the owner asked
//!   others and what others asked the owner, from the A2A task log and
//!   the gateway's held requests.
//!
//! Offered only to agents whose skill.md lists them in `allowed-tools`,
//! so an agent never gets the user's network by default.

use std::collections::HashMap;

use rusqlite::params;
use serde_json::{json, Value};

use super::task::{self, TaskRecord};
use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::model::Timestamp;
use crate::services::domain::AgentCard;
use crate::services::gateway::{AvailabilityStatus, GATEWAY_AGENT_ID};
use crate::services::network;
use crate::services::relay::protocol::PeopleMatch;
use crate::services::relay::unavailable;
use crate::services::repo_store::RepoStore;
use crate::services::AppState;

pub const FIND_PEOPLE: &str = "find_people";
pub const MY_REQUESTS: &str = "my_requests";

const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 50;
const SUMMARY_CHARS: usize = 300;

pub fn is_people_tool(name: &str) -> bool {
    name == FIND_PEOPLE || name == MY_REQUESTS
}

/// The people tools `agent_id`'s skill.md names in `allowed-tools`
pub fn offered(store: &RepoStore, agent_id: &str) -> Vec<&'static str> {
    let Ok(agent) = store.agent(agent_id) else {
        return Vec::new();
    };
    let allowed = agent.capabilities().tools().allowed().unwrap_or_default();
    [FIND_PEOPLE, MY_REQUESTS]
        .into_iter()
        .filter(|tool| allowed.iter().any(|t| t == tool))
        .collect()
}

pub fn spec(name: &str) -> Option<ToolSpec> {
    let (description, schema) = match name {
        FIND_PEOPLE => (
            "Find people on the user's Nandi network who are responsible for something \
(e.g. Risk, Support, SOX audit, SOC 1, a system), live from what each person publishes: their \
stated responsibilities, profile, and what their agents do. Leave `topic` empty to list everyone. \
Each result says what matched, whether they're online, and the target and capability to ask \
them with `delegate_to_agent`, and their agents with what each serves (send a task straight to \
one with its `target` and one of its capabilities): the ones that match the topic, or all of \
them when listing everyone. `agents_online` counts all their agents online, matching or not.",
            json!({
                "type": "object",
                "properties": {
                    "topic": {"type": "string", "description": "What they're responsible for, in a few words; empty lists everyone"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT},
                },
            }),
        ),
        MY_REQUESTS => (
            "List the requests between the user and other people: what the user (or their \
agents) asked others, and what others asked the user, with status, who, when, and the answer \
when there is one. Requests waiting for the user's approval are included, and work assigned (`assign-me`) shows its due date, flagged `overdue` once it's past.",
            json!({
                "type": "object",
                "properties": {
                    "direction": {"type": "string", "enum": ["asked_by_me", "asked_of_me", "all"], "description": "default all"},
                    "open_only": {"type": "boolean", "description": "default true: only requests not finished yet"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT},
                },
            }),
        ),
        _ => return None,
    };
    Some(ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: schema,
    })
}

pub async fn call(
    state: &AppState,
    store: &RepoStore,
    agent_id: &str,
    tool: &str,
    args: &Value,
) -> ToolOutcome {
    if !offered(store, agent_id).contains(&tool) {
        return error(format!(
            "Tool '{tool}' is not available: add it to this agent's allowed-tools"
        ));
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_LIMIT, |l| (l as usize).clamp(1, MAX_LIMIT));
    let result = match tool {
        FIND_PEOPLE => find_people(state, args, limit).await,
        _ => my_requests(state, args, limit).await,
    };
    match result {
        Ok(value) => ToolOutcome {
            result: value.to_string(),
            is_error: false,
        },
        Err(e) => error(e),
    }
}

fn error(message: String) -> ToolOutcome {
    ToolOutcome {
        result: message,
        is_error: true,
    }
}

// ---------------------------------------------------------------------------
// find_people
// ---------------------------------------------------------------------------

async fn find_people(state: &AppState, args: &Value, limit: usize) -> Result<Value, String> {
    let topic = args
        .get("topic")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let relay = unavailable::connected(state.relay_client())?;
    let limit = u32::try_from(limit).unwrap_or(u32::MAX);
    let found = relay.find_people(&topic, limit).await?;
    let local = network::local_instance_id(state.db()).unwrap_or_default();
    let mut names: HashMap<String, String> = network::known_members(state.db())
        .into_iter()
        .map(|m| (m.instance_id, m.display_name))
        .collect();
    // Whoever covers may be among the people found (the directory cache
    // may not have them yet)
    for person in &found.matches {
        names
            .entry(person.instance_id.clone())
            .or_insert_with(|| person.name.clone());
    }
    let people: Vec<Value> = found
        .matches
        .iter()
        .map(|person| person_json(person, &local, &names))
        .collect();
    Ok(json!({
        "topic": topic,
        "source": "relay (live)",
        "people_searched": found.searched,
        "people_without_stated_responsibilities": found.unstated,
        "matches": found.total,
        "people": people,
    }))
}

/// One person the relay found, as the tool shows them. `names` (instance
/// ID → name) names whoever covers for someone away; "you" when it's the
/// user (an instance ID would read as one of the person's own agents).
pub fn person_json(person: &PeopleMatch, local: &str, names: &HashMap<String, String>) -> Value {
    let mut entry = json!({
        "name": person.name,
        "handle": person.handle,
        "you": person.instance_id == local,
        "online": person.online,
        "last_seen": person.last_seen_ms.map(format_time),
        "takes_requests": person.takes_requests,
        "ask": person.takes_requests.then(|| json!({
            "target": AgentCard::card_id(&person.instance_id, GATEWAY_AGENT_ID),
            "capability": person.ask_capability,
        })),
        "responsibilities": person.responsibilities,
        "description": person.description,
        "matched_on": person.matched_on,
        "excerpt": person.excerpt,
        "agents_online": person.agents_online,
        "agents": person.agents.iter().map(|a| json!({
            "target": a.target,
            "name": a.name,
            "description": a.description,
            "serves": a.serves.iter().map(|c| json!({"capability": c.capability, "description": c.description})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let availability = &person.availability;
    if !availability.is_available() {
        let status = match availability.status {
            AvailabilityStatus::Busy => "busy",
            _ => "away",
        };
        entry["availability"] = json!({
            "status": status,
            "until": availability.until.map(format_time),
            "covered_by": availability.delegate_to.as_ref().map(|id| {
                if id == local {
                    "you".to_string()
                } else {
                    names.get(id).cloned().unwrap_or_else(|| id.clone())
                }
            }),
        });
    }
    entry
}

// ---------------------------------------------------------------------------
// my_requests
// ---------------------------------------------------------------------------

async fn my_requests(state: &AppState, args: &Value, limit: usize) -> Result<Value, String> {
    let direction = args
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("all");
    if !["asked_by_me", "asked_of_me", "all"].contains(&direction) {
        return Err(format!(
            "direction '{direction}' is unknown: use asked_by_me, asked_of_me or all"
        ));
    }
    let open_only = args
        .get("open_only")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let local = network::local_instance_id(state.db())?;
    let names: HashMap<String, String> = network::known_members(state.db())
        .into_iter()
        .map(|m| (m.instance_id, m.display_name))
        .collect();
    let conn = state.db().conn()?;
    let fetch = (limit * 4) as i64;
    let tasks = task::select(
        &conn,
        "requester_instance = ?1 OR provider_instance = ?1 ORDER BY updated_at DESC, id LIMIT ?2",
        &[&local, &fetch],
    )
    .map_err(|e| format!("Database error: {e}"))?;
    let holds = pending_holds(&conn)?;
    let mut requests = requests_from(&tasks, &holds, &local, &names, direction, open_only, limit);
    // Work the user assigned themselves goes through the local gateway with
    // no task row: it's only in the open assignments
    let assignments = crate::services::gateway::assignments::open_list(&conn)
        .map_err(|e| format!("Database error: {e}"))?;
    requests.extend(self_assigned(&assignments, &tasks, &local, direction));
    requests.truncate(limit);
    Ok(json!({ "direction": direction, "open_only": open_only, "requests": requests }))
}

/// Open assignments the user gave themselves (the caller is this Nandi
/// and there's no task row), as `my_requests` lists them
pub fn self_assigned(
    assignments: &[crate::services::gateway::assignments::GatewayAssignment],
    tasks: &[TaskRecord],
    local: &str,
    direction: &str,
) -> Vec<Value> {
    if direction == "asked_by_me" {
        return Vec::new();
    }
    assignments
        .iter()
        .filter(|a| a.caller_instance == local && !tasks.iter().any(|t| t.id == a.task_id))
        .map(|a| {
            let mut entry = json!({
                "id": a.task_id,
                "direction": "asked_of_me",
                "from": "you",
                "capability": super::assignment::ASSIGN_ME,
                "request": cut(&a.request, SUMMARY_CHARS),
                "status": "on your list",
                "created": format_time(a.created_at),
            });
            if let Some(due) = a.due_at {
                entry["due"] = json!(format_time(due));
                if due <= crate::db::now_ms() as Timestamp {
                    entry["overdue"] = json!(true);
                }
            }
            entry
        })
        .collect()
}

/// A request someone sent to the user's gateway that waits for the user
#[derive(Debug, Clone, PartialEq)]
pub struct Hold {
    pub task_id: String,
    pub caller_name: String,
    pub capability: Option<String>,
    pub summary: String,
    pub created_at: Timestamp,
}

fn pending_holds(conn: &rusqlite::Connection) -> Result<Vec<Hold>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT task_id, caller_name, capability, summary, created_at FROM gateway_holds
              WHERE status = 'pending' ORDER BY created_at DESC",
        )
        .map_err(|e| format!("Database error: {e}"))?;
    let rows = stmt
        .query_map(params![], |row| {
            Ok(Hold {
                task_id: row.get(0)?,
                caller_name: row.get(1)?,
                capability: row.get(2)?,
                summary: row.get(3)?,
                created_at: row.get::<_, i64>(4)? as Timestamp,
            })
        })
        .map_err(|e| format!("Database error: {e}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| format!("Database error: {e}"))
}

fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// The request in words: `input.text` when there is one, else the JSON
fn summarize(value: &Value) -> String {
    match value.get("text").and_then(Value::as_str) {
        Some(text) => cut(text, SUMMARY_CHARS),
        None => match value {
            Value::Null => String::new(),
            Value::String(s) => cut(s, SUMMARY_CHARS),
            other => cut(&other.to_string(), SUMMARY_CHARS),
        },
    }
}

/// The requests to show, newest first: held requests (waiting for the
/// user) first when asked-of-me requests are wanted
pub fn requests_from(
    tasks: &[TaskRecord],
    holds: &[Hold],
    local: &str,
    names: &HashMap<String, String>,
    direction: &str,
    open_only: bool,
    limit: usize,
) -> Vec<Value> {
    let name = |instance: &str| {
        if instance == local {
            "you".to_string()
        } else {
            names
                .get(instance)
                .cloned()
                .unwrap_or_else(|| instance.to_string())
        }
    };
    let mut out = Vec::new();
    if direction != "asked_by_me" {
        for hold in holds {
            out.push(json!({
                "id": hold.task_id,
                "direction": "asked_of_me",
                "from": hold.caller_name,
                "capability": hold.capability,
                "request": cut(&hold.summary, SUMMARY_CHARS),
                "status": "waiting for your approval",
                "created": format_time(hold.created_at),
            }));
        }
    }
    let held: Vec<&str> = holds.iter().map(|h| h.task_id.as_str()).collect();
    for record in tasks {
        let asked_by_me = record.requester_instance == local;
        let wanted = match direction {
            "asked_by_me" => asked_by_me,
            "asked_of_me" => !asked_by_me,
            _ => true,
        };
        if !wanted
            || (open_only && record.state.is_terminal())
            || held.contains(&record.id.as_str())
        {
            continue;
        }
        let mut entry = json!({
            "id": record.id,
            "direction": if asked_by_me { "asked_by_me" } else { "asked_of_me" },
            "capability": record.capability,
            "request": summarize(&record.input),
            "status": record.state.as_str(),
            "created": format_time(record.created_at),
            "updated": format_time(record.updated_at),
        });
        if asked_by_me {
            entry["to"] = json!(name(&record.provider_instance));
            entry["agent"] = json!(record.provider_agent);
        } else {
            entry["from"] = json!(name(&record.requester_instance));
        }
        // An assignment open on someone's list (theirs, or yours)
        let open = !record.state.is_terminal();
        if open && super::assignment::is_lasting(&record.capability) {
            entry["status"] = json!(if asked_by_me {
                "assigned, on their list"
            } else {
                "on your list"
            });
        }
        if let Some(deadline) = record.deadline {
            entry["due"] = json!(format_time(deadline));
            if open && deadline <= crate::db::now_ms() as Timestamp {
                entry["overdue"] = json!(true);
            }
        }
        if let Some(result) = &record.result {
            entry["answer"] = json!(summarize(result));
        }
        if let Some(error) = &record.error {
            entry["error"] = json!(cut(error, SUMMARY_CHARS));
        }
        out.push(entry);
    }
    out.truncate(limit);
    out
}

/// Epoch ms as local time, e.g. `2026-10-24 14:05`
fn format_time(ms: Timestamp) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::gateway::Availability;
    use crate::services::relay::protocol::ASK_ME;
    use nandi_protocol::{MatchedAgent, ServedCapability};

    fn found(instance: &str) -> PeopleMatch {
        PeopleMatch {
            instance_id: instance.into(),
            name: "Sam".into(),
            handle: Some("sam".into()),
            online: true,
            last_seen_ms: Some(1_700_000_000_000.0),
            takes_requests: true,
            ask_capability: ASK_ME.into(),
            responsibilities: vec!["security incidents".into()],
            description: "Security".into(),
            matched_on: vec!["agents".into()],
            excerpt: None,
            agents: vec![MatchedAgent {
                target: format!("{instance}/chief-of-staff"),
                name: "Chief of Staff".into(),
                description: String::new(),
                serves: vec![ServedCapability {
                    capability: ASK_ME.into(),
                    description: "Questions about Sam's work".into(),
                }],
            }],
            agents_online: 1,
            availability: Availability::available(),
        }
    }

    #[test]
    fn a_match_says_how_to_ask_and_what_their_agents_serve() {
        let json = person_json(&found("i-sam"), "i-me", &HashMap::new());
        assert_eq!(json["you"], false);
        assert_eq!(json["ask"]["target"], "i-sam/gateway");
        assert_eq!(json["ask"]["capability"], ASK_ME);
        assert_eq!(json["agents"][0]["target"], "i-sam/chief-of-staff");
        assert_eq!(json["agents"][0]["serves"][0]["capability"], ASK_ME);
        assert_eq!(json["agents_online"], 1);
        assert!(json.get("availability").is_none());
        assert_eq!(
            person_json(&found("i-me"), "i-me", &HashMap::new())["you"],
            true
        );
    }

    #[test]
    fn someone_away_says_until_when_and_who_covers() {
        let mut away = found("i-sam");
        away.availability = Availability {
            status: AvailabilityStatus::Away,
            until: Some(1_800_000_000_000.0),
            delegate_to: Some("i-lena".into()),
        };
        let names = HashMap::from([("i-lena".to_string(), "Lena".to_string())]);
        let json = person_json(&away, "i-me", &names);
        assert_eq!(json["availability"]["status"], "away");
        assert_eq!(json["availability"]["covered_by"], "Lena");
        assert!(json["availability"]["until"].as_str().is_some());
        // Covered by the user asking: "you", not their instance ID
        let covered_by_me = PeopleMatch {
            availability: Availability {
                delegate_to: Some("i-me".into()),
                ..away.availability.clone()
            },
            ..away.clone()
        };
        let json = person_json(&covered_by_me, "i-me", &names);
        assert_eq!(json["availability"]["covered_by"], "you");
    }

    fn save_agent(state: &AppState, id: &str, frontmatter: &str) {
        use crate::services::domain::{Agent, AgentId, EntityKind};
        use crate::services::repos::Repository;
        let agent = Agent::from_skill_md(
            AgentId::parse(id).unwrap(),
            &format!(
                "---
name: {id}
{frontmatter}---
Help the user.
"
            ),
        )
        .unwrap();
        state.store().repos().agents.save(&agent).unwrap();
        state.store().commit(EntityKind::Agent, id).unwrap();
    }

    #[tokio::test]
    async fn only_agents_that_list_them_get_the_people_tools() {
        let state = AppState::for_tests(vec![]);
        save_agent(
            &state,
            "chief-of-staff",
            "allowed-tools: [read_file, find_people, my_requests]
",
        );
        save_agent(&state, "writer", "");
        save_agent(
            &state,
            "reader",
            "allowed-tools: [read_file]
",
        );
        assert_eq!(
            offered(state.store(), "chief-of-staff"),
            [FIND_PEOPLE, MY_REQUESTS]
        );
        // Omitting allowed-tools grants the runtime tools, never the network
        assert!(offered(state.store(), "writer").is_empty());
        assert!(offered(state.store(), "reader").is_empty());

        let refused = call(&state, state.store(), "writer", FIND_PEOPLE, &json!({})).await;
        assert!(refused.is_error && refused.result.contains("allowed-tools"));
        // No relay: "not connected", never a guess from a cached copy
        let offline = call(
            &state,
            state.store(),
            "chief-of-staff",
            FIND_PEOPLE,
            &json!({"topic": "SOX"}),
        )
        .await;
        assert!(offline.is_error);
        assert!(
            crate::services::relay::RelayUnavailable::is(&offline.result),
            "{}",
            offline.result
        );
        let requests = call(
            &state,
            state.store(),
            "chief-of-staff",
            MY_REQUESTS,
            &json!({"direction": "sideways"}),
        )
        .await;
        assert!(requests.is_error && requests.result.contains("unknown"));
        let requests = call(
            &state,
            state.store(),
            "chief-of-staff",
            MY_REQUESTS,
            &json!({}),
        )
        .await;
        assert!(!requests.is_error, "{}", requests.result);
        state.shutdown();
    }

    fn record(
        id: &str,
        requester: &str,
        provider: &str,
        state: task::TaskState,
        input: Value,
    ) -> TaskRecord {
        let mut record = TaskRecord::requester(
            id,
            requester,
            "chief-of-staff",
            &format!("{provider}/gateway"),
            ASK_ME,
            &input,
            None,
        );
        record.state = state;
        record
    }

    #[test]
    fn requests_split_by_direction_and_hide_finished_ones() {
        let names = HashMap::from([("i-lena".to_string(), "Lena".to_string())]);
        let mut done = record(
            "t3",
            "i-me",
            "i-lena",
            task::TaskState::Completed,
            json!({"text": "Old"}),
        );
        done.result = Some(json!({"text": "Here it is"}));
        // Assigned to Lena, due long ago
        let mut assigned = record(
            "t1",
            "i-me",
            "i-lena",
            task::TaskState::Working,
            json!({"text": "Send the SOC 1 evidence"}),
        );
        assigned.capability = "assign-me".into();
        assigned.deadline = Some(1_700_000_000_000.0);
        let tasks = vec![
            assigned,
            record(
                "t2",
                "i-lena",
                "i-me",
                task::TaskState::Submitted,
                json!({"text": "Who approves deploys?"}),
            ),
            done,
        ];
        let hold = Hold {
            task_id: "t4".into(),
            caller_name: "Sam".into(),
            capability: Some("assign-me".into()),
            summary: "Please rotate the keys by Friday".into(),
            created_at: 1_700_000_200_000.0,
        };
        let open = requests_from(
            &tasks,
            std::slice::from_ref(&hold),
            "i-me",
            &names,
            "all",
            true,
            10,
        );
        let ids: Vec<&str> = open.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["t4", "t1", "t2"]);
        assert_eq!(open[0]["status"], "waiting for your approval");
        assert_eq!(open[1]["to"], "Lena");
        assert_eq!(open[1]["request"], "Send the SOC 1 evidence");
        assert_eq!(open[1]["status"], "assigned, on their list");
        assert_eq!(open[1]["overdue"], true);
        assert_eq!(open[2]["from"], "Lena");
        assert!(open[2].get("overdue").is_none());

        let mine = requests_from(&tasks, &[hold], "i-me", &names, "asked_by_me", false, 10);
        let ids: Vec<&str> = mine.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["t1", "t3"]);
        assert_eq!(mine[1]["answer"], "Here it is");
    }

    #[test]
    fn work_you_assign_yourself_is_on_your_list() {
        use crate::services::gateway::assignments::{AssignmentStatus, GatewayAssignment};
        let assignment = |task_id: &str, caller: &str| GatewayAssignment {
            task_id: task_id.into(),
            caller_instance: caller.into(),
            caller_name: "Dev".into(),
            request: "Rotate the CI deploy keys".into(),
            status: AssignmentStatus::Open,
            note: None,
            created_at: 1.0,
            due_at: Some(2.0),
            closed_at: None,
            links: Vec::new(),
        };
        // Yours (no task row), and one from Lena (has a task row elsewhere)
        let list = [assignment("g1", "i-me"), assignment("g2", "i-lena")];
        let mine = self_assigned(&list, &[], "i-me", "all");
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0]["id"], "g1");
        assert_eq!(mine[0]["from"], "you");
        assert_eq!(mine[0]["status"], "on your list");
        assert_eq!(mine[0]["overdue"], true);
        assert!(self_assigned(&list, &[], "i-me", "asked_by_me").is_empty());
    }
}
