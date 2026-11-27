//! @people and @groups in a chat: `@Sam can you review the Q4 numbers?`
//! sends the existing ask to Sam's gateway (`ask-me`, the same request a
//! workflow's ask step sends), and `@Finance …` sends it to each member of
//! the group but the person. The chat keeps the person's message with the
//! asks on it ([`PersonAsk`]): the chat shows each as a card whose status
//! and answer are its delegated task's. Nothing goes to an agent.
//!
//! People are on the relay: without a connected one this answers the typed
//! `RelayUnavailable`, and the `@` list shows no people in the first place.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{self, role, MessageRow, UserMeta};
use crate::model::{ChatMessage, PersonAsk};
use crate::services::a2a::assignment::ASK_ME;
use crate::services::domain::AgentCard;
use crate::services::gateway::GATEWAY_AGENT_ID;
use crate::services::groups::Groups;
use crate::services::network;
use crate::services::relay::unavailable;
use crate::services::AppState;

/// Who an ask from a chat goes to
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AskTarget {
    /// One person, by their Nandi's instance ID
    Person {
        #[serde(rename = "instanceId")]
        instance_id: String,
    },
    /// Every member of a group but the person
    Group {
        #[serde(rename = "groupId")]
        group_id: String,
    },
}

/// One person an ask goes to
#[derive(Debug, Clone, PartialEq)]
pub struct Recipient {
    pub instance_id: String,
    pub who: String,
}

/// A person by instance ID, named as the network knows them (`members`:
/// instance ID and name)
pub fn person(members: &[(String, String)], instance_id: &str) -> Recipient {
    let who = members
        .iter()
        .find(|(instance, _)| instance == instance_id)
        .map(|(_, name)| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| instance_id.to_string());
    Recipient {
        instance_id: instance_id.to_string(),
        who,
    }
}

/// A group's members as recipients, the person (`me`) left out
pub fn members_but_me(members: &[(String, String)], me: &str) -> Vec<Recipient> {
    members
        .iter()
        .filter(|(instance, _)| instance != me)
        .map(|(instance, name)| Recipient {
            instance_id: instance.clone(),
            who: if name.trim().is_empty() {
                instance.clone()
            } else {
                name.trim().to_string()
            },
        })
        .collect()
}

/// Sends `text` from the chat `conversation_id` to `target` as asks, and
/// keeps the person's message in the chat with the asks on it
pub async fn ask_in_chat(
    state: &AppState,
    conversation_id: String,
    target: AskTarget,
    text: String,
) -> Result<ChatMessage, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Type what to ask".to_string());
    }
    let id = conversation_id.clone();
    state
        .db()
        .call(move |conn| Ok(crate::services::chat::chat_agent(conn, &id)))
        .await??;
    unavailable::connected(state.relay_client())?;
    let me = network::local_instance_id(state.db())?;
    let (recipients, group) = match &target {
        AskTarget::Person { instance_id } => {
            if *instance_id == me {
                return Err("That's you: ask someone else".to_string());
            }
            let known: Vec<(String, String)> = network::known_members(state.db())
                .into_iter()
                .map(|m| (m.instance_id, m.display_name))
                .collect();
            (vec![person(&known, instance_id)], None)
        }
        AskTarget::Group { group_id } => {
            let view = Groups::of(state)?.get(group_id).await?;
            let members: Vec<(String, String)> = view
                .group
                .members
                .iter()
                .map(|m| (m.instance_id.clone(), m.display_name.clone()))
                .collect();
            (
                members_but_me(&members, &me),
                Some(view.group.summary.name.clone()),
            )
        }
    };
    if recipients.is_empty() {
        return Err("Nobody else is in that group yet".to_string());
    }
    let mut asked = Vec::new();
    for recipient in recipients {
        let card = AgentCard::card_id(&recipient.instance_id, GATEWAY_AGENT_ID);
        let task_id = crate::services::a2a::delegate_task(
            state,
            ASK_ME,
            serde_json::json!({ "text": text }),
            Some(&card),
        )
        .await?;
        asked.push(PersonAsk {
            task_id,
            who: recipient.who,
            instance_id: recipient.instance_id,
            group: group.clone(),
        });
    }
    let row = MessageRow::new(
        uuid::Uuid::new_v4().to_string(),
        &conversation_id,
        None,
        role::USER,
        text,
        UserMeta {
            attachments: Vec::new(),
            asked,
        },
    );
    let message = row.to_chat();
    state
        .db()
        .call(move |conn| db::insert_message(conn, &row))
        .await?;
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_person_is_named_as_the_network_knows_them() {
        let members = [("inst-sam".to_string(), "Sam Lee".to_string())];
        assert_eq!(person(&members, "inst-sam").who, "Sam Lee");
        assert_eq!(person(&members, "inst-new").who, "inst-new");
    }

    #[test]
    fn a_group_ask_goes_to_every_member_but_me() {
        let members = vec![
            ("me".to_string(), "Ada".to_string()),
            ("inst-sam".to_string(), "Sam".to_string()),
            ("inst-kim".to_string(), " ".to_string()),
        ];
        let to = members_but_me(&members, "me");
        let who: Vec<&str> = to.iter().map(|r| r.who.as_str()).collect();
        assert_eq!(who, ["Sam", "inst-kim"]);
    }

    #[tokio::test]
    async fn without_a_relay_it_says_so_and_keeps_nothing() {
        let state = AppState::for_tests(vec![]);
        let chat = state
            .db()
            .call(|conn| {
                let chat = db::chat_conversation_id("c1");
                db::insert_chat_conversation(
                    conn,
                    crate::services::builtin_agents::DEFAULT_AGENT,
                    &chat,
                )?;
                Ok(chat)
            })
            .await
            .unwrap();
        let target = AskTarget::Person {
            instance_id: "inst-sam".into(),
        };
        let error = ask_in_chat(&state, chat.clone(), target.clone(), "Review Q4?".into())
            .await
            .unwrap_err();
        assert!(
            crate::services::relay::unavailable::RelayUnavailable::is(&error),
            "{error}"
        );
        let rows = state
            .db()
            .call(move |conn| db::list_messages(conn, &chat, 10))
            .await
            .unwrap();
        assert!(rows.is_empty());
        // Not one of the person's chats: refused before the relay
        let error = ask_in_chat(&state, "task:t1".into(), target, "x".into())
            .await
            .unwrap_err();
        assert!(error.contains("not found"), "{error}");
        state.shutdown();
    }

    #[test]
    fn a_message_with_asks_reads_back_with_them_and_tells_the_model() {
        let row = MessageRow::new(
            "m1",
            "chat:c1",
            None,
            role::USER,
            "Review Q4?",
            UserMeta {
                attachments: Vec::new(),
                asked: vec![PersonAsk {
                    task_id: "t1".into(),
                    who: "Sam".into(),
                    instance_id: "inst-sam".into(),
                    group: None,
                }],
            },
        );
        let ChatMessage::User { asked, .. } = row.to_chat() else {
            panic!("not a user message");
        };
        assert_eq!(asked[0].who, "Sam");
        let history = crate::services::history::build(&[row], &Default::default());
        let crate::llm::Message::User { content } = &history[0] else {
            panic!("{history:?}");
        };
        assert!(
            content.contains("sent this to Sam as a request"),
            "{content}"
        );
    }
}
