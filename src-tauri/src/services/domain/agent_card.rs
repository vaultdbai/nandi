//! Agent cards: what other Nandi instances see of an agent that serves
//! capabilities (wave 2A, Track A). A card is derived from the agent's
//! skill.md; it is never stored in the gold copy.
//!
//! `id` is `<instance_id>/<agent_id>`, unique across instances.

pub use nandi_protocol::{AgentCard, OwnerRef};

use super::Agent;

/// The card for a local agent. Owner, signature, and issue time are left
/// for the caller (`with_owner`, `with_issued_at`); load starts at zero.
pub fn card_from_agent(agent: &Agent, instance_id: &str, endpoint: &str) -> AgentCard {
    let capabilities = agent.capabilities();
    AgentCard {
        id: AgentCard::card_id(instance_id, agent.id().as_str()),
        name: agent.name().to_string(),
        description: agent.description().to_string(),
        instance_id: instance_id.to_string(),
        endpoint: endpoint.to_string(),
        capabilities: capabilities.serves().to_vec(),
        accepts_delegation: capabilities.accepts_delegation(),
        max_concurrent: capabilities.delegation_limit(),
        current_load: 0,
        owner: OwnerRef::default(),
        signature: None,
        issued_at: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::AgentId;

    #[test]
    fn cards_carry_what_the_agent_serves() {
        let agent = Agent::from_skill_md(
            AgentId::parse("papers").unwrap(),
            "---\nname: Papers\nserves: [{name: find-papers}]\naccepts_delegation: true\n---\n",
        )
        .unwrap();
        let card = card_from_agent(&agent, "inst-1", "wss://relay/a/inst-1/papers");
        assert_eq!(card.id, "inst-1/papers");
        assert_eq!(card.agent_id(), "papers");
        assert!(card.serves("find-papers") && !card.serves("other"));
        assert!(card.accepts_delegation);
        assert_eq!(card.max_concurrent, 1);
    }
}
