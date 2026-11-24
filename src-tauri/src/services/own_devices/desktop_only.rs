//! Which approvals a paired phone may approve. The riskiest stay on the
//! computer: payments, sending, new access or grants, anything that
//! widens what an agent may do, and anything a browser types or submits.
//! A phone may still **decline** any of them: declining never widens.
//!
//! | Approval                                                   | From a phone |
//! | ---------------------------------------------------------- | ------------ |
//! | An access request (`connector_access`): new access         | desktop only |
//! | Starting work or delegating (`work_start`, …)              | desktop only |
//! | A browser call that isn't reading or moving between pages  | desktop only |
//! | A tool whose name says it sends, pays, grants or shares    | desktop only |
//! | Anything else (writing a file, running a report, …)        | may approve  |
//!
//! The name check reads the tool's own name (after a connector's `__`
//! prefix), word by word: `send_email`, `gmail__reply_to_thread`,
//! `stripe__create_payment`. It errs toward the desktop.

use serde_json::Value;

use crate::services::pilots;
use crate::services::run_plan::classify::{self, Effect};

/// Words in a tool's name that make its approval desktop only
const RISKY_WORDS: &[&str] = &[
    "send",
    "reply",
    "forward",
    "post",
    "publish",
    "tweet",
    "share",
    "invite",
    "grant",
    "pay",
    "payment",
    "payments",
    "transfer",
    "purchase",
    "buy",
    "order",
    "checkout",
    "refund",
    "permission",
    "permissions",
    "access",
    "allow",
    "trust",
    "device",
    "devices",
];

/// Why the approval of `tool` with `args` can only be given on the
/// computer, or `None` when a phone may give it
pub fn reason(tool: &str, args: &Value) -> Option<&'static str> {
    if tool == crate::services::access::ACCESS_TOOL {
        return Some("It gives an agent new access");
    }
    if let Some((pilot, browser_tool)) = pilots::tools::parse_tool_name(tool) {
        if pilots::browser::is_browser(pilot) {
            let action = pilots::browser::action(
                browser_tool,
                args,
                pilots::browser::is_read_tool(browser_tool),
                &[],
            );
            return match action {
                pilots::browser::Action::Read | pilots::browser::Action::Navigate => None,
                _ => Some("The browser would type, submit or pay on a site"),
            };
        }
    }
    if matches!(classify::effect(tool, args, None), Effect::Risky(_)) {
        return Some("It starts work or hands it to someone");
    }
    let base = tool.rsplit_once("__").map_or(tool, |(_, base)| base);
    let risky = base
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| RISKY_WORDS.contains(&word.to_ascii_lowercase().as_str()));
    risky.then_some("It sends, pays, shares or grants something")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn risky_approvals_stay_on_the_computer() {
        let none = json!({});
        for tool in [
            "connector_access",
            "work_start",
            "delegate_to_agent",
            "send_email",
            "gmail__reply_to_thread",
            "outlook__forward_message",
            "stripe__create_payment",
            "github__invite_collaborator",
            "drive__share_file",
            "aws__grant_role",
            "linkedin__publish_post",
        ] {
            assert!(reason(tool, &none).is_some(), "{tool}");
        }
    }

    #[test]
    fn everyday_approvals_may_be_given_from_a_phone() {
        let none = json!({});
        for tool in [
            "write_file",
            "run_report",
            "shell",
            "notes__append",
            "s3__put_object",
            "calendar__create_event",
        ] {
            assert_eq!(reason(tool, &none), None, "{tool}");
        }
    }

    #[test]
    fn a_browser_may_only_read_or_move_on_from_a_phone() {
        let browser = |tool: &str| pilots::tools::tool_name(pilots::browser::BROWSER, tool);
        assert_eq!(reason(&browser("browser_snapshot"), &json!({})), None);
        for tool in ["browser_type", "browser_click"] {
            let args = json!({"element": "Pay now", "text": "x"});
            assert!(reason(&browser(tool), &args).is_some(), "{tool}");
        }
    }
}
