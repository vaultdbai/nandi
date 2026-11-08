//! Screenshots for the run's log: named per run and step
//! (`browser/<run>/<step>-<n>.png`, step 0 before any step is being done),
//! kept 30 days in the agent's work folder, and never backed up.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// The folder under an agent's work folder that holds its browser's files
pub const BROWSER_FOLDER: &str = "browser";
/// How long screenshots are kept
pub const KEEP_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg"];

/// `browser/<run>`, relative to the agent's work folder
pub fn run_folder(run_id: &str) -> String {
    format!("{BROWSER_FOLDER}/{run_id}")
}

/// The next screenshot's name for `step`, given the file names already in
/// the run's folder
pub fn name<'a>(run_id: &str, step: usize, existing: impl IntoIterator<Item = &'a str>) -> String {
    let prefix = format!("{step}-");
    let taken = existing
        .into_iter()
        .filter_map(|file| file.strip_prefix(&prefix))
        .filter_map(|rest| rest.split('.').next()?.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    format!("{}/{step}-{}.png", run_folder(run_id), taken + 1)
}

/// The next screenshot's name for `step`, creating the run's folder in
/// `workdir` (the browser writes into it)
pub fn next_name(workdir: &Path, run_id: &str, step: usize) -> String {
    let folder = workdir.join(run_folder(run_id));
    if let Err(e) = std::fs::create_dir_all(&folder) {
        log::warn!("Couldn't create {}: {e}", folder.display());
    }
    let existing: Vec<String> = std::fs::read_dir(&folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    name(run_id, step, existing.iter().map(String::as_str))
}

/// Whether `path` (relative to an agent's work folder) is one of `run_id`'s
/// screenshots
pub fn is_run_screenshot(run_id: &str, path: &str) -> bool {
    let Some(file) = path.strip_prefix(&format!("{}/", run_folder(run_id))) else {
        return false;
    };
    !file.is_empty()
        && !file.contains(['/', '\\'])
        && !file.starts_with('.')
        && is_image(Path::new(file))
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn older_than(path: &Path, cutoff: SystemTime) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|modified| modified < cutoff)
}

/// Removes screenshots older than `keep_for` from every agent's browser
/// folder under `work_dir` (`<agent>/browser/**`), and run folders left
/// empty. Returns how many files went.
pub fn remove_old(work_dir: &Path, now: SystemTime, keep_for: Duration) -> usize {
    let Some(cutoff) = now.checked_sub(keep_for) else {
        return 0;
    };
    let Ok(agents) = std::fs::read_dir(work_dir) else {
        return 0;
    };
    let mut removed = 0;
    for agent in agents.flatten() {
        let browser = agent.path().join(BROWSER_FOLDER);
        if browser.is_dir() {
            removed += remove_old_in(&browser, cutoff, 0);
        }
    }
    removed
}

fn remove_old_in(dir: &Path, cutoff: SystemTime, depth: usize) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && depth < 2 {
            removed += remove_old_in(&path, cutoff, depth + 1);
            if depth == 0 && std::fs::read_dir(&path).is_ok_and(|mut e| e.next().is_none()) {
                let _ = std::fs::remove_dir(&path);
            }
        } else if kind.is_file()
            && is_image(&path)
            && older_than(&path, cutoff)
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Removes screenshots past [`KEEP_FOR`] now and then once a day
pub fn start_cleanup(runtime: &tokio::runtime::Handle) {
    runtime.spawn(async {
        let mut tick = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
        loop {
            tick.tick().await;
            let Ok(work_dir) = crate::utils::nandi_config::work_dir() else {
                continue;
            };
            let removed = tokio::task::spawn_blocking(move || {
                remove_old(&work_dir, SystemTime::now(), KEEP_FOR)
            })
            .await
            .unwrap_or(0);
            if removed > 0 {
                log::info!("Removed {removed} screenshot(s) older than 30 days");
            }
        }
    });
}

/// The `aws s3 sync --exclude` pattern that keeps agents' browser files
/// (screenshots, downloads) out of a backup of `folder`, when `folder`
/// holds Nandi's work folders (`work_dir`) or is one agent's work folder
pub fn backup_exclude(folder: &Path, work_dir: &Path) -> Option<String> {
    let relative = |path: &Path| -> String {
        path.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    };
    if let Ok(inside) = work_dir.strip_prefix(folder) {
        let prefix = relative(inside);
        return Some(if prefix.is_empty() {
            format!("*/{BROWSER_FOLDER}/*")
        } else {
            format!("{prefix}/*/{BROWSER_FOLDER}/*")
        });
    }
    let inside: PathBuf = folder.strip_prefix(work_dir).ok()?.to_path_buf();
    (inside.components().count() == 1).then(|| format!("{BROWSER_FOLDER}/*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_count_up_per_step() {
        assert_eq!(name("r1", 0, []), "browser/r1/0-1.png");
        assert_eq!(
            name("r1", 3, ["3-1.png", "3-2.png", "4-7.png"]),
            "browser/r1/3-3.png"
        );
        assert_eq!(name("r1", 4, ["3-1.png", "4-7.png"]), "browser/r1/4-8.png");
        assert_eq!(name("r1", 1, ["1-x.png", "10-2.png"]), "browser/r1/1-1.png");
    }

    #[test]
    fn next_name_creates_the_run_folder_and_skips_taken_names() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(next_name(dir.path(), "r1", 2), "browser/r1/2-1.png");
        assert!(dir.path().join("browser/r1").is_dir());
        std::fs::write(dir.path().join("browser/r1/2-1.png"), b"png").unwrap();
        assert_eq!(next_name(dir.path(), "r1", 2), "browser/r1/2-2.png");
    }

    #[test]
    fn only_the_run_s_own_images_are_its_screenshots() {
        assert!(is_run_screenshot("r1", "browser/r1/1-1.png"));
        assert!(is_run_screenshot("r1", "browser/r1/1-1.JPG"));
        assert!(!is_run_screenshot("r1", "browser/r2/1-1.png"));
        assert!(!is_run_screenshot("r1", "browser/r1/../r2/1-1.png"));
        assert!(!is_run_screenshot("r1", "browser/r1/notes.txt"));
        assert!(!is_run_screenshot("r1", "browser/r1/"));
        assert!(!is_run_screenshot("r1", "../browser/r1/1-1.png"));
    }

    #[test]
    fn old_screenshots_go_and_recent_ones_stay() {
        let work = tempfile::tempdir().unwrap();
        let run = work.path().join("agent-a/browser/r1");
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("1-1.png"), b"png").unwrap();
        std::fs::write(work.path().join("agent-a/browser/screenshot-1.png"), b"png").unwrap();
        std::fs::write(run.join("notes.txt"), b"kept").unwrap();
        let other = work.path().join("agent-a/report.png");
        std::fs::write(&other, b"not a browser file").unwrap();

        // Nothing is 30 days old yet
        assert_eq!(remove_old(work.path(), SystemTime::now(), KEEP_FOR), 0);
        // Thirty-one days on, the images go; other files and folders stay
        let later = SystemTime::now() + Duration::from_secs(31 * 24 * 60 * 60);
        assert_eq!(remove_old(work.path(), later, KEEP_FOR), 2);
        assert!(!run.join("1-1.png").exists());
        assert!(run.join("notes.txt").exists());
        assert!(other.exists());

        // A run folder left empty goes too
        std::fs::remove_file(run.join("notes.txt")).unwrap();
        std::fs::write(run.join("2-1.png"), b"png").unwrap();
        assert_eq!(remove_old(work.path(), later, KEEP_FOR), 1);
        assert!(!run.exists());
        assert!(work.path().join("agent-a/browser").is_dir());
    }

    #[test]
    fn backups_leave_browser_files_out() {
        let home = Path::new("/home/u");
        let work = Path::new("/home/u/.nandi/local/work");
        assert_eq!(
            backup_exclude(home, work).as_deref(),
            Some(".nandi/local/work/*/browser/*")
        );
        assert_eq!(backup_exclude(work, work).as_deref(), Some("*/browser/*"));
        assert_eq!(
            backup_exclude(&work.join("chief-of-staff"), work).as_deref(),
            Some("browser/*")
        );
        assert_eq!(backup_exclude(Path::new("/home/u/Pictures"), work), None);
        assert_eq!(backup_exclude(&work.join("a/browser"), work), None);
    }
}
