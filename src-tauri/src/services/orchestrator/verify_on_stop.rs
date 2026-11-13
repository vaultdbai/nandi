//! Verify-on-stop.
//!
//! When a step would end the run with text only, after files were written
//! and nothing since checked them (tests, a type check, re-reading the
//! file), the run gets one continuation asking the agent to verify. A
//! heuristic; firings are logged to `~/.nandi/local/logs/verify.log`.
//!
//! The heuristic, in `post_turn`:
//!
//! - The step is text-only (`ExitReason::Completed`), the run wrote files
//!   (`TurnContext::files_touched`), and verify-on-stop hasn't fired yet in
//!   this run. It fires at most once: if the agent ends again, it ends.
//! - Evidence is a tool result in the last `EVIDENCE_TURNS` steps, after the
//!   last write in `last_tool_results`: a shell-like call whose command
//!   looks like a check (`pytest`, `cargo test`, `cargo check`, `npm test`,
//!   `pnpm test`, `yarn test`, `jest`, `vitest`, `go test`, `make`, `tsc`,
//!   or anything with `--check`), or a read of a modified file.
//! - Without evidence, `continue_with` asks the agent to verify, and the
//!   firing is appended to `verify.log` as one JSON line.
//!
//! Writes into the user's notes (`my-notes/`, Chief of Staff's) aren't
//! code and never need a check.
//!
//! It can't tell whether the right checks ran, only that some did; skill.md
//! `middlewares: []` (or a list without `verify_on_stop`) turns it off.

use std::io::Write;
use std::path::{Path, PathBuf};

use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;

use super::middleware::{
    ExitReason, MiddlewareDecision, MiddlewareError, ToolResultSummary, TurnContext,
    TurnMiddleware, TurnResult,
};
use crate::db;
use crate::services::notes::NOTES_FOLDER_ID;
use crate::services::run_analysis::written_path;
use crate::utils::nandi_config;

/// How many recent steps count as verification evidence
pub const EVIDENCE_TURNS: u32 = 3;

/// The log file under `nandi_config::logs_dir()`
pub const LOG_FILE: &str = "verify.log";

const REASON: &str = "completed with unverified file changes";

/// Commands that count as a check (matched as whole words or phrases)
const CHECK_COMMANDS: &[&str] = &[
    "pytest",
    "cargo test",
    "cargo check",
    "cargo clippy",
    "npm test",
    "npm run test",
    "pnpm test",
    "pnpm run test",
    "yarn test",
    "jest",
    "vitest",
    "go test",
    "make",
    "tsc",
];

/// Argument keys that hold a shell command
const COMMAND_KEYS: &[&str] = &["command", "cmd", "script", "args"];
/// Argument keys that hold a file path
const PATH_KEYS: &[&str] = &["path", "file_path", "filePath", "file", "filename"];

#[derive(Default)]
pub struct VerifyOnStopMiddleware {
    /// Where `verify.log` goes; `None`: `nandi_config::logs_dir()`
    pub log_dir: Option<PathBuf>,
}

impl VerifyOnStopMiddleware {
    #[cfg(test)]
    pub fn with_log_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.log_dir = Some(dir.into());
        self
    }

    fn log_path(&self) -> Result<PathBuf, String> {
        let dir = match &self.log_dir {
            Some(dir) => dir.clone(),
            None => nandi_config::logs_dir()?,
        };
        Ok(dir.join(LOG_FILE))
    }
}

/// One `verify.log` line
#[derive(Debug, Serialize)]
struct VerifyLogEntry<'a> {
    agent_id: &'a str,
    run_id: &'a str,
    turn: u32,
    reason: &'a str,
    files: &'a [String],
    /// ms since the Unix epoch
    timestamp: i64,
}

fn append_log(path: &Path, entry: &VerifyLogEntry<'_>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("Couldn't open {}: {e}", path.display()))?;
    writeln!(file, "{line}").map_err(|e| format!("Couldn't write {}: {e}", path.display()))
}

/// The continuation sent when the run would end unverified
pub fn verify_prompt(files: &[String]) -> String {
    format!(
        "Before responding, verify your changes. You modified {}. Run the relevant checks.",
        files.join(", ")
    )
}

/// The files the run wrote, once each, in order, leaving out the notes
fn modified_files(ctx: &TurnContext) -> Vec<String> {
    let notes = format!("{NOTES_FOLDER_ID}/");
    let mut files: Vec<String> = Vec::new();
    for file in &ctx.files_touched {
        if !normalize(file).starts_with(&notes) && !files.contains(file) {
            files.push(file.clone());
        }
    }
    files
}

/// Whether the step should get a verify continuation
pub fn needs_verification(ctx: &TurnContext, result: &TurnResult) -> bool {
    if ctx.verify_fired || result.exit_reason != ExitReason::Completed {
        return false;
    }
    let files = modified_files(ctx);
    !files.is_empty() && !has_evidence(ctx, &files)
}

/// A check or a re-read since the last write, within the last
/// `EVIDENCE_TURNS` steps
fn has_evidence(ctx: &TurnContext, files: &[String]) -> bool {
    let results = &ctx.last_tool_results;
    let after_last_write = results
        .iter()
        .rposition(|r| written_path(&r.tool, &r.args).is_some())
        .map_or(0, |index| index + 1);
    let oldest_turn = ctx.turn_number.saturating_sub(EVIDENCE_TURNS - 1);
    results[after_last_write..]
        .iter()
        .filter(|r| r.turn >= oldest_turn)
        .any(|r| is_check(r) || reads_modified(r, files))
}

fn is_shell_like(tool: &str) -> bool {
    let tool = tool.to_ascii_lowercase();
    ["sh", "cmd", "run"].contains(&tool.as_str())
        || ["shell", "bash", "exec", "command", "terminal"]
            .iter()
            .any(|word| tool.contains(word))
}

fn command_text(args: &Value) -> Option<String> {
    let text_of = |value: &Value| match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    };
    match args {
        Value::String(s) => Some(s.clone()),
        _ => COMMAND_KEYS
            .iter()
            .find_map(|key| args.get(key).and_then(text_of)),
    }
}

/// A shell-like call that runs a test, build, or type check
pub fn is_check(result: &ToolResultSummary) -> bool {
    if !is_shell_like(&result.tool) {
        return false;
    }
    command_text(&result.args).is_some_and(|command| looks_like_check(&command))
}

/// Whether a shell command looks like a check (a heuristic)
pub fn looks_like_check(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    if command.contains("--check") {
        return true;
    }
    // Words with paths and shell punctuation stripped: `./node_modules/.bin/tsc`
    // and `(cd app && make)` count
    let words: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || ";&|()`\"'".contains(c))
        .filter(|w| !w.is_empty())
        .map(|w| w.rsplit('/').next().unwrap_or(w))
        .collect();
    CHECK_COMMANDS.iter().any(|check| {
        let phrase: Vec<&str> = check.split(' ').collect();
        words
            .windows(phrase.len())
            .any(|window| window == phrase.as_slice())
    })
}

fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.trim_start_matches("./");
    path.trim_start_matches("/agent/workdir/").to_string()
}

fn same_file(a: &str, b: &str) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    a == b || a.ends_with(&format!("/{b}")) || b.ends_with(&format!("/{a}"))
}

/// A read of one of the modified files
fn reads_modified(result: &ToolResultSummary, files: &[String]) -> bool {
    let tool = result.tool.to_ascii_lowercase();
    if !(tool.contains("read") || tool == "cat" || tool.contains("view")) {
        return false;
    }
    PATH_KEYS
        .iter()
        .filter_map(|key| result.args.get(key).and_then(Value::as_str))
        .any(|path| files.iter().any(|file| same_file(path, file)))
}

impl VerifyOnStopMiddleware {
    fn check(&self, ctx: &mut TurnContext, result: &mut TurnResult) {
        if !needs_verification(ctx, result) {
            return;
        }
        let files = modified_files(ctx);
        result.continue_with = Some(verify_prompt(&files));
        ctx.verify_fired = true;
        let entry = VerifyLogEntry {
            agent_id: &ctx.agent_id,
            run_id: &ctx.run_id,
            turn: ctx.turn_number,
            reason: REASON,
            files: &files,
            timestamp: db::now_ms(),
        };
        log::info!(
            "Verify-on-stop fired for run {} (turn {}): {}",
            ctx.run_id,
            ctx.turn_number,
            files.join(", ")
        );
        if let Err(e) = self.log_path().and_then(|path| append_log(&path, &entry)) {
            log::warn!("Couldn't write the verify log: {e}");
        }
    }
}

impl TurnMiddleware for VerifyOnStopMiddleware {
    fn name(&self) -> &'static str {
        super::middleware::VERIFY_ON_STOP
    }

    fn pre_turn<'a>(
        &'a self,
        _ctx: &'a mut TurnContext,
    ) -> BoxFuture<'a, Result<MiddlewareDecision, MiddlewareError>> {
        Box::pin(async { Ok(MiddlewareDecision::Continue) })
    }

    fn post_turn<'a>(
        &'a self,
        ctx: &'a mut TurnContext,
        result: &'a mut TurnResult,
    ) -> BoxFuture<'a, Result<(), MiddlewareError>> {
        Box::pin(async move {
            self.check(ctx, result);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(turn: u32, name: &str, args: Value) -> ToolResultSummary {
        ToolResultSummary {
            turn,
            tool: name.into(),
            args,
            is_error: false,
        }
    }

    fn write(turn: u32, path: &str) -> ToolResultSummary {
        tool(turn, "write_file", json!({ "path": path }))
    }

    fn shell(turn: u32, command: &str) -> ToolResultSummary {
        tool(turn, "shell", json!({ "command": command }))
    }

    /// Step 5 is text-only after `results`
    fn ctx(results: Vec<ToolResultSummary>) -> TurnContext {
        let files_touched = results
            .iter()
            .filter_map(|r| written_path(&r.tool, &r.args))
            .collect();
        TurnContext {
            agent_id: "coder".into(),
            run_id: "r1".into(),
            turn_number: 5,
            last_tool_results: results,
            files_touched,
            ..TurnContext::default()
        }
    }

    fn completed() -> TurnResult {
        TurnResult {
            final_message: Some("Done!".into()),
            exit_reason: ExitReason::Completed,
            ..TurnResult::default()
        }
    }

    fn middleware(log_dir: &Path) -> VerifyOnStopMiddleware {
        VerifyOnStopMiddleware::default().with_log_dir(log_dir)
    }

    fn log_lines(dir: &Path) -> Vec<Value> {
        std::fs::read_to_string(dir.join(LOG_FILE))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn unverified_modifications_need_verification() {
        let context = ctx(vec![write(3, "src/lib.rs"), tool(4, "list_dir", json!({}))]);
        assert!(needs_verification(&context, &completed()));
    }

    #[test]
    fn notes_writes_need_no_check() {
        let context = ctx(vec![write(3, "my-notes/deadlines.md")]);
        assert!(!needs_verification(&context, &completed()));
        let both = ctx(vec![write(2, "my-notes/owners.md"), write(3, "src/lib.rs")]);
        assert!(
            needs_verification(&both, &completed()),
            "code still needs a check"
        );
    }

    #[test]
    fn a_check_command_after_the_write_is_evidence() {
        for command in [
            "cargo test --all",
            "cd app && pnpm test",
            "python -m pytest -q",
            "npx vitest run",
            "./node_modules/.bin/tsc --noEmit",
            "go test ./...",
            "make",
            "ruff format --check .",
            "cargo check",
        ] {
            let context = ctx(vec![write(3, "src/lib.rs"), shell(4, command)]);
            assert!(!needs_verification(&context, &completed()), "{command}");
        }
    }

    #[test]
    fn other_commands_are_not_evidence() {
        for command in ["ls -la", "git status", "cmake-format x", "echo maketh"] {
            let context = ctx(vec![write(3, "src/lib.rs"), shell(4, command)]);
            assert!(needs_verification(&context, &completed()), "{command}");
        }
    }

    #[test]
    fn re_reading_a_modified_file_is_evidence() {
        let context = ctx(vec![
            write(3, "src/lib.rs"),
            tool(4, "read_file", json!({ "path": "./src/lib.rs" })),
        ]);
        assert!(!needs_verification(&context, &completed()));

        let other = ctx(vec![
            write(3, "src/lib.rs"),
            tool(4, "read_file", json!({ "path": "src/main.rs" })),
        ]);
        assert!(needs_verification(&other, &completed()));
    }

    #[test]
    fn evidence_must_follow_the_last_write_and_be_recent() {
        // Tests ran, then another write
        let stale = ctx(vec![
            write(1, "a.rs"),
            shell(2, "cargo test"),
            write(4, "b.rs"),
        ]);
        assert!(needs_verification(&stale, &completed()));

        // Older than EVIDENCE_TURNS steps
        let mut old = ctx(vec![write(1, "a.rs"), shell(2, "cargo test")]);
        old.turn_number = 2 + EVIDENCE_TURNS;
        assert!(needs_verification(&old, &completed()));
        old.turn_number = 1 + EVIDENCE_TURNS;
        assert!(!needs_verification(&old, &completed()));
    }

    #[test]
    fn only_completed_steps_with_written_files_fire() {
        let mut tool_step = completed();
        tool_step.exit_reason = ExitReason::ToolCalls;
        assert!(!needs_verification(
            &ctx(vec![write(3, "a.rs")]),
            &tool_step
        ));
        assert!(!needs_verification(
            &ctx(vec![tool(3, "list_dir", json!({}))]),
            &completed()
        ));

        let mut fired = ctx(vec![write(3, "a.rs")]);
        fired.verify_fired = true;
        assert!(!needs_verification(&fired, &completed()));
    }

    #[tokio::test]
    async fn fires_once_per_run_and_logs_each_firing() {
        let logs = tempfile::tempdir().unwrap();
        let mw = middleware(logs.path());
        let mut context = ctx(vec![write(3, "src/lib.rs"), write(4, "src/main.rs")]);

        let mut first = completed();
        mw.post_turn(&mut context, &mut first).await.unwrap();
        assert_eq!(
            first.continue_with.as_deref(),
            Some(
                "Before responding, verify your changes. You modified src/lib.rs, src/main.rs. Run the relevant checks."
            )
        );
        assert!(context.verify_fired);

        // The agent ends again without checking: this time the run ends
        context.turn_number += 1;
        let mut second = completed();
        mw.post_turn(&mut context, &mut second).await.unwrap();
        assert_eq!(second.continue_with, None);

        let lines = log_lines(logs.path());
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line["agent_id"], json!("coder"));
        assert_eq!(line["run_id"], json!("r1"));
        assert_eq!(line["turn"], json!(5));
        assert_eq!(line["files"], json!(["src/lib.rs", "src/main.rs"]));
        assert_eq!(line["reason"], json!(REASON));
        assert!(line["timestamp"].as_i64().unwrap() > 0);
    }

    #[tokio::test]
    async fn verified_runs_end_without_a_continuation_or_log() {
        let logs = tempfile::tempdir().unwrap();
        let mw = middleware(logs.path());
        let mut context = ctx(vec![write(3, "src/lib.rs"), shell(4, "cargo test")]);
        let mut result = completed();
        mw.post_turn(&mut context, &mut result).await.unwrap();
        assert_eq!(result.continue_with, None);
        assert!(!context.verify_fired);
        assert!(log_lines(logs.path()).is_empty());
    }

    #[test]
    fn the_default_pipeline_includes_it_unless_skill_md_opts_out() {
        use super::super::middleware::{TurnPolicy, VERIFY_ON_STOP};
        assert!(TurnPolicy::default().enables(VERIFY_ON_STOP));
        let off = TurnPolicy {
            middlewares: Some(Vec::new()),
            ..TurnPolicy::default()
        };
        assert!(!off.enables(VERIFY_ON_STOP));
    }
}
