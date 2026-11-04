//! Built-in tools (wave A, Track C): what the container runtime image
//! serves (`runtimes/nandi-runtime/app.py`: shell, read_file, write_file,
//! list_dir) plus the host's own grantable tools (`delegate_to_agent`, and
//! the people tools `find_people` / `my_requests`), as
//! `ToolSource::Builtin` definitions. Network access goes through the
//! egress proxy from `shell`; the runtime has no separate fetch tool.

use serde_json::{json, Value};

use super::definition::{normalize_id, LatencyClass, ToolDefinition, ToolSource};
use crate::services::a2a::people;
use crate::services::domain::capability::{DEFAULT_REQUIRES_APPROVAL, DELEGATION_TOOL};

/// Every built-in, in the order the runtime lists them
pub fn builtin_tools() -> Vec<ToolDefinition> {
    let path = || json!({ "type": "string", "description": "Relative to the workdir" });
    vec![
        builtin(
            "shell",
            "Run a shell command in the workdir. Times out after 30 seconds.",
            object(json!({ "cmd": { "type": "string" } }), &["cmd"]),
            LatencyClass::Slow,
            &["runtime", "shell"],
        ),
        builtin(
            "read_file",
            "Read a UTF-8 text file from the workdir.",
            object(json!({ "path": path() }), &["path"]),
            LatencyClass::Instant,
            &["runtime", "filesystem"],
        ),
        builtin(
            "write_file",
            "Write a UTF-8 text file in the workdir, creating parent folders.",
            object(
                json!({ "path": path(), "content": { "type": "string" } }),
                &["path", "content"],
            ),
            LatencyClass::Instant,
            &["runtime", "filesystem"],
        ),
        builtin(
            "list_dir",
            "List a folder in the workdir. Subfolders end with '/'.",
            object(json!({ "path": path() }), &[]),
            LatencyClass::Instant,
            &["runtime", "filesystem"],
        ),
        ToolDefinition {
            // Cross-user calls need approval unless skill.md says otherwise
            requires_approval: true,
            ..builtin(
                DELEGATION_TOOL,
                "Hand a task to another agent, on this Nandi or across the network.",
                Value::Null,
                LatencyClass::Long,
                &["delegation", "network"],
            )
        },
        people_tool(people::FIND_PEOPLE, LatencyClass::Fast),
        people_tool(people::MY_REQUESTS, LatencyClass::Instant),
    ]
}

/// A people tool, described as the agent sees it (offered only when its
/// skill.md lists it)
fn people_tool(name: &str, latency_class: LatencyClass) -> ToolDefinition {
    let spec = people::spec(name).expect("people tool names are known");
    builtin(
        name,
        &spec.description,
        spec.input_schema,
        latency_class,
        &["people", "network"],
    )
}

fn builtin(
    name: &str,
    description: &str,
    input_schema: Value,
    latency_class: LatencyClass,
    tags: &[&str],
) -> ToolDefinition {
    ToolDefinition {
        id: normalize_id(name),
        name: name.to_string(),
        description: description.to_string(),
        source: ToolSource::Builtin,
        input_schema: (!input_schema.is_null()).then_some(input_schema),
        requires_approval: DEFAULT_REQUIRES_APPROVAL.contains(&name),
        cost_estimate: None,
        latency_class,
        tags: tags.iter().map(|t| t.to_string()).collect(),
    }
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerates_the_runtime_tools_delegation_and_people_tools() {
        let tools = builtin_tools();
        let ids: Vec<_> = tools.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "builtin.shell",
                "builtin.read_file",
                "builtin.write_file",
                "builtin.list_dir",
                "builtin.delegate_to_agent",
                "builtin.find_people",
                "builtin.my_requests"
            ]
        );
        assert!(tools.iter().all(|t| t.source == ToolSource::Builtin));
        // Read-only: never gated
        assert!(tools
            .iter()
            .filter(|t| t.tags.contains(&"people".to_string()))
            .all(|t| !t.requires_approval));
    }

    #[test]
    fn approval_follows_the_runtime_defaults() {
        let approval = |id: &str| {
            builtin_tools()
                .into_iter()
                .find(|t| t.id == id)
                .unwrap()
                .requires_approval
        };
        assert!(approval("builtin.shell") && approval("builtin.write_file"));
        assert!(!approval("builtin.read_file") && !approval("builtin.list_dir"));
        assert!(approval("builtin.delegate_to_agent"));
    }
}
