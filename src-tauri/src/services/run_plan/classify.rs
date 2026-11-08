//! What a tool call does to the world, for the run plan's rules: the one
//! place that decides "changes something" (task decision: browser typing,
//! clicking, submitting; starting or updating work; delegating; writing
//! files outside the agent's notes).
//!
//! | Call                                                   | Effect                   |
//! | ------------------------------------------------------ | ------------------------ |
//! | `plan_set`, `plan_update`, reads, mail, people, search  | none                     |
//! | browser reading or moving between pages                | none                     |
//! | browser clicking, typing, files                         | changes                  |
//! | browser submit, payment, secret, hand-over             | risky (the user's)       |
//! | `work_update`                                          | changes                  |
//! | `work_start`, `delegate_to_agent`                      | risky (a delegation)     |
//! | writing files, except notes and memory                 | changes                  |
//!
//! Shell commands aren't classified: most only read, and the decision
//! names files. A risky call runs only while the current (`doing`) step is
//! of a kind that covers it ([`check_call`]).

use serde_json::Value;

use super::{RunPlan, RunStepKind};
use crate::services::pilots::browser::Action;

/// A risky call: only while a step of a covering kind is being done
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Submit,
    Pay,
    Secret,
    HandOver,
    Delegate,
}

impl Risk {
    /// The step kinds that cover it
    pub fn covered_by(self) -> &'static [RunStepKind] {
        match self {
            Self::Submit | Self::Pay | Self::Secret | Self::HandOver => &[
                RunStepKind::Submit,
                RunStepKind::Handover,
                RunStepKind::Approval,
            ],
            Self::Delegate => &[RunStepKind::Delegate, RunStepKind::Approval],
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Submit => "submits something",
            Self::Pay => "pays",
            Self::Secret => "types a secret (signing in)",
            Self::HandOver => "hands over to the person",
            Self::Delegate => "starts work or delegates it",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Reads, notes, the plan tools: never need a plan
    None,
    /// Needs a plan first
    Changes,
    Risky(Risk),
}

/// File tools that change a file (after a connector prefix)
const FILE_CHANGE_TOOLS: &[&str] = &[
    "write_file",
    "edit_file",
    "create_file",
    "append_file",
    "delete_file",
    "remove_file",
    "move_file",
    "rename_file",
    "copy_file",
    "make_dir",
    "apply_patch",
];
const PATH_KEYS: &[&str] = &["path", "file_path", "source", "destination", "from", "to"];
/// Notes folders: the Chief of Staff's `my-notes`, an agent's own notes and
/// memory
const NOTES_PREFIXES: &[&str] = &["my-notes/", "notes/", "memory/"];

fn base_tool_name(tool: &str) -> &str {
    tool.rsplit_once("__").map_or(tool, |(_, base)| base)
}

fn is_notes_path(path: &str) -> bool {
    let path = path.trim().replace('\\', "/");
    let path = path.trim_start_matches("./");
    let path = path.strip_prefix("/agent/workdir/").unwrap_or(path);
    !path.split('/').any(|part| part == "..")
        && (path == "memory.md" || NOTES_PREFIXES.iter().any(|p| path.starts_with(p)))
}

/// The effect of `tool` called with `args`. `browser` is the browser
/// pilot's own classification of the call (`browser::action`) when it's a
/// browser tool.
pub fn effect(tool: &str, args: &Value, browser: Option<Action>) -> Effect {
    if let Some(action) = browser {
        return match action {
            Action::Read | Action::Navigate => Effect::None,
            Action::Interact | Action::Upload => Effect::Changes,
            Action::Submit => Effect::Risky(Risk::Submit),
            Action::Pay => Effect::Risky(Risk::Pay),
            Action::Secret => Effect::Risky(Risk::Secret),
            Action::HandOver => Effect::Risky(Risk::HandOver),
        };
    }
    match tool {
        crate::services::requests::tool::START | crate::services::a2a::tool::NAME => {
            return Effect::Risky(Risk::Delegate)
        }
        crate::services::requests::tool::UPDATE => return Effect::Changes,
        _ => {}
    }
    if !FILE_CHANGE_TOOLS.contains(&base_tool_name(tool)) {
        return Effect::None;
    }
    let paths: Vec<&str> = PATH_KEYS
        .iter()
        .filter_map(|key| args.get(key).and_then(Value::as_str))
        .collect();
    if !paths.is_empty() && paths.iter().all(|path| is_notes_path(path)) {
        Effect::None
    } else {
        Effect::Changes
    }
}

/// What the model is told when a call needs a plan first
pub fn plan_first_message(tool: &str) -> String {
    format!(
        "Blocked: '{tool}' changes something, so make a plan first (`plan_set`): the steps in order, each with its kind (navigate, read, fill, submit, handover, approval, delegate, verify, cleanup, other) and `evidence: screenshot` where a screenshot belongs in the log. Then mark the step you're on `doing` with `plan_update` and try again."
    )
}

/// Why a call may not run under `plan`, if it may not. `screenshot` says
/// it's the browser's screenshot tool, which a hand-over step refuses.
pub fn check_call(
    plan: Option<&RunPlan>,
    tool: &str,
    effect: Effect,
    screenshot: bool,
) -> Option<String> {
    let current = plan.and_then(RunPlan::current);
    if screenshot && current.is_some_and(|(_, step)| step.kind == RunStepKind::Handover) {
        return Some(
            "Blocked: no screenshots during a handover step: the person may be typing a password or card number. Take it once the step is done."
                .to_string(),
        );
    }
    let risk = match effect {
        Effect::None => return None,
        Effect::Changes => return plan.is_none().then(|| plan_first_message(tool)),
        Effect::Risky(risk) => risk,
    };
    if plan.is_none() {
        return Some(plan_first_message(tool));
    }
    let kinds = risk.covered_by();
    if current.is_some_and(|(_, step)| kinds.contains(&step.kind)) {
        return None;
    }
    let names: Vec<String> = kinds.iter().map(|k| format!("`{}`", k.as_str())).collect();
    let now = match current {
        Some((number, step)) => format!(
            "The current step ({number}, {}) is a `{}` step.",
            step.title,
            step.kind.as_str()
        ),
        None => "No step is marked doing.".to_string(),
    };
    Some(format!(
        "Blocked: '{tool}' {}, which is the user's to decide. {now} Mark a {} step as doing with `plan_update` first, or add one with `plan_set` if the plan has none. Approvals still apply.",
        risk.describe(),
        names.join(" or ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::run_plan::parse_steps;
    use crate::services::run_plan::{apply_update, RunPlan};
    use serde_json::json;

    #[test]
    fn effects_table() {
        let none = json!({});
        let cases: Vec<(&str, Value, Option<Action>, Effect)> = vec![
            ("plan_set", none.clone(), None, Effect::None),
            ("plan_update", none.clone(), None, Effect::None),
            ("read_file", json!({ "path": "a.md" }), None, Effect::None),
            ("list_dir", json!({ "path": "." }), None, Effect::None),
            (
                "shell",
                json!({ "command": "rm -rf x" }),
                None,
                Effect::None,
            ),
            ("mail_search", none.clone(), None, Effect::None),
            ("mail_get_message", none.clone(), None, Effect::None),
            ("work_open", none.clone(), None, Effect::None),
            ("find_people", none.clone(), None, Effect::None),
            ("tool_search", none.clone(), None, Effect::None),
            ("work_update", none.clone(), None, Effect::Changes),
            (
                "work_start",
                none.clone(),
                None,
                Effect::Risky(Risk::Delegate),
            ),
            (
                "delegate_to_agent",
                none.clone(),
                None,
                Effect::Risky(Risk::Delegate),
            ),
            (
                "write_file",
                json!({ "path": "report.md" }),
                None,
                Effect::Changes,
            ),
            (
                "filesystem__write_file",
                json!({ "path": "my-notes/people.md" }),
                None,
                Effect::None,
            ),
            (
                "filesystem__write_file",
                json!({ "path": "my-notes/../x.md" }),
                None,
                Effect::Changes,
            ),
            (
                "write_file",
                json!({ "path": "/agent/workdir/notes/todo.md" }),
                None,
                Effect::None,
            ),
            (
                "write_file",
                json!({ "path": "memory.md" }),
                None,
                Effect::None,
            ),
            (
                "edit_file",
                json!({ "path": "src/main.rs" }),
                None,
                Effect::Changes,
            ),
            ("delete_file", json!({}), None, Effect::Changes),
            (
                "move_file",
                json!({ "source": "my-notes/a.md", "destination": "out/a.md" }),
                None,
                Effect::Changes,
            ),
            (
                "pilot_browser__browser_snapshot",
                none.clone(),
                Some(Action::Read),
                Effect::None,
            ),
            (
                "pilot_browser__browser_navigate",
                none.clone(),
                Some(Action::Navigate),
                Effect::None,
            ),
            (
                "pilot_browser__browser_close",
                none.clone(),
                Some(Action::Navigate),
                Effect::None,
            ),
            (
                "pilot_browser__browser_click",
                none.clone(),
                Some(Action::Interact),
                Effect::Changes,
            ),
            (
                "pilot_browser__browser_file_upload",
                none.clone(),
                Some(Action::Upload),
                Effect::Changes,
            ),
            (
                "pilot_browser__browser_click",
                none.clone(),
                Some(Action::Submit),
                Effect::Risky(Risk::Submit),
            ),
            (
                "pilot_browser__browser_click",
                none.clone(),
                Some(Action::Pay),
                Effect::Risky(Risk::Pay),
            ),
            (
                "pilot_browser__browser_type",
                none.clone(),
                Some(Action::Secret),
                Effect::Risky(Risk::Secret),
            ),
            (
                "pilot_browser__browser_hand_over",
                none.clone(),
                Some(Action::HandOver),
                Effect::Risky(Risk::HandOver),
            ),
        ];
        for (tool, args, browser, expected) in cases {
            assert_eq!(effect(tool, &args, browser), expected, "{tool} {args}");
        }
    }

    fn plan() -> RunPlan {
        RunPlan {
            run_id: "r1".into(),
            revision: 1,
            steps: parse_steps(&json!({ "steps": [
                { "title": "Open the site", "kind": "navigate" },
                { "title": "Sign in", "kind": "handover" },
                { "title": "Ask Lena", "kind": "delegate" },
                { "title": "Send it", "kind": "submit" },
            ]}))
            .unwrap(),
            updated_at: 0.0,
        }
    }

    fn doing(step: u64) -> RunPlan {
        let mut plan = plan();
        apply_update(&mut plan, &json!({ "step": step, "status": "doing" })).unwrap();
        plan
    }

    #[test]
    fn a_change_needs_a_plan_first_and_reads_never_do() {
        let blocked = check_call(None, "write_file", Effect::Changes, false).unwrap();
        assert!(
            blocked.contains("make a plan first (`plan_set`)"),
            "{blocked}"
        );
        assert!(check_call(None, "read_file", Effect::None, false).is_none());
        assert!(check_call(Some(&plan()), "write_file", Effect::Changes, false).is_none());
    }

    #[test]
    fn risky_calls_need_a_covering_doing_step() {
        let pay = Effect::Risky(Risk::Pay);
        let delegate = Effect::Risky(Risk::Delegate);
        assert!(check_call(None, "pay", pay, false)
            .unwrap()
            .contains("plan first"));
        let blocked = check_call(Some(&plan()), "click", pay, false).unwrap();
        assert!(blocked.contains("No step is marked doing"), "{blocked}");
        assert!(
            blocked.contains("`submit` or `handover` or `approval`"),
            "{blocked}"
        );
        let blocked = check_call(Some(&doing(1)), "click", pay, false).unwrap();
        assert!(
            blocked.contains("(1, Open the site) is a `navigate` step"),
            "{blocked}"
        );
        assert!(check_call(Some(&doing(2)), "click", pay, false).is_none());
        assert!(check_call(Some(&doing(4)), "click", Effect::Risky(Risk::Submit), false).is_none());
        let blocked = check_call(Some(&doing(2)), "work_start", delegate, false).unwrap();
        assert!(blocked.contains("`delegate` or `approval`"), "{blocked}");
        assert!(check_call(Some(&doing(3)), "work_start", delegate, false).is_none());
        // A delegate step doesn't cover a payment
        assert!(check_call(Some(&doing(3)), "click", pay, false).is_some());
    }

    #[test]
    fn a_handover_step_refuses_screenshots() {
        let blocked = check_call(Some(&doing(2)), "shot", Effect::None, true).unwrap();
        assert!(
            blocked.contains("no screenshots during a handover"),
            "{blocked}"
        );
        assert!(check_call(Some(&doing(1)), "shot", Effect::None, true).is_none());
        assert!(check_call(None, "shot", Effect::None, true).is_none());
    }
}
