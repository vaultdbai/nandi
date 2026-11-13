//! The audit logs' hash chain: every line carries the SHA-256 of the line
//! before it (`prev_sha256`), so an edited, removed or reordered line breaks
//! the chain where it happened. Each append is fsynced, and a failed one
//! marks its log degraded for the rest of the session.
//!
//! The file stays plain JSONL. The chain covers a line once the next one is
//! written: the newest line has no successor to vouch for it.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use specta::Type;

/// The field each line carries: the SHA-256 (hex) of the line before it
pub const PREV_FIELD: &str = "prev_sha256";

/// What the first line of a log chains to
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Bytes read from the end of a log per step while finding its last line
const TAIL_CHUNK: u64 = 4 * 1024;

/// Each log's chain head and write failures, by path. One lock for every
/// log: appends are small, and it keeps two writers of one file from
/// chaining to the same head.
static LOGS: LazyLock<Mutex<HashMap<PathBuf, LogState>>> = LazyLock::new(Mutex::default);

fn logs() -> MutexGuard<'static, HashMap<PathBuf, LogState>> {
    LOGS.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct LogState {
    /// The hash of the last line written, once known
    head: Option<String>,
    /// Entries that failed to write this session
    lost: u32,
    last_error: Option<String>,
}

/// Whether a log's lines are all there, as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ChainCheck {
    /// Every line chains to the one before it
    Intact { lines: u32 },
    /// `line` (1-based) doesn't follow the line before it: that line or the
    /// one before was edited, removed or moved
    Broken { line: u32, reason: String },
}

/// A log's state for the Logs view: what failed to write this session, and
/// whether the file is intact. Lost entries or a broken chain mean the
/// audit is degraded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AuditHealth {
    /// Entries that failed to write since Nandi started
    pub lost_entries: u32,
    pub last_error: Option<String>,
    pub chain: ChainCheck,
}

/// Appends `entry` (a JSON object) to the log at `path`, chained to its
/// last line and fsynced. A failure is logged and counted against the log
/// (see [`health`]), never returned: an unwritable audit log must not break
/// the action being audited.
pub async fn append(path: PathBuf, entry: &impl Serialize) {
    let entry = serde_json::to_value(entry).map_err(|e| e.to_string());
    let target = path.clone();
    let written = tokio::task::spawn_blocking(move || append_blocking(&target, entry?))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    if let Err(e) = written {
        log::error!("Audit degraded: failed to write {}: {e}", path.display());
        let mut logs = logs();
        let log = logs.entry(path).or_default();
        log.lost += 1;
        log.last_error = Some(e);
    }
}

fn append_blocking(path: &Path, entry: Value) -> Result<(), String> {
    let Value::Object(mut fields) = entry else {
        return Err("An audit entry must be a JSON object".to_string());
    };
    let mut logs = logs();
    let log = logs.entry(path.to_path_buf()).or_default();
    // Taken, so a failed write re-reads the head from the file: the write
    // may have got part of the way
    let prev = match log.head.take() {
        Some(head) => head,
        None => last_line(path)?.map_or_else(|| GENESIS.to_string(), |line| sha256_hex(&line)),
    };
    fields.insert(PREV_FIELD.to_string(), Value::String(prev));
    let line = serde_json::to_string(&fields).map_err(|e| e.to_string())?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(format!("{line}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    file.sync_data().map_err(|e| e.to_string())?;
    log.head = Some(sha256_hex(line.as_bytes()));
    Ok(())
}

/// The log at `path`: its failed writes this session, and its chain. A
/// missing log is intact and empty.
pub fn health(path: &Path) -> Result<AuditHealth, String> {
    let chain = verify(path)?;
    let logs = logs();
    let log = logs.get(path);
    Ok(AuditHealth {
        lost_entries: log.map_or(0, |log| log.lost),
        last_error: log.and_then(|log| log.last_error.clone()),
        chain,
    })
}

/// Walks the log at `path` and checks that every line chains to the one
/// before it.
pub fn verify(path: &Path) -> Result<ChainCheck, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ChainCheck::Intact { lines: 0 })
        }
        Err(e) => return Err(format!("Failed to open {}: {e}", path.display())),
    };
    let mut prev = GENESIS.to_string();
    let mut lines = 0;
    for line in BufReader::new(file).split(b'\n') {
        let line = line.map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        lines += 1;
        let broken = |reason: &str| ChainCheck::Broken {
            line: lines,
            reason: reason.to_string(),
        };
        let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(&line) else {
            return Ok(broken("not a JSON object"));
        };
        match fields.get(PREV_FIELD).and_then(Value::as_str) {
            Some(hash) if hash == prev => {}
            Some(_) => return Ok(broken("doesn't follow the line before it")),
            None => return Ok(broken("has no chain hash")),
        }
        prev = sha256_hex(&line);
    }
    Ok(ChainCheck::Intact { lines })
}

/// The last line of the file at `path`, without its newline, read
/// backwards so a long log isn't read whole. `None`: no file, or empty.
fn last_line(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to open {}: {e}", path.display())),
    };
    let read_error = |e: std::io::Error| format!("Failed to read {}: {e}", path.display());
    let mut end = file.metadata().map_err(read_error)?.len();
    let mut tail: Vec<u8> = Vec::new();
    while end > 0 {
        let start = end.saturating_sub(TAIL_CHUNK);
        let mut chunk = vec![0; (end - start) as usize];
        file.seek(SeekFrom::Start(start)).map_err(read_error)?;
        file.read_exact(&mut chunk).map_err(read_error)?;
        chunk.extend_from_slice(&tail);
        tail = chunk;
        // A newline before the file's final one starts the last line
        let body = tail.strip_suffix(b"\n").unwrap_or(&tail);
        if let Some(at) = body.iter().rposition(|b| *b == b'\n') {
            return Ok(Some(body[at + 1..].to_vec()));
        }
        end = start;
    }
    let body = tail.strip_suffix(b"\n").unwrap_or(&tail);
    Ok((!body.is_empty()).then(|| body.to_vec()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A log in a fresh folder, with `count` chained entries
    async fn log_with(count: u32) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs").join("test.log");
        for n in 1..=count {
            append(path.clone(), &json!({ "ts": n, "outcome": "ok" })).await;
        }
        (dir, path)
    }

    fn read_lines(path: &Path) -> Vec<String> {
        let text = std::fs::read_to_string(path).unwrap();
        text.lines().map(str::to_string).collect()
    }

    fn broken_at(check: &ChainCheck) -> Option<u32> {
        match check {
            ChainCheck::Broken { line, .. } => Some(*line),
            ChainCheck::Intact { .. } => None,
        }
    }

    #[tokio::test]
    async fn each_line_carries_the_previous_lines_hash() {
        let (_dir, path) = log_with(3).await;
        let lines = read_lines(&path);
        let prev = |n: usize| {
            let line: Value = serde_json::from_str(&lines[n]).unwrap();
            line[PREV_FIELD].as_str().unwrap().to_string()
        };
        assert_eq!(prev(0), GENESIS);
        assert_eq!(prev(1), sha256_hex(lines[0].as_bytes()));
        assert_eq!(prev(2), sha256_hex(lines[1].as_bytes()));
        assert_eq!(verify(&path).unwrap(), ChainCheck::Intact { lines: 3 });
    }

    #[tokio::test]
    async fn a_new_session_chains_to_the_files_last_line() {
        let (_dir, path) = log_with(2).await;
        // As after a restart: the head is read back from the file
        logs().remove(&path);
        append(path.clone(), &json!({ "ts": 3 })).await;
        assert_eq!(verify(&path).unwrap(), ChainCheck::Intact { lines: 3 });
    }

    #[test]
    fn a_missing_log_is_intact_and_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("none.log");
        assert_eq!(verify(&path).unwrap(), ChainCheck::Intact { lines: 0 });
    }

    #[tokio::test]
    async fn tampering_breaks_the_chain_where_it_happened() {
        type Tamper = fn(&mut Vec<String>);
        // What is done to four chained lines, and the first line that no
        // longer follows the one before it
        let cases: [(&str, Tamper, u32); 6] = [
            ("an edited line", |l| l[1] = l[1].replace("ok", "denied"), 3),
            ("a removed line", |l| drop(l.remove(1)), 2),
            ("a removed first line", |l| drop(l.remove(0)), 1),
            ("two lines swapped", |l| l.swap(1, 2), 2),
            ("an inserted line", |l| l.insert(2, r#"{"ts":9}"#.into()), 3),
            ("a line that isn't JSON", |l| l[3] = "garbage".into(), 4),
        ];
        for (case, tamper, line) in cases {
            let (_dir, path) = log_with(4).await;
            let mut lines = read_lines(&path);
            tamper(&mut lines);
            std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
            let check = verify(&path).unwrap();
            assert_eq!(broken_at(&check), Some(line), "{case}: {check:?}");
        }
    }

    #[tokio::test]
    async fn a_failed_write_marks_the_log_degraded() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the log file should be: every write fails
        let path = dir.path().join("blocked.log");
        std::fs::create_dir(&path).unwrap();
        append(path.clone(), &json!({ "ts": 1 })).await;
        append(path.clone(), &json!({ "ts": 2 })).await;

        let logs = logs();
        let log = logs.get(&path).unwrap();
        assert_eq!(log.lost, 2);
        assert!(log.last_error.is_some());
    }

    #[tokio::test]
    async fn health_reports_lost_entries_and_the_chain() {
        let (_dir, path) = log_with(2).await;
        let health = health(&path).unwrap();
        assert_eq!(health.lost_entries, 0);
        assert_eq!(health.last_error, None);
        assert_eq!(health.chain, ChainCheck::Intact { lines: 2 });
    }

    #[test]
    fn finds_the_last_line_across_read_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.log");
        let long = "x".repeat(TAIL_CHUNK as usize * 2 + 10);
        let cases = [
            (format!("first\n{long}\n"), Some(long.as_str())),
            ("only\n".to_string(), Some("only")),
            ("no newline".to_string(), Some("no newline")),
            (String::new(), None),
        ];
        for (text, last) in cases {
            std::fs::write(&path, &text).unwrap();
            let expected = last.map(|line| line.as_bytes().to_vec());
            assert_eq!(last_line(&path).unwrap(), expected, "{text:.20}");
        }
    }
}
