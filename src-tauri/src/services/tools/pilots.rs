//! Pilot tools (wave A, Track C): the host-side MCP tools of each pilot
//! (`services::pilots`), `ToolSource::Pilot`. Only a routable (running,
//! healthy) pilot can list its tools; an installed pilot that isn't keeps
//! the tools last seen, so stopping it doesn't make capabilities lose them.

use serde_json::Value;

use super::definition::{LatencyClass, ToolDefinition, ToolSource};
use crate::llm::ToolSpec;
use crate::services::pilots::{catalog, store, tools as pilot_tools_mod, PilotDefinition};
use crate::services::AppState;

/// What one scan of the pilots found
#[derive(Debug, Default)]
pub struct PilotScan {
    /// Tools of the routable pilots
    pub tools: Vec<ToolDefinition>,
    /// Installed pilots whose tools couldn't be listed now: keep their
    /// catalog rows
    pub unlisted: Vec<String>,
}

pub async fn scan(state: &AppState) -> PilotScan {
    let mut scan = PilotScan::default();
    for (pilot, persisted) in persisted(state) {
        if !persisted.installed {
            continue;
        }
        if !persisted.is_routable() {
            scan.unlisted.push(pilot.id.clone());
            continue;
        }
        match pilot_tools_mod::list(&pilot).await {
            Ok(tools) => scan.tools.extend(pilot_definitions(
                &pilot,
                tools.into_iter().map(|t| (t.spec, t.read_only)).collect(),
            )),
            Err(e) => {
                log::debug!("Tool catalog: {} didn't list its tools ({e})", pilot.id);
                scan.unlisted.push(pilot.id.clone());
            }
        }
    }
    scan
}

/// `(pilot, read_only)` tool specs as catalog definitions. Pilot tools need
/// approval unless the pilot marks them read-only.
pub fn pilot_definitions(
    pilot: &PilotDefinition,
    tools: Vec<(ToolSpec, bool)>,
) -> Vec<ToolDefinition> {
    tools
        .into_iter()
        .map(|(spec, read_only)| ToolDefinition {
            id: format!("{}.{}", pilot.id, spec.name),
            name: spec.name,
            description: spec.description,
            source: ToolSource::Pilot {
                pilot_id: pilot.id.clone(),
            },
            input_schema: (!spec.input_schema.is_null()).then_some(spec.input_schema),
            requires_approval: !read_only,
            cost_estimate: None,
            latency_class: LatencyClass::Fast,
            tags: std::iter::once("pilot".to_string())
                .chain(pilot.capabilities.iter().cloned())
                .collect(),
        })
        .collect()
}

/// Each pilot's persisted status, for change detection (no network)
pub fn fingerprint(state: &AppState) -> Vec<(String, Value)> {
    persisted(state)
        .into_iter()
        .map(|(pilot, p)| {
            let status = serde_json::to_value(p.status()).unwrap_or(Value::Null);
            (pilot.id, status)
        })
        .collect()
}

fn persisted(state: &AppState) -> Vec<(PilotDefinition, store::PersistedPilot)> {
    let pilots = catalog::for_platform(catalog::current_platform());
    let Ok(conn) = state.db().conn() else {
        return Vec::new();
    };
    pilots
        .into_iter()
        .filter_map(|pilot| {
            let persisted = store::load(&conn, &pilot.id).ok()?;
            Some((pilot, persisted))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pilot() -> PilotDefinition {
        PilotDefinition {
            id: "opendesk".into(),
            name: "OSWright".into(),
            platform: "any".into(),
            install_command: String::new(),
            start_command: String::new(),
            mcp_port: 0,
            description: String::new(),
            capabilities: vec!["screen.read".into()],
        }
    }

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: format!("{name} tool"),
            input_schema: json!({ "type": "object" }),
        }
    }

    #[test]
    fn a_started_pilot_contributes_its_tools() {
        let tools = pilot_definitions(
            &pilot(),
            vec![(spec("screenshot"), true), (spec("click"), false)],
        );
        assert_eq!(tools[0].id, "opendesk.screenshot");
        assert_eq!(
            tools[1].source,
            ToolSource::Pilot {
                pilot_id: "opendesk".into()
            }
        );
        assert!(!tools[0].requires_approval && tools[1].requires_approval);
        assert!(tools[0].tags.contains(&"screen.read".to_string()));
    }
}
