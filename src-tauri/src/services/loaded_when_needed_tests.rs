//! Each agent loads only when it is needed: other agents see its `name`
//! and `description` (and the capabilities it serves), its `agent.md` body
//! and `instructions.md` load when it runs, and its playbooks when they
//! are relevant. These tests hold it for what Nandi reads.

use serde_json::json;

use crate::services::ask::{context, AskContext};
use crate::services::builtin_agents::{self, DEFAULT_AGENT};
use crate::services::domain::{Agent, AgentId, EntityKind};
use crate::services::repos::Repository;
use crate::services::{a2a, playbooks, AppState};

const BODY: &str = "WRITER-BODY-ONLY-WHEN-IT-RUNS";
const RULES: &str = "WRITER-RULES-ONLY-WHEN-IT-RUNS";
const PLAYBOOK: &str = "WRITER-PLAYBOOK-ONLY-WHEN-RELEVANT";

/// Nandi, and a writer whose body, rules and playbook each carry a marker
fn state_with_a_writer() -> AppState {
    let state = AppState::for_tests(vec![]);
    let store = state.store();
    builtin_agents::seed(store.layout()).unwrap();
    store.commit(EntityKind::Agent, DEFAULT_AGENT).unwrap();
    let writer = Agent::from_files(
        AgentId::parse("writer").unwrap(),
        &format!(
            "---\nname: Writer\ndescription: Drafts posts in the owner's voice\n\
             serves: [{{name: draft-post, description: Drafts a post}}]\n\
             accepts_delegation: true\n---\n\n{BODY}\n"
        ),
        Some(RULES),
    )
    .unwrap();
    store.repos().agents.save(&writer).unwrap();
    let playbook = store.layout().agent_dir("writer").join("skills/tone.md");
    std::fs::create_dir_all(playbook.parent().unwrap()).unwrap();
    std::fs::write(
        &playbook,
        format!("---\nname: tone\ndescription: How the owner sounds\nwhen: Writing anything\n---\n\n{PLAYBOOK}\n"),
    )
    .unwrap();
    store.commit(EntityKind::Agent, "writer").unwrap();
    state
}

fn assert_carries_no_body(what: &str, text: &str) {
    for marker in [BODY, RULES, PLAYBOOK] {
        assert!(!text.contains(marker), "{what} carries {marker}:\n{text}");
    }
}

#[tokio::test]
async fn nandis_prompt_names_other_agents_and_never_carries_their_bodies() {
    let state = state_with_a_writer();
    let store = state.store();

    // Nandi's own part of the prompt: its agent.md body and rules only
    let nandi = store.skill(DEFAULT_AGENT).unwrap();
    assert!(!nandi.instructions.is_empty());
    assert_carries_no_body("Nandi's agent.md and rules", &nandi.instructions);
    // Nandi's playbooks: an index of its own, bodies on demand
    let section = playbooks::prompt_section(&playbooks::load(store, DEFAULT_AGENT), true);
    assert_carries_no_body("Nandi's playbook index", &section);

    // What Nandi reads about the others: name and description
    let others = context::describe(&context::load(&state, &AskContext::default()).await);
    assert!(others.contains("Writer"), "{others}");
    assert!(
        others.contains("Drafts posts in the owner's voice"),
        "{others}"
    );
    assert_carries_no_body("the agents Nandi is told about", &others);
    let providers = a2a::list_available_providers(&state, "").await.unwrap();
    let cards = json!(providers).to_string();
    assert!(
        cards.contains("Drafts posts in the owner's voice"),
        "{cards}"
    );
    assert_carries_no_body("the providers Nandi can delegate to", &cards);
    state.shutdown();
}

#[tokio::test]
async fn an_agents_body_loads_when_it_runs_and_its_playbooks_on_demand() {
    let state = state_with_a_writer();
    let store = state.store();

    let writer = store.skill("writer").unwrap();
    assert!(
        writer.instructions.contains(BODY),
        "{}",
        writer.instructions
    );
    assert!(
        writer.instructions.contains(RULES),
        "{}",
        writer.instructions
    );
    let playbooks = playbooks::load(store, "writer");
    let index = playbooks::prompt_section(&playbooks, false);
    assert!(index.contains("tone"), "{index}");
    assert!(!index.contains(PLAYBOOK), "only the index: {index}");
    let read = playbooks::call(
        store,
        "writer",
        playbooks::specs()[1].name.as_str(),
        &json!({ "name": "tone" }),
    );
    assert!(
        format!("{read:?}").contains(PLAYBOOK),
        "the body loads when asked: {read:?}"
    );
    state.shutdown();
}
