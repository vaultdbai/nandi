//! A know-me proposal: what the Chief of Staff drafts from the person's
//! answers, and what each part changes once the person confirms it. Pure:
//! [`KnowMeProposal::checked`] holds the rules, and the edits are made on
//! values ([`apply_to_manifest`], [`add_lines`]); `knowme::apply` reads
//! and writes the files.
//!
//! | Part               | Goes to                                                        |
//! | ------------------ | -------------------------------------------------------------- |
//! | work, on the card  | NANDI.md `responsibilities:` (shown if `share_responsibilities`) |
//! | work, off the card | My notes `about-me.md`, "Work"                                 |
//! | personal           | My notes `private/about-me.md` only, never NANDI.md             |
//! | personal + group   | also `private/groups/<group>.md`, for that group's members     |
//! | description        | NANDI.md `description`                                         |
//! | can / can't help   | NANDI.md body, "What I can help with" / "What I can't help with" |
//! | notes              | `about-me.md` "Notes", or `private/about-me.md` when private   |
//! | offer              | NANDI.md `policy` (no longer hidden; in `expose` if it lists)  |
//! | suggest            | nothing: shown on the card for the person to install           |

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::services::gateway::manifest::{normalize_responsibilities, MAX_RESPONSIBILITY_CHARS};
use crate::services::gateway::GatewayManifest;

/// The body's section for what this Nandi can help with
pub const CAN_HELP: &str = "What I can help with";
/// The body's section for what it can't
pub const CANT_HELP: &str = "What I can't help with";

const MAX_SUMMARY_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 300;
const MAX_SECTION_CHARS: usize = 2000;
const MAX_NOTE_CHARS: usize = 500;
const MAX_ITEMS: usize = 20;
const MAX_SUGGESTIONS: usize = 10;

/// Work goes where people can find it; personal stays private
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ResponsibilityKind {
    #[default]
    Work,
    /// Family, home, caring for someone
    Personal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ProposedResponsibility {
    pub text: String,
    pub kind: ResponsibilityKind,
    /// Work only: on the card (NANDI.md); off, it stays in My notes
    pub on_card: bool,
    /// Personal only: the one group (a group I'm in) it's shared with
    pub share_with_group: Option<String>,
}

impl Default for ProposedResponsibility {
    fn default() -> Self {
        Self {
            text: String::new(),
            kind: ResponsibilityKind::Work,
            on_card: true,
            share_with_group: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct ProposedNote {
    pub text: String,
    /// For the person only (`private/about-me.md`)
    pub private: bool,
}

/// An agent or pack for something no agent here does: suggested only
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct KnowMeSuggestion {
    pub name: String,
    pub why: String,
}

/// What the person confirms on the card
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct KnowMeProposal {
    /// One line: what changes
    pub summary: String,
    pub responsibilities: Vec<ProposedResponsibility>,
    /// NANDI.md `description`
    pub description: Option<String>,
    /// The body's "What I can help with"
    pub can_help_with: Option<String>,
    /// The body's "What I can't help with"
    pub cant_help_with: Option<String>,
    pub notes: Vec<ProposedNote>,
    /// Capabilities agents here serve, to offer to others
    pub offer: Vec<String>,
    pub suggest: Vec<KnowMeSuggestion>,
}

/// A group I'm in, as the card offers it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GroupRef {
    pub id: String,
    pub name: String,
}

fn cut(text: &str, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((end, _)) => text[..end].trim_end().to_string(),
        None => text.to_string(),
    }
}

/// One line: newlines would forge Markdown structure in a note
fn line(text: &str, max: usize) -> String {
    cut(&text.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

fn some(text: Option<String>, max: usize) -> Option<String> {
    text.map(|t| cut(&t, max)).filter(|t| !t.is_empty())
}

impl KnowMeProposal {
    /// The proposal as it may be applied: trimmed and capped, personal
    /// items off the card, a group only on a personal item and only one
    /// of `groups`, offered capabilities only ones agents here serve
    pub fn checked(self, groups: &[GroupRef], served: &[String]) -> Result<Self, String> {
        let mut responsibilities = Vec::new();
        for item in self.responsibilities.into_iter().take(MAX_ITEMS) {
            let text = line(&item.text, MAX_RESPONSIBILITY_CHARS);
            if text.is_empty() {
                continue;
            }
            let item = match item.kind {
                ResponsibilityKind::Work => ProposedResponsibility {
                    text,
                    kind: ResponsibilityKind::Work,
                    on_card: item.on_card,
                    share_with_group: None,
                },
                ResponsibilityKind::Personal => {
                    let group = item
                        .share_with_group
                        .map(|g| g.trim().to_string())
                        .filter(|g| !g.is_empty());
                    if let Some(group) = &group {
                        if !groups.iter().any(|g| &g.id == group) {
                            return Err(format!(
                                "'{group}' isn't a group you're in: a personal item can only be \
shared with one of your groups"
                            ));
                        }
                    }
                    ProposedResponsibility {
                        text,
                        kind: ResponsibilityKind::Personal,
                        on_card: false,
                        share_with_group: group,
                    }
                }
            };
            responsibilities.push(item);
        }
        let notes: Vec<ProposedNote> = self
            .notes
            .into_iter()
            .take(MAX_ITEMS)
            .map(|n| ProposedNote {
                text: line(&n.text, MAX_NOTE_CHARS),
                private: n.private,
            })
            .filter(|n| !n.text.is_empty())
            .collect();
        let mut offer: Vec<String> = Vec::new();
        for name in self.offer {
            let name = name.trim().to_string();
            if name.is_empty() || offer.contains(&name) {
                continue;
            }
            if !served.contains(&name) {
                return Err(format!(
                    "No agent here serves '{name}': suggest an agent or pack for it instead"
                ));
            }
            offer.push(name);
        }
        let suggest: Vec<KnowMeSuggestion> = self
            .suggest
            .into_iter()
            .take(MAX_SUGGESTIONS)
            .map(|s| KnowMeSuggestion {
                name: line(&s.name, 100),
                why: line(&s.why, 300),
            })
            .filter(|s| !s.name.is_empty())
            .collect();
        let checked = Self {
            summary: line(&self.summary, MAX_SUMMARY_CHARS),
            responsibilities,
            description: some(self.description, MAX_DESCRIPTION_CHARS)
                .map(|d| line(&d, MAX_DESCRIPTION_CHARS)),
            can_help_with: some(self.can_help_with, MAX_SECTION_CHARS),
            cant_help_with: some(self.cant_help_with, MAX_SECTION_CHARS),
            notes,
            offer,
            suggest,
        };
        if checked.changes_nothing() {
            return Err("The proposal changes nothing: add what the person told you".to_string());
        }
        Ok(checked)
    }

    fn changes_nothing(&self) -> bool {
        self.responsibilities.is_empty()
            && self.description.is_none()
            && self.can_help_with.is_none()
            && self.cant_help_with.is_none()
            && self.notes.is_empty()
            && self.offer.is_empty()
            && self.suggest.is_empty()
    }

    /// Whether NANDI.md changes
    pub fn touches_manifest(&self) -> bool {
        self.on_card().next().is_some()
            || self.description.is_some()
            || self.can_help_with.is_some()
            || self.cant_help_with.is_some()
            || !self.offer.is_empty()
    }

    /// Work items for the card
    pub fn on_card(&self) -> impl Iterator<Item = &ProposedResponsibility> {
        self.responsibilities
            .iter()
            .filter(|r| r.kind == ResponsibilityKind::Work && r.on_card)
    }

    fn of_kind(
        &self,
        kind: ResponsibilityKind,
    ) -> impl Iterator<Item = &ProposedResponsibility> + '_ {
        self.responsibilities
            .iter()
            .filter(move |r| r.kind == kind && !(kind == ResponsibilityKind::Work && r.on_card))
    }

    /// The My notes lines, by note and section: `(note, title, section,
    /// lines)`. Personal ones never leave `private/`.
    pub fn note_lines(&self, groups: &[GroupRef]) -> Vec<NoteLines> {
        let group_name = |id: &str| {
            groups
                .iter()
                .find(|g| g.id == id)
                .map_or(id.to_string(), |g| g.name.clone())
        };
        let mut out = Vec::new();
        let work: Vec<String> = self
            .of_kind(ResponsibilityKind::Work)
            .map(|r| r.text.clone())
            .collect();
        push(&mut out, ABOUT_ME, "About me", "Work", work);
        let public_notes: Vec<String> = self
            .notes
            .iter()
            .filter(|n| !n.private)
            .map(|n| n.text.clone())
            .collect();
        push(&mut out, ABOUT_ME, "About me", "Notes", public_notes);
        let personal: Vec<String> = self
            .of_kind(ResponsibilityKind::Personal)
            .map(|r| match &r.share_with_group {
                Some(group) => format!("{} (shared with {})", r.text, group_name(group)),
                None => r.text.clone(),
            })
            .collect();
        push(
            &mut out,
            PRIVATE_ABOUT_ME,
            "About me (private)",
            "Personal",
            personal,
        );
        let private_notes: Vec<String> = self
            .notes
            .iter()
            .filter(|n| n.private)
            .map(|n| n.text.clone())
            .collect();
        push(
            &mut out,
            PRIVATE_ABOUT_ME,
            "About me (private)",
            "Notes",
            private_notes,
        );
        for group in groups {
            let shared: Vec<String> = self
                .of_kind(ResponsibilityKind::Personal)
                .filter(|r| r.share_with_group.as_deref() == Some(group.id.as_str()))
                .map(|r| r.text.clone())
                .collect();
            push(
                &mut out,
                &group_note(&group.id),
                &format!("Shared with {}", group.name),
                SHARED_SECTION,
                shared,
            );
        }
        out
    }

    /// What changed, in words for the decisions file (in the repo, so
    /// personal text never goes there: only counts)
    pub fn record(&self, groups: &[GroupRef]) -> String {
        let mut parts = Vec::new();
        let card: Vec<&str> = self.on_card().map(|r| r.text.as_str()).collect();
        if !card.is_empty() {
            parts.push(format!("on the card: {}", card.join(", ")));
        }
        let count = |n: usize, what: &str| (n > 0).then(|| format!("{what}: {n}"));
        parts.extend(count(
            self.of_kind(ResponsibilityKind::Work).count(),
            "work in My notes",
        ));
        parts.extend(count(
            self.of_kind(ResponsibilityKind::Personal).count(),
            "personal in My notes",
        ));
        for group in groups {
            parts.extend(count(
                self.responsibilities
                    .iter()
                    .filter(|r| r.share_with_group.as_deref() == Some(group.id.as_str()))
                    .count(),
                &format!("shared with {}", group.name),
            ));
        }
        parts.extend(count(self.notes.len(), "notes"));
        if self.description.is_some() {
            parts.push("description".to_string());
        }
        if self.can_help_with.is_some() {
            parts.push(CAN_HELP.to_string());
        }
        if self.cant_help_with.is_some() {
            parts.push(CANT_HELP.to_string());
        }
        if !self.offer.is_empty() {
            parts.push(format!("offered: {}", self.offer.join(", ")));
        }
        if !self.suggest.is_empty() {
            let names: Vec<&str> = self.suggest.iter().map(|s| s.name.as_str()).collect();
            parts.push(format!("suggested: {}", names.join(", ")));
        }
        parts.join("; ")
    }
}

/// The `knowme_propose` tool, offered only in the know-me chat
pub fn spec() -> crate::llm::ToolSpec {
    let text = |max: usize| serde_json::json!({"type": "string", "maxLength": max});
    crate::llm::ToolSpec {
        name: crate::services::inbox::tool::PROPOSE.to_string(),
        description: "Propose changes to the user's card (NANDI.md) and My notes from what they told you in this chat. It waits on a card where the user can edit and confirm it: nothing is written before they do. Work responsibilities may go on the card (`onCard`, on unless they say no). Personal ones (family, home, health, caring for someone) never go on the card: they stay in My notes, unless the user chose one of their groups (`shareWithGroup`, a group id from the list) to share one with. `offer` only names capabilities agents here serve; put anything else in `suggest` (agents or packs to install, suggested only)."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["summary"],
            "properties": {
                "summary": text(MAX_SUMMARY_CHARS),
                "responsibilities": {"type": "array", "maxItems": MAX_ITEMS, "items": {
                    "type": "object",
                    "required": ["text", "kind"],
                    "properties": {
                        "text": text(MAX_RESPONSIBILITY_CHARS),
                        "kind": {"type": "string", "enum": ["work", "personal"]},
                        "onCard": {"type": "boolean"},
                        "shareWithGroup": {"type": "string"}
                    }
                }},
                "description": text(MAX_DESCRIPTION_CHARS),
                "canHelpWith": text(MAX_SECTION_CHARS),
                "cantHelpWith": text(MAX_SECTION_CHARS),
                "notes": {"type": "array", "maxItems": MAX_ITEMS, "items": {
                    "type": "object",
                    "required": ["text"],
                    "properties": {
                        "text": text(MAX_NOTE_CHARS),
                        "private": {"type": "boolean"}
                    }
                }},
                "offer": {"type": "array", "items": {"type": "string"}},
                "suggest": {"type": "array", "maxItems": MAX_SUGGESTIONS, "items": {
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": text(100),
                        "why": text(300)
                    }
                }}
            }
        }),
    }
}

/// My notes' file for what the person told Nandi
pub const ABOUT_ME: &str = "about-me.md";
/// The same, for them only (the Chief of Staff never shares `private/`)
pub const PRIVATE_ABOUT_ME: &str = "private/about-me.md";
/// The section of a group's note
pub const SHARED_SECTION: &str = "Personal";

/// The note of what's shared with one group, inside `private/`: only the
/// gateway's turns for that group's members read it (`knowme::sharing`)
pub fn group_note(group_id: &str) -> String {
    format!("private/groups/{group_id}.md")
}

/// Lines to add to one section of one note
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLines {
    pub note: String,
    pub title: String,
    pub section: String,
    pub lines: Vec<String>,
}

fn push(out: &mut Vec<NoteLines>, note: &str, title: &str, section: &str, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    if let Some(existing) = out
        .iter_mut()
        .find(|n| n.note == note && n.section == section)
    {
        existing.lines.extend(lines);
        return;
    }
    out.push(NoteLines {
        note: note.to_string(),
        title: title.to_string(),
        section: section.to_string(),
        lines,
    });
}

/// `existing` (a note, maybe empty) with `lines` added as bullets under
/// `## section` (made when missing), each once (any case), and its
/// "Last updated" line set to `today`. Nothing already there is removed.
pub fn add_lines(
    existing: &str,
    title: &str,
    section: &str,
    lines: &[String],
    today: &str,
) -> String {
    let mut text = if existing.trim().is_empty() {
        format!("# {title}\n\nLast updated: {today}\n")
    } else {
        existing.trim_end().to_string() + "\n"
    };
    // "Last updated" follows the edit
    let mut out: Vec<String> = Vec::new();
    let mut dated = false;
    for l in text.lines() {
        if !dated && l.starts_with("Last updated:") {
            out.push(format!("Last updated: {today}"));
            dated = true;
        } else {
            out.push(l.to_string());
        }
    }
    text = out.join("\n") + "\n";
    let known: Vec<String> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- "))
        .map(|l| l.trim().to_lowercase())
        .collect();
    let mut new: Vec<String> = Vec::new();
    for l in lines {
        let key = l.trim().to_lowercase();
        if !known.contains(&key) && !new.iter().any(|n| n.to_lowercase() == key) {
            new.push(l.trim().to_string());
        }
    }
    if new.is_empty() {
        return text;
    }
    let bullets: String = new.iter().map(|l| format!("- {l}\n")).collect();
    let heading = format!("## {section}");
    let all: Vec<&str> = text.lines().collect();
    match all.iter().position(|l| l.trim() == heading) {
        Some(start) => {
            // The section ends at the next heading
            let end = all[start + 1..]
                .iter()
                .position(|l| l.starts_with("## ") || l.starts_with("# "))
                .map_or(all.len(), |i| start + 1 + i);
            // After its last non-blank line
            let mut at = end;
            while at > start + 1 && all[at - 1].trim().is_empty() {
                at -= 1;
            }
            let mut out: Vec<String> = all[..at].iter().map(|l| l.to_string()).collect();
            if at == start + 1 {
                out.push(String::new());
            }
            out.extend(new.iter().map(|l| format!("- {l}")));
            if end < all.len() {
                out.push(String::new());
            }
            out.extend(
                all[at..]
                    .iter()
                    .skip_while(|l| l.trim().is_empty())
                    .map(|l| l.to_string()),
            );
            out.join("\n") + "\n"
        }
        None => format!(
            "{}\n{heading}\n\n{bullets}",
            text.trim_end().to_string() + "\n"
        ),
    }
}

/// `body` with `## heading`'s text replaced by `text` (the section is
/// added at the end when missing). Other sections stay as written.
pub fn set_section(body: &str, heading: &str, text: &str) -> String {
    let marker = format!("## {heading}");
    let lines: Vec<&str> = body.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == marker) else {
        let base = body.trim_end();
        let gap = if base.is_empty() { "" } else { "\n\n" };
        return format!("{base}{gap}{marker}\n\n{}\n", text.trim());
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## ") || l.starts_with("# "))
        .map_or(lines.len(), |i| start + 1 + i);
    let mut out: Vec<String> = lines[..=start].iter().map(|l| l.to_string()).collect();
    out.push(String::new());
    out.push(text.trim().to_string());
    if end < lines.len() {
        out.push(String::new());
        out.extend(lines[end..].iter().map(|l| l.to_string()));
    }
    out.join("\n") + "\n"
}

/// The NANDI.md edits of a checked proposal. Personal items never get
/// here: only work items on the card join `responsibilities:`.
pub fn apply_to_manifest(manifest: &mut GatewayManifest, proposal: &KnowMeProposal) {
    let added: Vec<String> = proposal.on_card().map(|r| r.text.clone()).collect();
    if !added.is_empty() {
        let mut all = manifest.responsibilities.clone();
        all.extend(added);
        manifest.responsibilities = normalize_responsibilities(all);
    }
    if let Some(description) = &proposal.description {
        manifest.description = description.clone();
    }
    if let Some(text) = &proposal.can_help_with {
        manifest.body = set_section(&manifest.body, CAN_HELP, text);
    }
    if let Some(text) = &proposal.cant_help_with {
        manifest.body = set_section(&manifest.body, CANT_HELP, text);
    }
    // Offered: no longer hidden, and listed when `expose` lists (empty
    // `expose` offers everything already)
    let policy = &mut manifest.policy;
    for name in &proposal.offer {
        policy.hide.retain(|h| h != name);
        if !policy.expose.is_empty() && !policy.expose.contains(name) {
            policy.expose.push(name.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family() -> Vec<GroupRef> {
        vec![GroupRef {
            id: "g_0123456789abcdef".into(),
            name: "Family".into(),
        }]
    }

    fn work(text: &str, on_card: bool) -> ProposedResponsibility {
        ProposedResponsibility {
            text: text.into(),
            on_card,
            ..ProposedResponsibility::default()
        }
    }

    fn personal(text: &str, group: Option<&str>) -> ProposedResponsibility {
        ProposedResponsibility {
            text: text.into(),
            kind: ResponsibilityKind::Personal,
            // Asking for the card doesn't put a personal item there
            on_card: true,
            share_with_group: group.map(str::to_string),
        }
    }

    #[test]
    fn a_work_item_is_on_the_card_unless_switched_off() {
        let parsed: KnowMeProposal = serde_json::from_value(serde_json::json!({
            "responsibilities": [{"text": "Sprint planning", "kind": "work"}]
        }))
        .unwrap();
        assert!(parsed.responsibilities[0].on_card, "on by default");
    }

    #[test]
    fn personal_items_never_go_on_the_card() {
        let proposal = KnowMeProposal {
            responsibilities: vec![
                work("Jira admin", true),
                work("Interviews", false),
                personal("School runs", None),
            ],
            ..KnowMeProposal::default()
        }
        .checked(&family(), &[])
        .unwrap();
        let mut manifest = GatewayManifest {
            responsibilities: vec!["Risk".into()],
            ..GatewayManifest::default()
        };
        apply_to_manifest(&mut manifest, &proposal);
        assert_eq!(manifest.responsibilities, ["Risk", "Jira admin"]);
        let notes = proposal.note_lines(&family());
        let public = notes.iter().find(|n| n.note == ABOUT_ME).unwrap();
        assert_eq!(public.lines, ["Interviews"]);
        let private = notes.iter().find(|n| n.note == PRIVATE_ABOUT_ME).unwrap();
        assert_eq!(private.lines, ["School runs"]);
        assert!(!notes
            .iter()
            .any(|n| n.note != PRIVATE_ABOUT_ME && n.lines.iter().any(|l| l.contains("School"))));
        // The decisions file gets counts, never the personal text
        let record = proposal.record(&family());
        assert!(record.contains("on the card: Jira admin"), "{record}");
        assert!(record.contains("personal in My notes: 1"), "{record}");
        assert!(!record.contains("School"), "{record}");
    }

    #[test]
    fn a_personal_item_is_shared_only_with_a_group_i_am_in() {
        let refused = KnowMeProposal {
            responsibilities: vec![personal("Mum's appointments", Some("g_other"))],
            ..KnowMeProposal::default()
        }
        .checked(&family(), &[]);
        assert!(refused.unwrap_err().contains("isn't a group you're in"));

        let proposal = KnowMeProposal {
            responsibilities: vec![personal("Mum's appointments", Some("g_0123456789abcdef"))],
            ..KnowMeProposal::default()
        }
        .checked(&family(), &[])
        .unwrap();
        assert!(!proposal.touches_manifest());
        let notes = proposal.note_lines(&family());
        let group = notes
            .iter()
            .find(|n| n.note == "private/groups/g_0123456789abcdef.md")
            .unwrap();
        assert_eq!(group.lines, ["Mum's appointments"]);
        assert_eq!(group.title, "Shared with Family");
        let private = notes.iter().find(|n| n.note == PRIVATE_ABOUT_ME).unwrap();
        assert_eq!(private.lines, ["Mum's appointments (shared with Family)"]);
        // A group on a work item is dropped: work goes on the card
        let work_group = KnowMeProposal {
            responsibilities: vec![ProposedResponsibility {
                share_with_group: Some("g_0123456789abcdef".into()),
                ..work("Payroll", true)
            }],
            ..KnowMeProposal::default()
        }
        .checked(&family(), &[])
        .unwrap();
        assert_eq!(work_group.responsibilities[0].share_with_group, None);
    }

    #[test]
    fn only_served_capabilities_are_offered_and_empty_proposals_are_refused() {
        let served = vec!["sprint-review".to_string()];
        let refused = KnowMeProposal {
            offer: vec!["payroll".into()],
            ..KnowMeProposal::default()
        }
        .checked(&[], &served);
        assert!(refused.unwrap_err().contains("suggest an agent"));
        assert!(KnowMeProposal {
            responsibilities: vec![work("   ", true)],
            ..KnowMeProposal::default()
        }
        .checked(&[], &served)
        .unwrap_err()
        .contains("changes nothing"));

        let proposal = KnowMeProposal {
            offer: vec!["sprint-review".into()],
            ..KnowMeProposal::default()
        }
        .checked(&[], &served)
        .unwrap();
        let mut manifest = GatewayManifest::default();
        manifest.policy.hide = vec!["sprint-review".into()];
        manifest.policy.expose = vec!["ask-me".into()];
        apply_to_manifest(&mut manifest, &proposal);
        assert!(manifest.policy.hide.is_empty());
        assert_eq!(manifest.policy.expose, ["ask-me", "sprint-review"]);
        // An empty `expose` already offers everything: it stays empty
        let mut open = GatewayManifest::default();
        apply_to_manifest(&mut open, &proposal);
        assert!(open.policy.expose.is_empty());
    }

    #[test]
    fn the_help_sections_are_replaced_in_place() {
        let body = "\nI'm Sam.\n\n## What I can help with\n\n(describe)\n\n## What I can't help with\n\n(nothing)\n";
        let edited = set_section(body, CAN_HELP, "Sprint planning and Jira questions.");
        assert_eq!(
            edited,
            "\nI'm Sam.\n\n## What I can help with\n\nSprint planning and Jira questions.\n\n## What I can't help with\n\n(nothing)\n"
        );
        let added = set_section("I'm Sam.\n", CANT_HELP, "Payroll.");
        assert_eq!(added, "I'm Sam.\n\n## What I can't help with\n\nPayroll.\n");
    }

    #[test]
    fn lines_are_added_once_under_their_section() {
        let first = add_lines("", "About me", "Work", &["Interviews".into()], "2026-11-10");
        assert_eq!(
            first,
            "# About me\n\nLast updated: 2026-11-10\n\n## Work\n\n- Interviews\n"
        );
        let second = add_lines(
            &format!("{first}\n## Notes\n\n- Prefers mornings\n"),
            "About me",
            "Work",
            &["interviews".into(), "On-call rota".into()],
            "2026-11-11",
        );
        assert_eq!(
            second,
            "# About me\n\nLast updated: 2026-11-11\n\n## Work\n\n- Interviews\n- On-call rota\n\n## Notes\n\n- Prefers mornings\n"
        );
        // A line can't forge a heading: it's one line, trimmed
        let checked = KnowMeProposal {
            notes: vec![ProposedNote {
                text: "x\n## Hacked".into(),
                private: false,
            }],
            ..KnowMeProposal::default()
        }
        .checked(&[], &[])
        .unwrap();
        assert_eq!(checked.notes[0].text, "x ## Hacked");
    }
}
