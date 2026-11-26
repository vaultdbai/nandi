//! Phone Controls: how much an ask from a paired phone may do, set per
//! device in Settings › Your devices and enforced here, at every tool call
//! of a phone-started run (`orchestrator::call_gate`), never in the UI.
//!
//! | Level                           | A phone-started run may                                  |
//! | ------------------------------- | -------------------------------------------------------- |
//! | [`AskLevel::Answer`]            | read what the agent may already read, and answer         |
//! | [`AskLevel::Confirm`] (default) | act, but every action waits for the person's yes         |
//! | [`AskLevel::Off`]               | nothing: the phone only approves, asks are refused       |
//!
//! At every level a phone never creates agents or automations, gives new
//! access or changes settings ([`never_from_a_phone`]), and never unlocks a
//! Lock box key that asks each time (`lock_box::release`). A run whose
//! device is gone (revoked) or locked does nothing either.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::model::{RunSummary, SourceType};

/// How much a paired phone may ask
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AskLevel {
    /// Read and answer; no actions
    Answer,
    /// Act, with the person's yes on every action
    #[default]
    Confirm,
    /// No asks: the phone only approves
    Off,
}

impl AskLevel {
    /// As `own_devices.ask_level` stores it (migration 013)
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Answer => "answer",
            Self::Confirm => "confirm",
            Self::Off => "off",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "answer" => Some(Self::Answer),
            "confirm" => Some(Self::Confirm),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// What a phone-started run may do with one tool call
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhoneVerdict {
    /// As at the computer (a read)
    Allow,
    /// Only with the person's yes, whatever the agent's own rules say
    NeedsYes,
    /// Not from a phone: the model reads why
    Refuse(String),
}

/// The device a run was asked from (`RunSource::own_device`), if any
pub fn asked_from(run: &RunSummary) -> Option<&str> {
    (run.source_type == SourceType::User)
        .then_some(run.source_id.as_deref())
        .flatten()
}

/// Never from a phone, at any level: making agents or automations, new
/// access, the person's settings and profile
pub fn never_from_a_phone(tool: &str) -> bool {
    tool == crate::services::studio::save::SAVE_ARTIFACT
        || crate::services::studio::tools::is_build_tool(tool)
        || tool == crate::services::access::ACCESS_TOOL
        || tool == crate::services::inbox::tool::PROPOSE
        || crate::services::playbooks::is_propose(tool)
}

/// The tools that only read what the agent may already read: its sandbox's
/// files, the web, the person's mail, calendar and files, its notes, and
/// the bookkeeping of its own turn
pub fn only_reads(tool: &str) -> bool {
    matches!(
        tool,
        "read_file"
            | "list_dir"
            | "web_search__search"
            | "filesystem__read_file"
            | "filesystem__list_dir"
            // Who is responsible, open requests, the to-do list
            | "find_people"
            | "my_requests"
            | "todo_list"
    ) || tool == crate::services::orchestrator::tool_search::TOOL_SEARCH_NAME
        || crate::services::email::tool::is_mail_tool(tool)
        || crate::services::workspace::tool::is_workspace_tool(tool)
        || crate::services::run_plan::tool::is_plan_tool(tool)
}

/// What `level` lets a phone-started run do with `tool`. `None` for the
/// level: the device is gone or locked, and the run does nothing.
pub fn verdict(level: Option<AskLevel>, tool: &str) -> PhoneVerdict {
    if never_from_a_phone(tool) {
        return PhoneVerdict::Refuse(
            "Not from a phone: making agents or automations, new access and settings are \
             done on the computer. Tell the person to do it there."
                .to_string(),
        );
    }
    match level {
        None | Some(AskLevel::Off) => PhoneVerdict::Refuse(
            "This phone may no longer ask: its device was removed, locked or turned off."
                .to_string(),
        ),
        Some(_) if only_reads(tool) => PhoneVerdict::Allow,
        Some(AskLevel::Answer) => PhoneVerdict::Refuse(
            "This phone may only ask for answers: no actions. Answer from what you can read, \
             or tell the person to do it on the computer."
                .to_string(),
        ),
        Some(AskLevel::Confirm) => PhoneVerdict::NeedsYes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIONS: &[&str] = &[
        "shell",
        "write_file",
        "gmail__send_message",
        "delegate_to_agent",
    ];
    const READS: &[&str] = &["read_file", "list_dir", "web_search__search", "tool_search"];
    const NEVER: &[&str] = &[
        "nandi_save_artifact",
        "nandi_check_artifact",
        "connector_access",
        "knowme_propose",
    ];

    #[test]
    fn answer_only_reads_and_answers() {
        for tool in READS {
            assert_eq!(
                verdict(Some(AskLevel::Answer), tool),
                PhoneVerdict::Allow,
                "{tool}"
            );
        }
        for tool in ACTIONS {
            assert!(
                matches!(
                    verdict(Some(AskLevel::Answer), tool),
                    PhoneVerdict::Refuse(_)
                ),
                "{tool}"
            );
        }
    }

    #[test]
    fn do_with_your_yes_asks_for_every_action() {
        for tool in READS {
            assert_eq!(
                verdict(Some(AskLevel::Confirm), tool),
                PhoneVerdict::Allow,
                "{tool}"
            );
        }
        for tool in ACTIONS {
            assert_eq!(
                verdict(Some(AskLevel::Confirm), tool),
                PhoneVerdict::NeedsYes,
                "{tool}"
            );
        }
    }

    #[test]
    fn off_a_gone_or_locked_device_does_nothing() {
        for level in [Some(AskLevel::Off), None] {
            for tool in READS.iter().chain(ACTIONS) {
                assert!(
                    matches!(verdict(level, tool), PhoneVerdict::Refuse(_)),
                    "{tool}"
                );
            }
        }
    }

    #[test]
    fn the_never_list_is_refused_at_every_level() {
        for level in [
            Some(AskLevel::Answer),
            Some(AskLevel::Confirm),
            Some(AskLevel::Off),
            None,
        ] {
            for tool in NEVER {
                let PhoneVerdict::Refuse(why) = verdict(level, tool) else {
                    panic!("{tool} at {level:?}");
                };
                assert!(
                    why.contains("Not from a phone") || level.is_none_or(|l| l == AskLevel::Off)
                );
            }
        }
    }

    #[test]
    fn levels_read_back_as_stored() {
        for level in [AskLevel::Answer, AskLevel::Confirm, AskLevel::Off] {
            assert_eq!(AskLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(AskLevel::parse("all"), None);
        assert_eq!(AskLevel::default(), AskLevel::Confirm);
    }
}
