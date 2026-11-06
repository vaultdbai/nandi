//! The repo file watcher: keeps the running app in step with the gold copy
//! when files change outside it (an editor, a script, a `git pull`).
//!
//! `notify` watches the repo recursively. Events are batched until the files have been
//! quiet for `DEBOUNCE` (at most `MAX_DELAY` after the first), mapped from
//! paths to entities (`classify`), deduplicated, and each entity is
//! reloaded through the `RepoStore`:
//!
//! - valid: the aggregate and projection update and `repo://<kind>.*`
//!   fires (an app's own write has the same hash and is a no-op)
//! - invalid: `repo://validation_failed`; the last good version stays
//!
//! Capability files (`.nandi/capabilities/`, wave A) aren't `RepoStore`
//! entities: their paths go to the handler the capability service
//! registers for the repo (`set_capability_handler`).
//!
//! Policy files (`.nandi/policies/<id>.md`) aren't either: they're read
//! from disk on every run (`services::policies`), so a change is only
//! checked and reported. A broken one emits `repo://validation_failed`
//! and blocks the agents it may cover until it's fixed (fail closed).
//!
//! Every reloaded entity and changed policy also emits
//! `repo://file_changed`. Ignored: `.git/`, hidden and editor temp files
//! (`.*.swp`, `*.tmp`, `*~`), the app's own atomic-write temp files, and
//! workflow run artifacts (`runs/`).
//!
//! Every change also goes to the change feed (`services::live`): entities
//! through the store, policies from here, and two files outside the store
//! from here only: `installed.yaml` (the installed packs) and the repo
//! root's `NANDI.md` (the user's profile).
//!
//! When the OS may have dropped events (it set the rescan flag, the
//! watcher reported an error, or more than [`BURST_LIMIT`] paths changed
//! in one batch, which is when a Windows change buffer overflows without
//! saying so) the whole repo is re-scanned and the log says why.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use crate::services::domain::{ChangedBy, EntityKind};
use crate::services::events::{self, EventBus, RepoFileChanged, RepoValidationFailed};
use crate::services::gateway::manifest::MANIFEST_FILE as PROFILE_FILE;
use crate::services::live::{ChangeArea, ChangeFeed};
use crate::services::policies::{self, PolicyFile};
use crate::services::repo_store::{Reload, RepoStore};
use crate::services::repos::fs_atomic::TEMP_MARKER;
use crate::services::repos::layout::{INSTALLED_FILE, MARKETPLACE_DIR, OVERRIDES_DIR, PACKS_DIR};
use crate::services::repos::RepoLayout;

/// Quiet time before a batch of changes is applied
pub const DEBOUNCE: Duration = Duration::from_millis(500);
/// A steady stream of changes is still applied this often
const MAX_DELAY: Duration = Duration::from_secs(2);
/// More changed paths than this in one batch: the OS may have dropped
/// some (a Windows change buffer overflows silently), so re-scan
pub const BURST_LIMIT: usize = 512;

/// What a notify callback hands the batching loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// A path changed; `created` when the OS said it was created
    Path { path: PathBuf, created: bool },
    /// Events may have been lost: re-scan (the reason goes in the log)
    Rescan(String),
}

/// The signals one notify callback result carries
pub fn signals(result: notify::Result<notify::Event>) -> Vec<Signal> {
    match result {
        Ok(event) if event.need_rescan() => {
            vec![Signal::Rescan("the OS dropped file events".to_string())]
        }
        Ok(event) if event.kind.is_access() => Vec::new(),
        Ok(event) => {
            let created = event.kind.is_create();
            event
                .paths
                .into_iter()
                .map(|path| Signal::Path { path, created })
                .collect()
        }
        Err(e) => vec![Signal::Rescan(format!("the file watcher failed: {e}"))],
    }
}

/// Signals that arrived close together (see [`next_batch`])
#[derive(Debug, Default)]
pub struct Batch {
    /// Each changed path, and whether the OS said it was created
    pub paths: HashMap<PathBuf, bool>,
    /// Why to re-scan, when events may have been lost
    pub rescan: Option<String>,
}

impl Batch {
    pub fn add(&mut self, signal: Signal) {
        match signal {
            Signal::Path { path, created } => {
                *self.paths.entry(path).or_default() |= created;
            }
            Signal::Rescan(why) => self.rescan = Some(why),
        }
        if self.rescan.is_none() && self.paths.len() > BURST_LIMIT {
            self.rescan = Some(format!("more than {BURST_LIMIT} files changed at once"));
        }
    }
}

/// Waits for the next batch: from the first signal until the files have
/// been quiet for [`DEBOUNCE`] (at most [`MAX_DELAY`]). `None` once the
/// watcher is gone.
pub async fn next_batch(rx: &mut mpsc::UnboundedReceiver<Signal>) -> Option<Batch> {
    let mut batch = Batch::default();
    batch.add(rx.recv().await?);
    let started = Instant::now();
    loop {
        let quiet = tokio::time::sleep(DEBOUNCE);
        let deadline = tokio::time::sleep_until(started + MAX_DELAY);
        tokio::select! {
            next = rx.recv() => match next {
                Some(signal) => batch.add(signal),
                None => return Some(batch),
            },
            _ = quiet => return Some(batch),
            _ = deadline => return Some(batch),
        }
    }
}

/// A notify watcher that sends what each callback carries to `tx`
pub fn notify_watcher(tx: mpsc::UnboundedSender<Signal>) -> notify::Result<RecommendedWatcher> {
    notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        for signal in signals(result) {
            let _ = tx.send(signal);
        }
    })
}

/// What a changed path means.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Entity(EntityKind, String),
    /// A whole folder (or the manifest) changed: re-scan everything
    Everything,
    /// A path under `.nandi/capabilities/` (the folder itself included)
    Capability(PathBuf),
    /// `.nandi/policies/<id>.md`
    Policy(String),
    /// The `.nandi/policies/` folder itself (created, removed, replaced by
    /// a checkout): check every policy in it. On Linux a file written into
    /// a new folder can arrive before the folder is watched
    Policies,
    /// `.nandi/installed.yaml`: the installed packs (reported only)
    Packs,
    /// The repo root's `NANDI.md` (reported only; the gateway reads it)
    Profile,
}

/// Takes changed capability paths (wave A); see `set_capability_handler`
pub type CapabilityHandler = Arc<dyn Fn(Vec<PathBuf>) + Send + Sync>;

/// Repo root to its capability handler
static CAPABILITY_HANDLERS: LazyLock<Mutex<HashMap<PathBuf, CapabilityHandler>>> =
    LazyLock::new(Mutex::default);

/// Makes the watcher of the repo at `root` hand `.nandi/capabilities/`
/// changes to `handler` (replacing any earlier one)
pub fn set_capability_handler(root: &Path, handler: CapabilityHandler) {
    CAPABILITY_HANDLERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(root.to_path_buf(), handler);
}

fn capability_handler(root: &Path) -> Option<CapabilityHandler> {
    CAPABILITY_HANDLERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(root)
        .cloned()
}

/// A running watcher. Dropping it stops watching.
pub struct RepoWatcher {
    _watcher: RecommendedWatcher,
    task: AbortHandle,
}

impl Drop for RepoWatcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Area {
    Agents,
    Workflows,
    Triggers,
    Capabilities,
    Policies,
    Meta,
}

/// Maps paths to entities for one layout.
pub struct Classifier {
    /// Most specific first; each folder also under its canonical path
    areas: Vec<(PathBuf, Area)>,
    /// Single files (`installed.yaml`, `NANDI.md`), also canonical
    files: Vec<(PathBuf, Target)>,
}

impl Classifier {
    pub fn new(layout: &RepoLayout) -> Self {
        let mut areas = Vec::new();
        let capabilities_dir = layout.capabilities_dir();
        let policies_dir = layout.policies_dir();
        for (dir, area) in [
            (&layout.agents_dir, Area::Agents),
            (&layout.workflows_dir, Area::Workflows),
            (&layout.triggers_dir, Area::Triggers),
            (&capabilities_dir, Area::Capabilities),
            (&policies_dir, Area::Policies),
            (&layout.meta_dir, Area::Meta),
        ] {
            areas.push((dir.clone(), area));
            if let Ok(canonical) = dir.canonicalize() {
                if canonical != *dir {
                    areas.push((canonical, area));
                }
            }
        }
        let mut files = Vec::new();
        for (dir, name, target) in [
            (&layout.meta_dir, INSTALLED_FILE, Target::Packs),
            (&layout.root, PROFILE_FILE, Target::Profile),
        ] {
            files.push((dir.join(name), target.clone()));
            if let Ok(canonical) = dir.canonicalize() {
                if canonical != *dir {
                    files.push((canonical.join(name), target));
                }
            }
        }
        Self { areas, files }
    }

    pub fn classify(&self, path: &Path) -> Option<Target> {
        if let Some((_, target)) = self.files.iter().find(|(file, _)| file == path) {
            return Some(target.clone());
        }
        for (base, area) in &self.areas {
            let Ok(relative) = path.strip_prefix(base) else {
                continue;
            };
            let parts: Vec<String> = relative
                .components()
                .filter_map(|c| match c {
                    Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            if parts.iter().any(|part| ignored(part)) {
                return None;
            }
            if *area == Area::Capabilities {
                // Only the folder and the files directly in it
                return (parts.len() <= 1).then(|| Target::Capability(path.to_path_buf()));
            }
            return classify_in(*area, &parts);
        }
        None
    }
}

fn classify_in(area: Area, parts: &[String]) -> Option<Target> {
    let yaml_stem = |name: &str| {
        name.strip_suffix(".yaml")
            .or_else(|| name.strip_suffix(".yml"))
            .map(str::to_string)
    };
    match (area, parts) {
        // The folder itself (created, removed, replaced by a checkout)
        (Area::Agents | Area::Workflows | Area::Triggers, []) => Some(Target::Everything),
        (Area::Agents, [first, rest @ ..]) if first == MARKETPLACE_DIR => {
            classify_marketplace(rest)
        }
        (Area::Agents, [_, dir, file, ..]) if dir == "workflows" => {
            yaml_stem(file).map(|id| Target::Entity(EntityKind::Workflow, id))
        }
        (Area::Agents, [_, dir]) if dir == "workflows" => Some(Target::Everything),
        (Area::Agents, [id, ..]) => Some(Target::Entity(EntityKind::Agent, id.clone())),
        (Area::Workflows, [file]) => {
            yaml_stem(file).map(|id| Target::Entity(EntityKind::Workflow, id))
        }
        (Area::Triggers, [file]) => {
            yaml_stem(file).map(|id| Target::Entity(EntityKind::Trigger, id))
        }
        (Area::Policies, []) => Some(Target::Policies),
        (Area::Policies, [file]) => file
            .strip_suffix(".md")
            .map(|id| Target::Policy(id.to_string())),
        // An installed agent's own instructions (`agent_repo::save_instructions`)
        (Area::Meta, [dir, id, ..]) if dir == OVERRIDES_DIR => {
            Some(Target::Entity(EntityKind::Agent, id.clone()))
        }
        (Area::Meta, [file]) => match file.as_str() {
            "settings.yaml" => Some(Target::Entity(EntityKind::Settings, "settings".into())),
            "connectors.yaml" => Some(Target::Entity(EntityKind::Connectors, "connectors".into())),
            "manifest.yaml" => Some(Target::Everything),
            _ => None,
        },
        _ => None,
    }
}

/// Paths under `.nandi/agents/marketplace/`. A vendored folder appearing or
/// disappearing as a whole (an install, an uninstall, a manual clone)
/// re-scans everything; files inside map to their agent or workflow.
fn classify_marketplace(parts: &[String]) -> Option<Target> {
    let agent_files = |id: &String, rest: &[String]| match rest {
        [dir, file, ..] if dir == "workflows" => {
            yaml_stem(file).map(|wf| Target::Entity(EntityKind::Workflow, wf))
        }
        [dir] if dir == "workflows" => Some(Target::Everything),
        _ => Some(Target::Entity(EntityKind::Agent, id.clone())),
    };
    match parts {
        // marketplace/packs/<pack>/…
        [packs, rest @ ..] if packs == PACKS_DIR => match rest {
            // packs/, a pack folder, its agents/ or workflows/ folder, or an
            // agent folder inside it
            [] | [_] => Some(Target::Everything),
            [_, dir] if dir == "agents" || dir == "workflows" => Some(Target::Everything),
            [_, dir, _id] if dir == "agents" => Some(Target::Everything),
            [_, dir, id, more @ ..] if dir == "agents" => agent_files(id, more),
            [_, dir, file] if dir == "workflows" => {
                yaml_stem(file).map(|wf| Target::Entity(EntityKind::Workflow, wf))
            }
            _ => None,
        },
        // marketplace/, a publisher folder, or an agent folder
        [] | [_] | [_, _] => Some(Target::Everything),
        // marketplace/<publisher>/<id>/…
        [_publisher, id, more @ ..] => agent_files(id, more),
    }
}

fn yaml_stem(name: &str) -> Option<String> {
    name.strip_suffix(".yaml")
        .or_else(|| name.strip_suffix(".yml"))
        .map(str::to_string)
}

/// Paths the watcher never acts on.
pub(crate) fn ignored(part: &str) -> bool {
    part == ".git"
        || part == "runs"
        || part.contains(TEMP_MARKER)
        || part.ends_with(".tmp")
        || part.ends_with('~')
        || (part.starts_with('.') && (part.ends_with(".swp") || part.ends_with(".swx")))
        || part.starts_with(".#")
}

/// Starts watching the store's repo. A repo folder that doesn't exist yet
/// isn't watched; the next app start picks it up.
pub fn spawn(
    store: RepoStore,
    bus: EventBus,
    feed: ChangeFeed,
    runtime: &tokio::runtime::Handle,
) -> Result<RepoWatcher, String> {
    let layout = store.layout().clone();
    if !layout.root.is_dir() {
        return Err(format!("{} doesn't exist yet", layout.root.display()));
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<Signal>();
    let mut watcher =
        notify_watcher(tx).map_err(|e| format!("Couldn't create the repo watcher: {e}"))?;
    watcher
        .watch(&layout.root, RecursiveMode::Recursive)
        .map_err(|e| format!("Couldn't watch {}: {e}", layout.root.display()))?;
    log::info!("Watching {} for changes", layout.root.display());
    let classifier = Classifier::new(&layout);
    store.set_watching(true);

    let watched = store.clone();
    let task = runtime.spawn(async move {
        while let Some(batch) = next_batch(&mut rx).await {
            let targets = targets_of(&classifier, &batch, watched.layout());
            if let Some(why) = &batch.rescan {
                log::warn!("Re-scanning {}: {why}", watched.layout().root.display());
            }
            dispatch(&watched, &bus, &feed, targets).await;
        }
    });
    Ok(RepoWatcher {
        _watcher: watcher,
        task: task.abort_handle(),
    })
}

/// What a batch means. A batch that may have lost events means
/// everything: every entity, policy and capability file.
fn targets_of(classifier: &Classifier, batch: &Batch, layout: &RepoLayout) -> HashSet<Target> {
    let mut targets: HashSet<Target> = batch
        .paths
        .keys()
        .filter_map(|path| classifier.classify(path))
        .collect();
    if batch.rescan.is_some() {
        targets.extend([
            Target::Everything,
            Target::Policies,
            Target::Packs,
            Target::Profile,
            Target::Capability(layout.capabilities_dir()),
        ]);
    }
    targets
}

/// Hands each target to whoever owns it
async fn dispatch(
    store: &RepoStore,
    bus: &EventBus,
    feed: &ChangeFeed,
    mut targets: HashSet<Target>,
) {
    let capabilities: Vec<PathBuf> = targets
        .iter()
        .filter_map(|t| match t {
            Target::Capability(path) => Some(path.clone()),
            _ => None,
        })
        .collect();
    targets.retain(|t| !matches!(t, Target::Capability(_)));
    if !capabilities.is_empty() {
        match capability_handler(&store.layout().root) {
            Some(handler) => handler(capabilities),
            None => log::debug!("Capability files changed; no capability service"),
        }
    }
    let layout = store.layout();
    for (target, area, path) in [
        (Target::Packs, ChangeArea::Packs, layout.installed_path()),
        (
            Target::Profile,
            ChangeArea::Profile,
            layout.root.join(PROFILE_FILE),
        ),
    ] {
        if targets.remove(&target) {
            feed.file_changed(area, None, &path);
        }
    }
    if !targets.is_empty() {
        apply(store, bus, feed, targets).await;
    }
}

/// One changed file, as `repo://file_changed` reports it
struct FileChange {
    kind: &'static str,
    id: String,
    path: PathBuf,
    /// changed | unchanged | invalid
    result: &'static str,
    /// Why it's invalid
    error: Option<String>,
}

/// Reloads every entity in a batch (or re-scans everything) and checks
/// every changed policy.
async fn apply(store: &RepoStore, bus: &EventBus, feed: &ChangeFeed, batch: HashSet<Target>) {
    let store = store.clone();
    let changes = tokio::task::spawn_blocking(move || {
        let mut policies = Vec::new();
        let mut entities = Vec::new();
        let mut whole_folder = false;
        for target in batch {
            match target {
                Target::Policy(id) => policies.push(id),
                Target::Policies => whole_folder = true,
                other => entities.push(other),
            }
        }
        if whole_folder {
            policies.extend(policy_ids(&store.layout().policies_dir()));
            policies.sort();
            policies.dedup();
        }
        let mut changes = reload_entities(&store, entities);
        changes.extend(policies.into_iter().map(|id| check_policy(&store, id)));
        changes
    })
    .await
    .unwrap_or_default();
    for change in changes {
        log::debug!(
            "{} '{}' file changed ({})",
            change.kind,
            change.id,
            change.result
        );
        if change.kind == "policy" && change.result == "changed" {
            feed.file_changed(ChangeArea::Policy, Some(change.id.clone()), &change.path);
        }
        let source_path = change.path.display().to_string();
        let changed_by = ChangedBy::Watcher.as_str().to_string();
        if let Some(error) = change.error {
            events::emit(
                bus,
                &RepoValidationFailed {
                    kind: change.kind.to_string(),
                    id: Some(change.id.clone()),
                    source_path: Some(source_path.clone()),
                    error,
                    changed_by: changed_by.clone(),
                },
            );
        }
        events::emit(
            bus,
            &RepoFileChanged {
                kind: change.kind.to_string(),
                id: change.id,
                source_path,
                result: change.result.to_string(),
                changed_by,
            },
        );
    }
}

/// The policy IDs in `dir` (`<id>.md`); none when it doesn't exist
fn policy_ids(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (!ignored(&name))
                .then(|| name.strip_suffix(".md").map(str::to_string))
                .flatten()
        })
        .collect()
}

/// Reloads `RepoStore` entities (the store publishes what changed)
fn reload_entities(store: &RepoStore, targets: Vec<Target>) -> Vec<FileChange> {
    if targets.contains(&Target::Everything) {
        log::info!("Repo folders changed; re-scanning everything");
        store.rescan_all(ChangedBy::Watcher);
        return Vec::new();
    }
    let mut entities: Vec<(EntityKind, String)> = targets
        .into_iter()
        .filter_map(|target| match target {
            Target::Entity(kind, id) => Some((kind, id)),
            _ => None,
        })
        .collect();
    // Agents first, so workflows and triggers referring to a new agent
    // resolve
    entities.sort();
    entities
        .into_iter()
        .map(|(kind, id)| {
            let reload = store.reload(kind, &id, ChangedBy::Watcher);
            let result = match &reload {
                Reload::Unchanged => "unchanged",
                Reload::Changed(_) => "changed",
                Reload::Invalid(_) => "invalid",
            };
            FileChange {
                kind: kind.as_str(),
                path: store.layout().source_path(kind, &id, None),
                id,
                result,
                // The store publishes the validation failure itself
                error: None,
            }
        })
        .collect()
}

/// Checks a changed policy file; the next run reads it from disk
fn check_policy(store: &RepoStore, id: String) -> FileChange {
    let (result, error) = match policies::check_file(store, &id) {
        PolicyFile::Valid | PolicyFile::Removed => ("changed", None),
        PolicyFile::Invalid(error) => {
            log::warn!("Policy '{id}' is invalid: {error}");
            ("invalid", Some(error))
        }
    };
    FileChange {
        kind: "policy",
        path: store.layout().policy_path(&id),
        id,
        result,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::events::tests::RecordingSink;
    use crate::services::AppState;
    use std::fs;
    use std::sync::Arc;

    fn target(kind: EntityKind, id: &str) -> Option<Target> {
        Some(Target::Entity(kind, id.to_string()))
    }

    #[test]
    fn classifies_gold_paths() {
        let layout = RepoLayout::new("/r", "/trash");
        let c = Classifier::new(&layout);
        let p = |s: &str| PathBuf::from(format!("/r/{s}"));
        assert_eq!(
            c.classify(&p(".nandi/agents/writer/skill.md")),
            target(EntityKind::Agent, "writer")
        );
        assert_eq!(
            c.classify(&p(".nandi/agents/writer/instructions.md")),
            target(EntityKind::Agent, "writer")
        );
        assert_eq!(
            c.classify(&p(".nandi/agents/writer")),
            target(EntityKind::Agent, "writer")
        );
        // An installed agent's rules, kept outside its folder
        assert_eq!(
            c.classify(&p(".nandi/overrides/arxiv/instructions.md")),
            target(EntityKind::Agent, "arxiv")
        );
        assert_eq!(
            c.classify(&p(".nandi/agents/writer/workflows/daily.yaml")),
            target(EntityKind::Workflow, "daily")
        );
        assert_eq!(
            c.classify(&p(".nandi/workflows/daily.yml")),
            target(EntityKind::Workflow, "daily")
        );
        assert_eq!(
            c.classify(&p(".nandi/triggers/hook.yaml")),
            target(EntityKind::Trigger, "hook")
        );
        // Capability files go to the capability service (wave A)
        let cap = p(".nandi/capabilities/code-review.python.yaml");
        assert_eq!(c.classify(&cap), Some(Target::Capability(cap.clone())));
        let dir = p(".nandi/capabilities");
        assert_eq!(c.classify(&dir), Some(Target::Capability(dir.clone())));
        assert_eq!(c.classify(&p(".nandi/capabilities/sub/x.yaml")), None);
        assert_eq!(
            c.classify(&p(".nandi/capabilities/.x.yaml.nandi-tmp-1")),
            None
        );
        // Policies are checked, not reloaded
        assert_eq!(
            c.classify(&p(".nandi/policies/no-shell.md")),
            Some(Target::Policy("no-shell".into()))
        );
        assert_eq!(c.classify(&p(".nandi/policies")), Some(Target::Policies));
        assert_eq!(c.classify(&p(".nandi/policies/notes.txt")), None);
        assert_eq!(c.classify(&p(".nandi/policies/.no-shell.md.swp")), None);
        assert_eq!(
            c.classify(&p(".nandi/settings.yaml")),
            target(EntityKind::Settings, "settings")
        );
        assert_eq!(
            c.classify(&p(".nandi/connectors.yaml")),
            target(EntityKind::Connectors, "connectors")
        );
        assert_eq!(
            c.classify(&p(".nandi/manifest.yaml")),
            Some(Target::Everything)
        );
        assert_eq!(c.classify(&p(".nandi/agents")), Some(Target::Everything));
    }

    #[test]
    fn classifies_marketplace_paths() {
        let layout = RepoLayout::new("/r", "/trash");
        let c = Classifier::new(&layout);
        let p = |s: &str| PathBuf::from(format!("/r/.nandi/agents/marketplace/{s}"));
        assert_eq!(
            c.classify(&p("alice/arxiv/skill.md")),
            target(EntityKind::Agent, "arxiv")
        );
        assert_eq!(
            c.classify(&p("alice/arxiv/workflows/daily.yaml")),
            target(EntityKind::Workflow, "daily")
        );
        assert_eq!(
            c.classify(&p("packs/research/agents/summarizer/instructions.md")),
            target(EntityKind::Agent, "summarizer")
        );
        assert_eq!(
            c.classify(&p("packs/research/workflows/review.yaml")),
            target(EntityKind::Workflow, "review")
        );
        for folder in [
            "",
            "alice",
            "alice/arxiv",
            "packs",
            "packs/research",
            "packs/research/agents",
            "packs/research/agents/summarizer",
        ] {
            let path = p(folder);
            let path = path.to_string_lossy().trim_end_matches('/').to_string();
            assert_eq!(
                c.classify(Path::new(&path)),
                Some(Target::Everything),
                "{folder}"
            );
        }
        assert_eq!(c.classify(&p("packs/research/README.md")), None);
        assert_eq!(c.classify(&p("packs/research/.nandi/manifest.yaml")), None);
        assert_eq!(c.classify(&p(".arxiv.nandi-tmp-1/skill.md")), None);
        // Reported to the change feed, not reloaded
        assert_eq!(
            c.classify(Path::new("/r/.nandi/installed.yaml")),
            Some(Target::Packs)
        );
    }

    #[test]
    fn ignores_git_temp_files_and_run_artifacts() {
        let layout = RepoLayout::new("/r", "/trash");
        let c = Classifier::new(&layout);
        for path in [
            "/r/.git/index",
            "/r/.git/refs/heads/main",
            "/r/.nandi/agents/writer/.skill.md.swp",
            "/r/.nandi/agents/writer/skill.md~",
            "/r/.nandi/agents/writer/notes.tmp",
            "/r/.nandi/triggers/.hook.yaml.nandi-tmp-abc",
            "/r/.nandi/agents/writer/runs/1/out.txt",
            "/r/.nandi/workflows/notes.md",
            "/r/README.md",
            "/elsewhere/skill.md",
        ] {
            assert_eq!(c.classify(Path::new(path)), None, "{path}");
        }
    }

    #[test]
    fn classifies_the_profile_and_installed_packs() {
        let layout = RepoLayout::new("/r", "/trash");
        let c = Classifier::new(&layout);
        assert_eq!(c.classify(Path::new("/r/NANDI.md")), Some(Target::Profile));
        assert_eq!(
            c.classify(Path::new("/r/.nandi/installed.yaml")),
            Some(Target::Packs)
        );
        assert_eq!(c.classify(Path::new("/r/.nandi/NANDI.md")), None);
        assert_eq!(c.classify(Path::new("/r/docs/NANDI.md")), None);
    }

    #[test]
    fn events_the_os_may_have_dropped_rescan_everything() {
        use notify::event::{AccessKind, CreateKind, EventKind, Flag};
        let path = PathBuf::from("/r/.nandi/agents/writer/skill.md");
        let event = |kind| notify::Event::new(kind).add_path(path.clone());
        assert_eq!(
            signals(Ok(event(EventKind::Create(CreateKind::File)))),
            [Signal::Path {
                path: path.clone(),
                created: true
            }]
        );
        assert!(signals(Ok(event(EventKind::Access(AccessKind::Read)))).is_empty());
        let dropped = notify::Event::new(EventKind::Other).set_flag(Flag::Rescan);
        assert!(matches!(
            signals(Ok(dropped)).as_slice(),
            [Signal::Rescan(_)]
        ));
        let failed = notify::Error::generic("buffer overflow");
        assert!(matches!(
            signals(Err(failed)).as_slice(),
            [Signal::Rescan(_)]
        ));

        // Too many paths at once counts as dropped too
        let mut burst = Batch::default();
        for n in 0..=BURST_LIMIT {
            burst.add(Signal::Path {
                path: PathBuf::from(format!("/r/.nandi/agents/a{n}/skill.md")),
                created: false,
            });
        }
        assert!(burst.rescan.is_some());

        let layout = RepoLayout::new("/r", "/trash");
        let mut batch = Batch::default();
        batch.add(Signal::Rescan("the OS dropped file events".into()));
        let targets = targets_of(&Classifier::new(&layout), &batch, &layout);
        for target in [
            Target::Everything,
            Target::Policies,
            Target::Packs,
            Target::Profile,
            Target::Capability(layout.capabilities_dir()),
        ] {
            assert!(targets.contains(&target), "{target:?}");
        }
    }

    use crate::services::live::testing::wait_until;

    fn projected_name(state: &AppState, id: &str) -> Option<String> {
        use rusqlite::OptionalExtension;
        let conn = state.db().conn().unwrap();
        conn.query_row("SELECT name FROM agents WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn edits_on_disk_reach_the_aggregate_projection_and_bus() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let layout = state.store().layout().clone();
        fs::create_dir_all(&layout.agents_dir).unwrap();
        fs::create_dir_all(&layout.root).unwrap();
        state.store().load().unwrap();
        let _watcher = spawn(
            state.store().clone(),
            state.bus().clone(),
            state.live().feed().clone(),
            state.runtime(),
        )
        .unwrap();
        let skill = layout.skill_path("writer");

        // A new agent written by an editor
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(&skill, "---\nname: Writer\n---\nWrite.").unwrap();
        wait_until("the new agent", || {
            projected_name(&state, "writer").as_deref() == Some("Writer")
        })
        .await;
        assert_eq!(state.store().display_name("writer"), "Writer");
        // The projection lands before the bus fan-out finishes, so wait for the events too
        wait_until("the created events", || {
            let names = recorder.names();
            names.contains(&"repo://agent-created") && names.contains(&"repo://file_changed")
        })
        .await;

        // Edited in place
        fs::write(&skill, "---\nname: Author\n---\nWrite.").unwrap();
        wait_until("the edit", || {
            projected_name(&state, "writer").as_deref() == Some("Author")
        })
        .await;
        wait_until("the updated event", || {
            recorder.names().contains(&"repo://agent-updated")
        })
        .await;

        // Broken: validation fails, the last good version stays everywhere
        fs::write(&skill, "---\nname: [oops\n---\n").unwrap();
        wait_until("the validation error", || {
            recorder.names().contains(&"repo://validation_failed")
        })
        .await;
        assert_eq!(projected_name(&state, "writer").as_deref(), Some("Author"));
        assert_eq!(state.store().display_name("writer"), "Author");

        // The folder deleted
        fs::remove_dir_all(skill.parent().unwrap()).unwrap();
        wait_until("the removal", || projected_name(&state, "writer").is_none()).await;
        assert!(recorder.names().contains(&"repo://agent-deleted"));
        state.shutdown();
    }

    /// The `repo://file_changed` payloads recorded for policy `id`
    fn policy_results(recorder: &RecordingSink, id: &str) -> Vec<String> {
        recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, payload)| {
                *name == "repo://file_changed" && payload["kind"] == "policy" && payload["id"] == id
            })
            .map(|(_, payload)| payload["result"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn policy_edits_on_disk_are_checked_and_published() {
        let recorder = Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![recorder.clone()]);
        let layout = state.store().layout().clone();
        let skill = layout.skill_path("writer");
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(
            &skill,
            "---
name: Writer
---
Write.",
        )
        .unwrap();
        state.store().load().unwrap();
        let _watcher = spawn(
            state.store().clone(),
            state.bus().clone(),
            state.live().feed().clone(),
            state.runtime(),
        )
        .unwrap();
        let policy = layout.policy_path("no-shell");
        fs::create_dir_all(policy.parent().unwrap()).unwrap();

        // Added: reported, and the next run sees it
        fs::write(
            &policy,
            "---
applies_to: ['*']
tools: { deny: [shell] }
---
",
        )
        .unwrap();
        wait_until("the added policy", || {
            policy_results(&recorder, "no-shell")
                .last()
                .map(String::as_str)
                == Some("changed")
        })
        .await;
        assert!(!state.store().skill("writer").unwrap().allows_tool("shell"));

        // Broken: a validation error, and the agents it may cover stop
        fs::write(
            &policy,
            "---
applies_to: ['*']
tools: { nope: 1 }
---
",
        )
        .unwrap();
        wait_until("the invalid policy", || {
            policy_results(&recorder, "no-shell")
                .last()
                .map(String::as_str)
                == Some("invalid")
        })
        .await;
        let failed = recorder
            .take()
            .into_iter()
            .find(|(name, _)| *name == "repo://validation_failed")
            .map(|(_, payload)| payload)
            .expect("a validation failure");
        assert_eq!(
            (failed["kind"].as_str(), failed["id"].as_str()),
            (Some("policy"), Some("no-shell"))
        );
        let blocked = state.store().skill("writer").unwrap_err();
        assert!(blocked.contains("'no-shell'"), "{blocked}");

        // Deleted: reported, and the agent runs as its skill.md says
        fs::remove_file(&policy).unwrap();
        wait_until("the removed policy", || {
            policy_results(&recorder, "no-shell")
                .last()
                .map(String::as_str)
                == Some("changed")
        })
        .await;
        assert!(state.store().skill("writer").unwrap().allows_tool("shell"));
        state.shutdown();
    }
}
