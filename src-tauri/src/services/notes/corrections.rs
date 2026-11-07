//! What the person told the Chief of Staff about its notes, so it never
//! does it again: capability suggestions they accepted or dismissed
//! (`suggestions.md`), and cited lines they removed or changed on the My
//! notes page. Both land in `corrections.md`, which the Chief of Staff reads
//! before it adds anything (its skill.md says so).
//!
//! ```markdown
//! # Suggestions
//! - [ ] Add a PR review capability (from 6 review meetings this week)
//! ```
//!
//! ```markdown
//! # Corrections
//! - Dismissed suggestion: Add a PR review capability (2026-11-06)
//! - Removed: Lena owns the SOX audit (from Lena Park, 2026-11-03, "Q3") (2026-11-06)
//! ```

use serde::{Deserialize, Serialize};
use specta::Type;

pub const SUGGESTIONS: &str = "suggestions.md";
pub const CORRECTIONS: &str = "corrections.md";

/// One open suggestion
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// Stable while the line is unchanged
    pub id: String,
    pub text: String,
}

fn id_of(text: &str) -> String {
    crate::services::repos::fs_atomic::content_hash(text.as_bytes())[..12].to_string()
}

/// The open (`- [ ]`) suggestions in `suggestions.md`
pub fn open_suggestions(content: &str) -> Vec<Suggestion> {
    content
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("- [ ] "))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| Suggestion {
            id: id_of(text),
            text: text.to_string(),
        })
        .collect()
}

/// `suggestions.md` with suggestion `id` answered (accepted: ticked;
/// dismissed: removed), and the line for `corrections.md`. `None` when
/// there's no such open suggestion
pub fn answer(
    content: &str,
    id: &str,
    accept: bool,
    today: &str,
) -> Option<(String, String, String)> {
    let mut found = None;
    let mut out = Vec::new();
    for line in content.lines() {
        let open = line.trim_start().strip_prefix("- [ ] ").map(str::trim);
        match open {
            Some(text) if found.is_none() && id_of(text) == id => {
                found = Some(text.to_string());
                if accept {
                    out.push(format!("- [x] {text}"));
                }
            }
            _ => out.push(line.to_string()),
        }
    }
    let text = found?;
    let correction = if accept {
        format!("- Accepted suggestion: {text} ({today})")
    } else {
        format!("- Dismissed suggestion: {text} ({today})")
    };
    Some((format!("{}\n", out.join("\n").trim_end()), correction, text))
}

/// A line the Chief of Staff wrote: it names its source
fn is_cited(line: &str) -> bool {
    let line = line.trim();
    line.starts_with(['-', '|', '*']) && (line.contains("(from ") || line.contains("(calendar"))
}

/// Cited lines the person removed or changed when saving a note
pub fn removed_lines(before: &str, after: &str) -> Vec<String> {
    let kept: std::collections::HashSet<&str> = after.lines().map(str::trim).collect();
    before
        .lines()
        .map(str::trim)
        .filter(|line| is_cited(line) && !kept.contains(line))
        .map(|line| line.trim_start_matches(['-', '*', ' ']).trim().to_string())
        .collect()
}

/// `corrections.md` with `lines` added (it starts with a heading)
pub fn append(corrections: &str, lines: &[String]) -> String {
    let mut out = if corrections.trim().is_empty() {
        "# Corrections\n\nWhat I told the Chief of Staff about its notes. It doesn't add these back.\n".to_string()
    } else {
        format!("{}\n", corrections.trim_end())
    };
    for line in lines {
        if !out.contains(line.as_str()) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUGGESTED: &str = "# Suggestions\n\n- [ ] Add a PR review capability (from 6 review meetings)\n- [x] Done before\n";

    #[test]
    fn open_suggestions_are_answered_and_remembered() {
        let open = open_suggestions(SUGGESTED);
        assert_eq!(open.len(), 1);
        let (file, correction, text) = answer(SUGGESTED, &open[0].id, false, "2026-11-06").unwrap();
        assert!(!file.contains("PR review"), "dismissed ones go");
        assert!(file.contains("- [x] Done before"));
        assert_eq!(correction, "- Dismissed suggestion: Add a PR review capability (from 6 review meetings) (2026-11-06)");
        assert!(text.starts_with("Add a PR review"));
        let (file, correction, _) = answer(SUGGESTED, &open[0].id, true, "2026-11-06").unwrap();
        assert!(file.contains("- [x] Add a PR review capability"));
        assert!(correction.starts_with("- Accepted suggestion:"));
        assert!(answer(SUGGESTED, "nope", true, "x").is_none());
    }

    #[test]
    fn removed_cited_lines_become_corrections() {
        let before = "# Owners\n- Lena owns SOX (from Lena Park, 2026-11-03, \"Q3\")\n- My own line\n- Review on Monday (calendar, 2026-11-09, \"Review\")\n";
        let after = "# Owners\n- Review on Monday (calendar, 2026-11-09, \"Review\")\n";
        assert_eq!(
            removed_lines(before, after),
            ["Lena owns SOX (from Lena Park, 2026-11-03, \"Q3\")"]
        );
        let corrections = append("", &["Removed: x".to_string()]);
        assert!(corrections.starts_with("# Corrections"));
        let again = append(
            &corrections,
            &["Removed: x".to_string(), "Removed: y".to_string()],
        );
        assert_eq!(again.matches("Removed: x").count(), 1);
        assert!(again.ends_with("Removed: y\n"));
    }
}
