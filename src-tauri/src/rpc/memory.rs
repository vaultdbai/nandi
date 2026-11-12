//! Agent memory (`services::agent_memory`).

use crate::services::agent_memory::{self, MemoryEntry};

super::commands! {
    /// An agent's memory entries
    fn load_agent_memory(state, agent_id: String) -> Vec<MemoryEntry> => agent_memory::load_memory(&state, &agent_id).await;

    /// Replaces one memory entry
    fn update_agent_memory(state, agent_id: String, index: u32, content: String) -> () => agent_memory::update_memory_entry(&state, &agent_id, index, content).await;

    /// Deletes one memory entry
    fn delete_agent_memory(state, agent_id: String, index: u32) -> () => agent_memory::delete_memory(&state, &agent_id, index).await;

    /// Re-runs extraction on the agent's latest run; returns the whole memory
    fn refresh_agent_memory(state, agent_id: String) -> Vec<MemoryEntry> => agent_memory::refresh_from_run(&state, &agent_id, None).await;

    /// Whether the agent keeps a memory
    fn get_agent_memory_enabled(state, agent_id: String) -> bool => agent_memory::memory_enabled(&state, &agent_id).await;

    /// Writes `memory_disabled` to the agent's skill.md
    fn set_agent_memory_enabled(state, agent_id: String, enabled: bool) -> () => agent_memory::set_memory_enabled(&state, &agent_id, enabled).await;
}
