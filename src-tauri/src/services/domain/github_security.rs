//! The GitHub security check's rules (`services::github_security`): what
//! the reader saw on GitHub (settings and metadata only, as GitHub's JSON)
//! becomes a list of checks, each passed, failed (one plain sentence of
//! what to fix, with the settings link) or unknown, and the checks become
//! steps on the user's `check` to-dos. Pure: no I/O.
//!
//! | Source ref                            | Fails when                                              |
//! | ------------------------------------- | ------------------------------------------------------- |
//! | `github:user:2fa`                     | the account has two-factor sign-in off                  |
//! | `github:repo:<owner>/<repo>:public-backup` | the Backup remote is a public github.com repository |
//! | `github:org:<org>:2fa`                | an org you own doesn't require two-factor sign-in       |
//! | `github:org:<org>:secret-scanning`    | its public repos have secret scanning or push protection off |
//! | `github:org:<org>:branch-protection`  | its public repos' default branch allows force-push or deletion |
//! | `github:org:<org>:unreadable`         | GitHub hid the org's settings (the org limits apps)     |
//! | `github:scope:read`                   | the sign-in lacks `read:org` / `read:user`: sign in again |
//!
//! A field GitHub didn't show is never guessed: that check is unknown, and
//! an unknown check leaves its to-do as it is.

use serde_json::Value;

use super::todo::MAX_TEXT_CHARS;

/// How many repository names a to-do lists before "and N more"
const NAMES_SHOWN: usize = 3;

/// The scopes the check needs on top of `repo`, and the wider ones that
/// include them
const ORG_SCOPES: &[&str] = &["read:org", "write:org", "admin:org"];
const USER_SCOPES: &[&str] = &["read:user", "user"];

/// The start of the note Nandi closes a fixed finding's to-do with
pub const FIXED_NOTE_PREFIX: &str = "Fixed: checked again on";

/// What one API read gave
#[derive(Debug, Clone, PartialEq)]
pub enum Fetched<T = Value> {
    Got(T),
    /// GitHub said no (403 or 404): the sign-in may not see it
    Hidden,
    /// Not read: GitHub unreachable, rate-limited, or the call budget spent
    Unread,
}

impl<T> Fetched<T> {
    pub fn got(&self) -> Option<&T> {
        match self {
            Fetched::Got(value) => Some(value),
            _ => None,
        }
    }
}

/// Everything the reader saw, for [`evaluate`]
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// The sign-in's scopes (`X-OAuth-Scopes`); `None` when GitHub didn't say
    pub scopes: Option<Vec<String>>,
    /// `GET /user`
    pub user: Fetched,
    /// The Backup remote, when it is a github.com repository
    pub backup: Option<BackupSnapshot>,
    /// The orgs the user is an owner of (`GET /user/memberships/orgs`)
    pub orgs: Fetched<Vec<OrgSnapshot>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackupSnapshot {
    pub owner: String,
    pub repo: String,
    /// `GET /repos/{owner}/{repo}`
    pub info: Fetched,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrgSnapshot {
    pub login: String,
    /// `GET /orgs/{org}`
    pub info: Fetched,
    /// Its public, not archived repositories (`GET /orgs/{org}/repos`), at
    /// most the reader's cap
    pub repos: Fetched<Vec<RepoSnapshot>>,
    /// Public repositories past the cap, not read ("and N more")
    pub unread_repos: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepoSnapshot {
    /// The repository as `GET /orgs/{org}/repos` lists it
    pub info: Value,
    /// `GET /repos/{o}/{r}/branches/{default}` (`protected`)
    pub branch: Fetched,
    /// `GET /repos/{o}/{r}/rules/branches/{default}`: the rules that apply
    pub rules: Fetched,
}

/// How one check came out
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    /// What's wrong and what to do, one sentence with the settings link
    Fail(String),
    /// GitHub didn't show enough to tell
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// The to-do's stable `source_ref`
    pub source_ref: String,
    pub outcome: Outcome,
}

impl Check {
    fn new(source_ref: String, outcome: Outcome) -> Self {
        Self {
            source_ref,
            outcome,
        }
    }
}

/// A check's results
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckRun {
    pub checks: Vec<Check>,
    /// Ref prefixes this check saw in full: an open to-do under one that no
    /// check named (an org you no longer own, a Backup remote you changed)
    /// no longer applies
    pub settled: Vec<String>,
}

impl CheckRun {
    /// How many findings it has
    pub fn failed(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| matches!(check.outcome, Outcome::Fail(_)))
            .count()
    }
}

/// A GitHub login, org or repository name: the only text put in an API path
pub fn is_github_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A branch name safe to put in an API path (no `..`, no encoding needed)
pub fn is_branch_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.contains("..")
        && !name.starts_with('/')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// `(owner, repo)` of a github.com remote: `https://github.com/o/r(.git)`,
/// `git@github.com:o/r.git` or `ssh://git@github.com/o/r.git`
pub fn github_repo_of(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let path = [
        "git@github.com:",
        "ssh://git@github.com/",
        "https://github.com/",
        "https://www.github.com/",
    ]
    .iter()
    .find_map(|prefix| url.strip_prefix(prefix))?;
    let (owner, repo) = path.trim_end_matches('/').split_once('/')?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    (is_github_name(owner) && is_github_name(repo)).then(|| (owner.to_string(), repo.to_string()))
}

fn bool_at(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn has_any(scopes: &[String], wanted: &[&str]) -> bool {
    scopes.iter().any(|scope| wanted.contains(&scope.as_str()))
}

/// Whether the sign-in may read org and account settings; `None` when
/// GitHub didn't list its scopes
fn scopes_ok(snapshot: &Snapshot) -> Option<bool> {
    let scopes = snapshot.scopes.as_ref()?;
    Some(has_any(scopes, ORG_SCOPES) && has_any(scopes, USER_SCOPES))
}

fn user_2fa(user: &Fetched) -> Check {
    let outcome = match user
        .got()
        .and_then(|u| bool_at(u, "two_factor_authentication"))
    {
        Some(true) => Outcome::Pass,
        Some(false) => Outcome::Fail(
            "Turn on two-factor sign-in for your GitHub account: \
             https://github.com/settings/security"
                .to_string(),
        ),
        None => Outcome::Unknown,
    };
    Check::new("github:user:2fa".to_string(), outcome)
}

fn backup_public(backup: &BackupSnapshot) -> Check {
    let (owner, repo) = (&backup.owner, &backup.repo);
    let source_ref = format!(
        "github:repo:{}/{}:public-backup",
        owner.to_lowercase(),
        repo.to_lowercase()
    );
    let private = backup.info.got().and_then(|info| {
        bool_at(info, "private").or_else(|| {
            info.get("visibility")
                .and_then(Value::as_str)
                .map(|v| v != "public")
        })
    });
    let outcome = match private {
        Some(true) => Outcome::Pass,
        Some(false) => Outcome::Fail(format!(
            "Your Backup repository {owner}/{repo} on GitHub is public: make it private \
             under Danger Zone at https://github.com/{owner}/{repo}/settings"
        )),
        None => Outcome::Unknown,
    };
    Check::new(source_ref, outcome)
}

fn read_scope(snapshot: &Snapshot) -> Check {
    let hidden = matches!(snapshot.orgs, Fetched::Hidden);
    let outcome = match (scopes_ok(snapshot), hidden) {
        (Some(false), _) | (None, true) => Outcome::Fail(
            "Couldn't check your GitHub organization settings: sign in to GitHub again in \
             Settings › Account to allow reading organization settings"
                .to_string(),
        ),
        (Some(true), _) | (None, false) => match snapshot.orgs {
            Fetched::Unread => Outcome::Unknown,
            _ => Outcome::Pass,
        },
    };
    Check::new("github:scope:read".to_string(), outcome)
}

/// "a, b, c and 4 more"
fn names(list: &[&str], more: usize) -> String {
    let shown = list.iter().take(NAMES_SHOWN).copied().collect::<Vec<_>>();
    let more = more + list.len().saturating_sub(NAMES_SHOWN);
    match (shown.split_last(), more) {
        (None, _) => String::new(),
        (Some((last, [])), 0) => (*last).to_string(),
        (Some((last, rest)), 0) => format!("{} and {last}", rest.join(", ")),
        (Some(_), more) => format!("{} and {more} more", shown.join(", ")),
    }
}

/// `with_names`, or `without` when the names would make it too long
fn fit(with_names: String, without: String) -> String {
    if with_names.chars().count() <= MAX_TEXT_CHARS {
        with_names
    } else {
        without
    }
}

fn repo_name(repo: &RepoSnapshot) -> &str {
    repo.info
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// A repo's secret scanning and push protection both on; `None` when
/// GitHub didn't show them
fn scanning_on(repo: &RepoSnapshot) -> Option<bool> {
    let security = repo.info.get("security_and_analysis")?;
    let status = |key: &str| {
        security
            .get(key)
            .and_then(|s| s.get("status"))
            .and_then(Value::as_str)
            .map(|s| s == "enabled")
    };
    Some(status("secret_scanning")? && status("secret_scanning_push_protection")?)
}

/// A repo's default branch refuses force-push and deletion: classic
/// protection, or rules (the repo's or the org's rulesets) with both
/// `non_fast_forward` and `deletion`
fn branch_guarded(repo: &RepoSnapshot) -> Option<bool> {
    let protected = repo.branch.got().and_then(|b| bool_at(b, "protected"));
    if protected == Some(true) {
        return Some(true);
    }
    let rules = repo.rules.got()?.as_array()?;
    let has = |kind: &str| {
        rules
            .iter()
            .any(|rule| rule.get("type").and_then(Value::as_str) == Some(kind))
    };
    if has("non_fast_forward") && has("deletion") {
        return Some(true);
    }
    protected.map(|_| false)
}

/// The org's public repositories that fail `rule`; `None` when none fail
/// and some couldn't be told
fn failing(
    repos: &[RepoSnapshot],
    rule: impl Fn(&RepoSnapshot) -> Option<bool>,
) -> Option<Vec<&str>> {
    let mut unknown = false;
    let mut failed = Vec::new();
    for repo in repos {
        match rule(repo) {
            Some(true) => {}
            Some(false) => failed.push(repo_name(repo)),
            None => unknown = true,
        }
    }
    (!failed.is_empty() || !unknown).then_some(failed)
}

fn repo_count(count: usize) -> String {
    if count == 1 {
        "1 public repository".to_string()
    } else {
        format!("{count} public repositories")
    }
}

fn org_checks(org: &OrgSnapshot, scopes_ok: Option<bool>) -> Vec<Check> {
    let login = &org.login;
    let key = login.to_lowercase();
    let r = |name: &str| format!("github:org:{key}:{name}");
    let two_factor = org
        .info
        .got()
        .and_then(|info| bool_at(info, "two_factor_requirement_enabled"));
    let mut checks = vec![Check::new(
        r("2fa"),
        match two_factor {
            Some(true) => Outcome::Pass,
            Some(false) => Outcome::Fail(format!(
                "Require two-factor sign-in for everyone in the GitHub organization {login}: \
                 https://github.com/organizations/{login}/settings/authentication_security"
            )),
            None => Outcome::Unknown,
        },
    )];
    // Hidden with the right scopes: the org limits third-party apps
    let hidden =
        matches!(org.info, Fetched::Hidden) || (org.info.got().is_some() && two_factor.is_none());
    checks.push(Check::new(
        r("unreadable"),
        match (hidden, scopes_ok) {
            (true, Some(true) | None) => Outcome::Fail(format!(
                "Couldn't read the settings of the GitHub organization {login}: if it limits \
                 third-party apps, approve Nandi at \
                 https://github.com/organizations/{login}/settings/oauth_application_policy"
            )),
            (true, Some(false)) => Outcome::Unknown,
            (false, _) => match org.info {
                Fetched::Unread => Outcome::Unknown,
                _ => Outcome::Pass,
            },
        },
    ));

    let Fetched::Got(repos) = &org.repos else {
        checks.push(Check::new(r("secret-scanning"), Outcome::Unknown));
        checks.push(Check::new(r("branch-protection"), Outcome::Unknown));
        return checks;
    };
    let unchecked = org.unread_repos;

    let scanning = match failing(repos, scanning_on) {
        None => Outcome::Unknown,
        Some(off) if off.is_empty() => Outcome::Pass,
        Some(off) => {
            let link =
                format!("https://github.com/organizations/{login}/settings/security_analysis");
            let count = repo_count(off.len());
            Outcome::Fail(fit(
                format!(
                    "Turn on secret scanning and push protection for {count} in {login} \
                     ({}): {link}",
                    names(&off, unchecked)
                ),
                format!(
                    "Turn on secret scanning and push protection for {count} in {login}: {link}"
                ),
            ))
        }
    };
    checks.push(Check::new(r("secret-scanning"), scanning));

    let branches = match failing(repos, branch_guarded) {
        None => Outcome::Unknown,
        Some(open) if open.is_empty() => Outcome::Pass,
        Some(open) => {
            let link = format!("https://github.com/organizations/{login}/settings/rules");
            let count = repo_count(open.len());
            Outcome::Fail(fit(
                format!(
                    "Protect the default branch of {count} in {login} ({}) from force-push \
                     and deletion with a branch ruleset: {link}",
                    names(&open, unchecked)
                ),
                format!(
                    "Protect the default branch of {count} in {login} from force-push and \
                     deletion with a branch ruleset: {link}"
                ),
            ))
        }
    };
    checks.push(Check::new(r("branch-protection"), branches));
    checks
}

/// The checks for what the reader saw
pub fn evaluate(snapshot: &Snapshot) -> CheckRun {
    let mut run = CheckRun {
        checks: vec![user_2fa(&snapshot.user), read_scope(snapshot)],
        // The Backup remote is read from config.json every time
        settled: vec!["github:repo:".to_string()],
    };
    if let Some(backup) = &snapshot.backup {
        run.checks.push(backup_public(backup));
    }
    if let Fetched::Got(orgs) = &snapshot.orgs {
        run.settled.push("github:org:".to_string());
        let scopes_ok = scopes_ok(snapshot);
        for org in orgs {
            run.checks.extend(org_checks(org, scopes_ok));
        }
    }
    run
}

// -- To-dos ------------------------------------------------------------------

/// A security-check to-do already there
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingCheck {
    pub task_id: String,
    pub source_ref: String,
    pub open: bool,
    /// Its note: Nandi's "Fixed: …", or the user's
    pub note: Option<String>,
}

impl ExistingCheck {
    /// Nandi closed it as fixed (not the user by hand)
    fn closed_as_fixed(&self) -> bool {
        !self.open
            && self
                .note
                .as_deref()
                .is_some_and(|n| n.starts_with(FIXED_NOTE_PREFIX))
    }
}

/// What to do to the to-dos
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A new finding: a `check` to-do
    Add { source_ref: String, text: String },
    /// Fixed, or it no longer applies: close it with [`fixed_note`]
    Close { task_id: String },
    /// Nandi closed it as fixed and it is back
    Reopen { task_id: String },
}

/// "Fixed: checked again on 2026-11-20"
pub fn fixed_note(date: chrono::NaiveDate) -> String {
    format!("{FIXED_NOTE_PREFIX} {}", date.format("%Y-%m-%d"))
}

/// The steps that bring the to-dos in line with `run`: a finding opens its
/// to-do once (a re-check adds none), a passed check closes its open to-do,
/// an unknown one changes nothing. A finding the user closed by hand stays
/// closed; one Nandi closed as fixed opens again when it comes back.
pub fn plan(run: &CheckRun, existing: &[ExistingCheck]) -> Vec<Step> {
    let find = |source_ref: &str| existing.iter().find(|e| e.source_ref == source_ref);
    let mut steps = Vec::new();
    for check in &run.checks {
        let todo = find(&check.source_ref);
        match (&check.outcome, todo) {
            (Outcome::Fail(text), None) => steps.push(Step::Add {
                source_ref: check.source_ref.clone(),
                text: text.clone(),
            }),
            (Outcome::Fail(_), Some(todo)) if todo.closed_as_fixed() => steps.push(Step::Reopen {
                task_id: todo.task_id.clone(),
            }),
            (Outcome::Pass, Some(todo)) if todo.open => steps.push(Step::Close {
                task_id: todo.task_id.clone(),
            }),
            _ => {}
        }
    }
    for todo in existing.iter().filter(|todo| todo.open) {
        let named = run.checks.iter().any(|c| c.source_ref == todo.source_ref);
        let settled = run.settled.iter().any(|p| todo.source_ref.starts_with(p));
        if !named && settled {
            steps.push(Step::Close {
                task_id: todo.task_id.clone(),
            });
        }
    }
    steps
}

#[cfg(test)]
#[path = "github_security_tests.rs"]
mod tests;
