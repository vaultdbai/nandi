//! Connector tool names as the model sees them: `<connector_id>__<tool>`.
//!
//! Container tools never contain `__`, so a prefixed name can't shadow one.
//! LLM providers accept `[A-Za-z0-9_-]{1,64}`, so other characters in a
//! connector ID (it may contain `.`) become `_`, and names over 64
//! characters are refused rather than truncated (truncation could collide).

const SEPARATOR: &str = "__";
const MAX_TOOL_NAME: usize = 64;

/// The `<prefix>` part of a connector's tool names.
pub(super) fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn valid_tool(tool: &str) -> bool {
    !tool.is_empty()
        && tool
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The model-facing name of `tool` on connector `connector_id`, or `None`
/// if it can't be expressed safely.
pub fn prefixed_name(connector_id: &str, tool: &str) -> Option<String> {
    let prefix = sanitize(connector_id);
    // A `__` in the prefix would make the split ambiguous
    if prefix.is_empty() || prefix.contains(SEPARATOR) || !valid_tool(tool) {
        return None;
    }
    let name = format!("{prefix}{SEPARATOR}{tool}");
    (name.len() <= MAX_TOOL_NAME).then_some(name)
}

/// Splits a prefixed name into `(connector_id, tool)`, matching only the
/// connectors in `declared`. `None`: not one of their tools.
pub fn parse_prefixed(name: &str, declared: &[String]) -> Option<(String, String)> {
    let (prefix, tool) = name.split_once(SEPARATOR)?;
    if tool.is_empty() {
        return None;
    }
    let id = declared.iter().find(|id| sanitize(id) == prefix)?;
    Some((id.clone(), tool.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_with_the_connector_id() {
        assert_eq!(
            prefixed_name("filesystem", "read_file").as_deref(),
            Some("filesystem__read_file")
        );
        assert_eq!(
            prefixed_name("team.docs", "list_dir").as_deref(),
            Some("team_docs__list_dir")
        );
    }

    #[test]
    fn refuses_names_providers_would_reject() {
        assert_eq!(prefixed_name("fs", "bad name"), None);
        assert_eq!(prefixed_name("fs", ""), None);
        assert_eq!(prefixed_name("", "read_file"), None);
        assert_eq!(prefixed_name(&"x".repeat(60), "read_file"), None);
        assert_eq!(prefixed_name("my__fs", "read_file"), None);
    }

    #[test]
    fn parses_only_declared_connectors() {
        let declared = vec!["filesystem".to_string(), "team.docs".to_string()];
        assert_eq!(
            parse_prefixed("filesystem__read_file", &declared),
            Some(("filesystem".into(), "read_file".into()))
        );
        // Tool names may themselves contain `__`
        assert_eq!(
            parse_prefixed("team_docs__a__b", &declared),
            Some(("team.docs".into(), "a__b".into()))
        );
        assert_eq!(parse_prefixed("web_search__search", &declared), None);
        assert_eq!(parse_prefixed("read_file", &declared), None);
        assert_eq!(parse_prefixed("filesystem__", &declared), None);
    }

    #[test]
    fn container_tools_never_look_prefixed() {
        let declared = vec!["shell".to_string()];
        for container_tool in ["shell", "read_file", "write_file", "list_dir"] {
            assert_eq!(parse_prefixed(container_tool, &declared), None);
        }
    }
}
