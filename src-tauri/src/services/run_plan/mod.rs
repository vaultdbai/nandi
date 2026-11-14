//! Run plans: how one run does its part, step by step (task "Run plans,
//! playbooks and site accounts", step 1).
//!
//! A Request's steps say who does what (the work plan); a run plan says how
//! one run gets its part done: open the site, sign in (handed over), go to
//! billing, fill in the amount, pay (handed over), read the confirmation,
//! close the browser. Every agent gets two host tools ([`tool`]):
//!
//! | Tool          | Arguments                                   | Does                                        |
//! | ------------- | ------------------------------------------- | ------------------------------------------- |
//! | `plan_set`    | `{steps: [{title, kind, evidence?, note?}]}` | sets the run's plan (a new revision)        |
//! | `plan_update` | `{step, status, result?, files?}`           | one step's status, result and evidence      |
//!
//! The plan is run data (`run_plans`, [`store`]), never the gold copy. Two
//! middlewares enforce it (`orchestrator/run_plan.rs`): plan first (a call
//! that changes something needs a plan; a risky one needs a `doing` step of
//! a kind that covers it, [`classify`]) and the finish check (one
//! continuation when steps are open, evidence is missing or the browser is
//! still open). Screenshots are named per step and kept 30 days
//! ([`screenshots`]).
//!
//! This module is pure: parsing the tool arguments, applying an update,
//! and what the finish check looks at.

pub mod classify;
pub mod screenshots;
pub mod store;
pub mod tool;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use strum::{EnumString, IntoStaticStr, VariantNames};

use crate::model::Timestamp;
use crate::services::events::NandiEvent;

/// Steps in one plan at most
pub const MAX_STEPS: usize = 30;
/// Longest step title, in characters
pub const MAX_TITLE_CHARS: usize = 200;
/// Longest note or result, in characters
pub const MAX_TEXT_CHARS: usize = 500;
/// Evidence files linked to one step at most
pub const MAX_FILES: usize = 10;

/// What a step does: decides which risky calls it covers
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Type,
    EnumString,
    IntoStaticStr,
    VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RunStepKind {
    Navigate,
    Read,
    Fill,
    Submit,
    /// The person does it (sign-in, payment, posting): never screenshotted
    Handover,
    Approval,
    Delegate,
    Verify,
    Cleanup,
    Other,
}

impl RunStepKind {
    /// The kind's name
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Type,
    EnumString,
    IntoStaticStr,
    VariantNames,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RunStepStatus {
    Pending,
    Doing,
    Done,
    Skipped,
    Failed,
}

impl RunStepStatus {
    /// The status's name
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Still to do: the finish check wants it closed
    pub fn is_open(self) -> bool {
        matches!(self, Self::Pending | Self::Doing)
    }
}

/// What a step should leave behind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RunStepEvidence {
    Screenshot,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunPlanStep {
    pub title: String,
    pub kind: RunStepKind,
    pub evidence: RunStepEvidence,
    pub note: Option<String>,
    pub status: RunStepStatus,
    /// A short result, set by `plan_update`
    pub result: Option<String>,
    /// Evidence files, relative to the agent's work folder
    /// (`browser/<run>/<step>-<n>.png`)
    pub files: Vec<String>,
}

impl RunPlanStep {
    /// Done, and marked `screenshot`, without a file to show for it
    pub fn misses_evidence(&self) -> bool {
        self.status == RunStepStatus::Done
            && self.evidence == RunStepEvidence::Screenshot
            && self.files.is_empty()
    }
}

/// One run's plan, latest revision
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunPlan {
    pub run_id: String,
    /// 1 for the first `plan_set`; each later one adds a revision
    pub revision: u32,
    pub steps: Vec<RunPlanStep>,
    pub updated_at: Timestamp,
}

impl RunPlan {
    /// The step being done now (1-based number and the step): the first
    /// `doing` one
    pub fn current(&self) -> Option<(usize, &RunPlanStep)> {
        self.steps
            .iter()
            .enumerate()
            .find(|(_, step)| step.status == RunStepStatus::Doing)
            .map(|(index, step)| (index + 1, step))
    }

    /// The 1-based numbers of steps still pending or doing
    pub fn open_steps(&self) -> Vec<usize> {
        numbers(&self.steps, |step| step.status.is_open())
    }

    /// The 1-based numbers of done steps marked `screenshot` with no file
    pub fn missing_evidence(&self) -> Vec<usize> {
        numbers(&self.steps, RunPlanStep::misses_evidence)
    }

    /// The plan as the model reads it back: one line per step
    pub fn checklist(&self) -> String {
        let mut text = format!("Run plan (revision {}):", self.revision);
        for (index, step) in self.steps.iter().enumerate() {
            text.push_str(&format!(
                "\n{}. [{}] {} ({})",
                index + 1,
                step.status.as_str(),
                step.title,
                step.kind.as_str()
            ));
            if step.evidence == RunStepEvidence::Screenshot {
                text.push_str(", screenshot");
            }
            if let Some(result) = &step.result {
                text.push_str(&format!(": {result}"));
            }
            if !step.files.is_empty() {
                text.push_str(&format!(" [{}]", step.files.join(", ")));
            }
        }
        text
    }
}

fn numbers(steps: &[RunPlanStep], keep: impl Fn(&RunPlanStep) -> bool) -> Vec<usize> {
    steps
        .iter()
        .enumerate()
        .filter(|(_, step)| keep(step))
        .map(|(index, _)| index + 1)
        .collect()
}

/// `run://plan_updated`: a run's plan was set or a step changed
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunPlanUpdated {
    pub agent_id: String,
    pub conversation_id: String,
    pub run_id: String,
    pub plan: RunPlan,
}

impl NandiEvent for RunPlanUpdated {
    const NAME: &'static str = "run://plan_updated";
}

// ---------------------------------------------------------------------------
// Tool arguments
// ---------------------------------------------------------------------------

fn text_arg(value: &Value, key: &str, max: usize) -> Result<Option<String>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            let text = text.trim();
            if text.chars().count() > max {
                Err(format!("`{key}` is longer than {max} characters"))
            } else {
                Ok((!text.is_empty()).then(|| text.to_string()))
            }
        }
        Some(_) => Err(format!("`{key}` must be text")),
    }
}

/// `plan_set`'s steps, all `pending`
pub fn parse_steps(args: &Value) -> Result<Vec<RunPlanStep>, String> {
    let items = args
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("`steps` must be a list of steps")?;
    if items.is_empty() {
        return Err("A plan needs at least one step".into());
    }
    if items.len() > MAX_STEPS {
        return Err(format!(
            "A plan has at most {MAX_STEPS} steps: plan at a higher level"
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| parse_step(item).map_err(|e| format!("Step {}: {e}", index + 1)))
        .collect()
}

fn parse_step(item: &Value) -> Result<RunPlanStep, String> {
    let title = text_arg(item, "title", MAX_TITLE_CHARS)?.ok_or("`title` is required")?;
    let kind = item
        .get("kind")
        .and_then(Value::as_str)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| format!("`kind` must be one of {}", RunStepKind::VARIANTS.join(", ")))?;
    let evidence = match item.get("evidence").and_then(Value::as_str) {
        None | Some("none") => RunStepEvidence::None,
        Some("screenshot") => RunStepEvidence::Screenshot,
        Some(other) => return Err(format!("`evidence` is screenshot or none, not '{other}'")),
    };
    if kind == RunStepKind::Handover && evidence == RunStepEvidence::Screenshot {
        return Err("a handover step is the person's and is never screenshotted".into());
    }
    Ok(RunPlanStep {
        title,
        kind,
        evidence,
        note: text_arg(item, "note", MAX_TEXT_CHARS)?,
        status: RunStepStatus::Pending,
        result: None,
        files: Vec::new(),
    })
}

/// A file path the model links as evidence: relative, inside the work folder
fn evidence_path(path: &str) -> Result<String, String> {
    let path = path.trim().replace('\\', "/");
    let path = path.trim_start_matches("./");
    let path = path.strip_prefix("/agent/workdir/").unwrap_or(path);
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(':')
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return Err(format!(
            "'{path}' isn't a file in your work folder (e.g. browser/<run>/1-1.png)"
        ));
    }
    Ok(path.to_string())
}

/// Applies `plan_update`'s arguments to `plan`
pub fn apply_update(plan: &mut RunPlan, args: &Value) -> Result<(), String> {
    let number = args
        .get("step")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| (1..=plan.steps.len()).contains(n))
        .ok_or_else(|| {
            format!(
                "`step` must be a step number from 1 to {}",
                plan.steps.len()
            )
        })?;
    let status = match args.get("status").and_then(Value::as_str) {
        Some(text) => Some(text.parse().map_err(|_| {
            format!(
                "`status` must be one of {}",
                RunStepStatus::VARIANTS.join(", ")
            )
        })?),
        None => None,
    };
    let result = text_arg(args, "result", MAX_TEXT_CHARS)?;
    let files = match args.get("files") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .ok_or_else(|| "`files` must be a list of file names".to_string())
                    .and_then(evidence_path)
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("`files` must be a list of file names".into()),
    };
    if status.is_none() && result.is_none() && files.is_empty() {
        return Err("Give a `status`, a `result` or `files` to update".into());
    }
    if status == Some(RunStepStatus::Doing) {
        if let Some((other, _)) = plan.current().filter(|(other, _)| *other != number) {
            return Err(format!(
                "Step {other} is still doing: mark it done, skipped or failed first"
            ));
        }
    }
    let step = &mut plan.steps[number - 1];
    if step.kind == RunStepKind::Handover && !files.is_empty() {
        return Err("A handover step is the person's: it has no screenshots".into());
    }
    if let Some(status) = status {
        step.status = status;
    }
    if result.is_some() {
        step.result = result;
    }
    for file in files {
        if !step.files.contains(&file) {
            step.files.push(file);
        }
    }
    if step.files.len() > MAX_FILES {
        return Err(format!("A step links at most {MAX_FILES} files"));
    }
    Ok(())
}

/// Links an evidence file to the current step, if there is one and it
/// isn't linked yet. Returns whether the plan changed.
pub fn link_to_current(plan: &mut RunPlan, file: &str) -> bool {
    let Some(index) = plan
        .steps
        .iter()
        .position(|step| step.status == RunStepStatus::Doing)
    else {
        return false;
    };
    let step = &mut plan.steps[index];
    if step.kind == RunStepKind::Handover
        || step.files.iter().any(|f| f == file)
        || step.files.len() >= MAX_FILES
    {
        return false;
    }
    step.files.push(file.to_string());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan(steps: Value) -> RunPlan {
        RunPlan {
            run_id: "r1".into(),
            revision: 1,
            steps: parse_steps(&json!({ "steps": steps })).unwrap(),
            updated_at: 0.0,
        }
    }

    fn bill() -> RunPlan {
        plan(json!([
            { "title": "Open acme-energy.com", "kind": "navigate", "evidence": "screenshot" },
            { "title": "Sign in", "kind": "handover" },
            { "title": "Enter $84", "kind": "fill", "note": "From the bill" },
            { "title": "Pay", "kind": "handover" },
            { "title": "Close the browser", "kind": "cleanup" },
        ]))
    }

    #[test]
    fn an_unknown_kind_or_status_lists_the_names_the_model_may_use() {
        let error =
            parse_steps(&json!({ "steps": [{ "title": "Go", "kind": "teleport" }] })).unwrap_err();
        assert!(error.ends_with(
            "`kind` must be one of navigate, read, fill, submit, handover, approval, \
             delegate, verify, cleanup, other"
        ));
        let mut plan = bill();
        let error =
            apply_update(&mut plan, &json!({ "step": 1, "status": "finished" })).unwrap_err();
        assert_eq!(
            error,
            "`status` must be one of pending, doing, done, skipped, failed"
        );
    }

    #[test]
    fn plan_set_parses_every_step_as_pending() {
        let plan = bill();
        assert_eq!(plan.steps.len(), 5);
        assert!(plan
            .steps
            .iter()
            .all(|s| s.status == RunStepStatus::Pending));
        assert_eq!(plan.steps[0].evidence, RunStepEvidence::Screenshot);
        assert_eq!(plan.steps[1].kind, RunStepKind::Handover);
        assert_eq!(plan.steps[2].note.as_deref(), Some("From the bill"));
        assert_eq!(plan.open_steps(), [1, 2, 3, 4, 5]);
        assert!(plan.current().is_none());
    }

    #[test]
    fn plan_set_refuses_bad_steps() {
        let err = |steps: Value| parse_steps(&json!({ "steps": steps })).unwrap_err();
        assert!(err(json!([])).contains("at least one"));
        assert!(err(json!([{ "kind": "read" }])).contains("`title`"));
        assert!(err(json!([{ "title": "x", "kind": "fly" }])).contains("`kind` must be"));
        assert!(
            err(json!([{ "title": "x", "kind": "read", "evidence": "video" }]))
                .contains("evidence")
        );
        assert!(
            err(json!([{ "title": "x", "kind": "handover", "evidence": "screenshot" }]))
                .contains("never screenshotted")
        );
        let many: Vec<Value> = (0..=MAX_STEPS)
            .map(|i| json!({ "title": format!("s{i}"), "kind": "other" }))
            .collect();
        assert!(err(Value::Array(many)).contains("at most"));
        assert!(parse_steps(&json!({})).is_err());
    }

    #[test]
    fn plan_update_moves_one_step_at_a_time() {
        let mut plan = bill();
        apply_update(&mut plan, &json!({ "step": 1, "status": "doing" })).unwrap();
        assert_eq!(plan.current().map(|(n, _)| n), Some(1));
        let err = apply_update(&mut plan, &json!({ "step": 2, "status": "doing" })).unwrap_err();
        assert!(err.contains("Step 1 is still doing"), "{err}");
        apply_update(
            &mut plan,
            &json!({ "step": 1, "status": "done", "result": "Opened", "files": ["browser/r1/1-1.png", "./browser/r1/1-1.png"] }),
        )
        .unwrap();
        assert_eq!(plan.steps[0].files, ["browser/r1/1-1.png"]);
        assert_eq!(plan.steps[0].result.as_deref(), Some("Opened"));
        assert!(plan.current().is_none());
        apply_update(&mut plan, &json!({ "step": 2, "status": "doing" })).unwrap();
        assert_eq!(
            plan.current().map(|(n, s)| (n, s.kind)),
            Some((2, RunStepKind::Handover))
        );
        // Re-marking the current step doing is fine
        apply_update(&mut plan, &json!({ "step": 2, "status": "doing" })).unwrap();
    }

    #[test]
    fn plan_update_refuses_bad_arguments() {
        let mut plan = bill();
        let mut err = |args: Value| apply_update(&mut plan, &args).unwrap_err();
        assert!(err(json!({ "step": 0, "status": "done" })).contains("from 1 to 5"));
        assert!(err(json!({ "step": 6, "status": "done" })).contains("from 1 to 5"));
        assert!(err(json!({ "step": 1, "status": "finished" })).contains("`status`"));
        assert!(err(json!({ "step": 1 })).contains("Give a"));
        assert!(err(json!({ "step": 1, "files": ["../secrets.txt"] })).contains("work folder"));
        assert!(err(json!({ "step": 1, "files": ["/etc/passwd"] })).contains("work folder"));
        assert!(err(json!({ "step": 1, "files": ["C:/x.png"] })).contains("work folder"));
        assert!(err(json!({ "step": 2, "files": ["browser/r1/2-1.png"] })).contains("handover"));
    }

    #[test]
    fn the_finish_check_sees_open_steps_and_missing_evidence() {
        let mut plan = bill();
        for (step, status) in [(1, "done"), (2, "done"), (3, "skipped"), (4, "failed")] {
            apply_update(&mut plan, &json!({ "step": step, "status": status })).unwrap();
        }
        assert_eq!(plan.open_steps(), [5]);
        assert_eq!(plan.missing_evidence(), [1]);
        apply_update(
            &mut plan,
            &json!({ "step": 1, "files": ["browser/r1/1-1.png"] }),
        )
        .unwrap();
        apply_update(&mut plan, &json!({ "step": 5, "status": "done" })).unwrap();
        assert!(plan.open_steps().is_empty());
        assert!(plan.missing_evidence().is_empty());
    }

    #[test]
    fn screenshots_link_to_the_current_step_but_never_a_handover() {
        let mut plan = bill();
        assert!(!link_to_current(&mut plan, "browser/r1/0-1.png"));
        apply_update(&mut plan, &json!({ "step": 1, "status": "doing" })).unwrap();
        assert!(link_to_current(&mut plan, "browser/r1/1-1.png"));
        assert!(!link_to_current(&mut plan, "browser/r1/1-1.png"));
        apply_update(&mut plan, &json!({ "step": 1, "status": "done" })).unwrap();
        apply_update(&mut plan, &json!({ "step": 2, "status": "doing" })).unwrap();
        assert!(!link_to_current(&mut plan, "browser/r1/2-1.png"));
        assert_eq!(plan.steps[0].files, ["browser/r1/1-1.png"]);
    }

    #[test]
    fn the_checklist_reads_back_each_step() {
        let mut plan = bill();
        apply_update(
            &mut plan,
            &json!({ "step": 1, "status": "done", "result": "Opened", "files": ["browser/r1/1-1.png"] }),
        )
        .unwrap();
        let text = plan.checklist();
        assert!(text.starts_with("Run plan (revision 1):"), "{text}");
        assert!(
            text.contains("1. [done] Open acme-energy.com (navigate), screenshot: Opened [browser/r1/1-1.png]"),
            "{text}"
        );
        assert!(text.contains("2. [pending] Sign in (handover)"), "{text}");
    }
}
