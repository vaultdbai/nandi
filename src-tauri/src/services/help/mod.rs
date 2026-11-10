//! The built-in help guide: short answers to common questions (install
//! Docker, connect a model, join a relay, what agents, groups and the
//! Inbox are, where things are) for when no model can answer. The free
//! trial may be used up, paused or unreachable, the computer offline, or
//! there's no key yet; Nandi still helps with the basics.
//!
//! It's a small content file ([`guide.json`](guide.json)) and a keyword
//! matcher, and it stays out of the model path: when a model exists, the
//! model answers. The file holds each topic's ID, its keywords in every
//! language the app speaks (en, fr, ar, ja) and its links (navigation
//! targets such as `settings:computer`, or `url:` pages). The answers'
//! words live with the UI's other strings (`locales/*.json`,
//! `help.topics.<id>`), so they're translated like everything else.
//!
//! Matching: the question is lowercased and French accents folded; each
//! keyword found adds its length to its topic's score (Latin keywords must
//! stand as words, an optional plural `s` allowed; Arabic and Japanese
//! ones match anywhere, as words aren't spaced the same way). The best
//! score wins; ties go to the topic listed first.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use specta::Type;

/// How many other matching topics an answer suggests
const MAX_RELATED: usize = 3;

/// A place in the app (or a page on the web) a topic points to
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HelpLink {
    /// `settings:<section>[:<focus>]`, `network:<tab>`, `agents`,
    /// `library`, `automations`, `inbox`, `home`, `settings`, or
    /// `url:<https://...>`
    pub target: String,
    /// The link's words: `help.links.<label>` in the locales
    pub label: String,
}

/// One topic of the guide; its title and answer are
/// `help.topics.<id>.title` and `.answer` in the locales
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HelpTopic {
    pub id: String,
    pub links: Vec<HelpLink>,
}

/// Why the guide answered instead of a model
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum HelpReason {
    /// No model can be called (no key, sign-in or usable trial)
    NoModel,
    /// A model was tried and failed (`detail` says how)
    ModelFailed,
}

/// The guide's answer to a question
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HelpAnswer {
    /// The topic that matches best, if any
    pub topic: Option<HelpTopic>,
    /// Other topics: the next best matches, or every topic when none matched
    pub related: Vec<HelpTopic>,
    pub reason: HelpReason,
    /// The model's error, for `ModelFailed`
    pub detail: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GuideFile {
    topics: Vec<GuideTopic>,
}

#[derive(Debug, Deserialize)]
struct GuideTopic {
    id: String,
    /// Keywords per language (`en`, `fr`, `ar`, `ja`)
    keywords: std::collections::BTreeMap<String, Vec<String>>,
    links: Vec<HelpLink>,
    /// Scales the score; generic topics (where things are) weigh less
    #[serde(default = "full_weight")]
    weight: f32,
}

fn full_weight() -> f32 {
    1.0
}

impl GuideTopic {
    fn topic(&self) -> HelpTopic {
        HelpTopic {
            id: self.id.clone(),
            links: self.links.clone(),
        }
    }

    fn score(&self, text: &str) -> f32 {
        let found: usize = self
            .keywords
            .values()
            .flatten()
            .map(|keyword| fold(keyword))
            .filter(|keyword| contains_keyword(text, keyword))
            .map(|keyword| keyword.chars().count())
            .sum();
        found as f32 * self.weight
    }
}

/// The guide, read once from the file built into the app
fn guide() -> &'static GuideFile {
    static GUIDE: OnceLock<GuideFile> = OnceLock::new();
    GUIDE.get_or_init(|| {
        serde_json::from_str(include_str!("guide.json")).expect("help/guide.json is valid")
    })
}

/// Every topic, in the guide's order
pub fn topics() -> Vec<HelpTopic> {
    guide().topics.iter().map(GuideTopic::topic).collect()
}

/// Lowercase, French accents folded, typographic apostrophes plain
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '’' | '‘' => '\'',
            other => other,
        })
        .collect()
}

/// Latin-script keywords must stand as words; others match anywhere
fn contains_keyword(text: &str, keyword: &str) -> bool {
    if keyword.is_empty() {
        return false;
    }
    let latin = |c: char| (c as u32) < 0x0250;
    if !keyword.chars().all(latin) {
        return text.contains(keyword);
    }
    text.match_indices(keyword).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let mut after = text[at + keyword.len()..].chars();
        // Another script right next to it ("Dockerを") is a boundary too
        let boundary = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() && latin(c)));
        let next = after.next();
        boundary(before) && (boundary(next) || (next == Some('s') && boundary(after.next())))
    })
}

/// The topics that match `question`, best first
fn ranked(question: &str) -> Vec<&'static GuideTopic> {
    let text = fold(question);
    let mut scored: Vec<(usize, f32, &GuideTopic)> = guide()
        .topics
        .iter()
        .enumerate()
        .map(|(order, topic)| (order, topic.score(&text), topic))
        .filter(|(_, score, _)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().map(|(_, _, topic)| topic).collect()
}

/// The guide's answer to `question`: the best topic and a few others, or
/// every topic when nothing matches
pub fn answer(question: &str, reason: HelpReason, detail: Option<String>) -> HelpAnswer {
    let ranked = ranked(question);
    let (topic, related) = match ranked.split_first() {
        Some((best, rest)) => (
            Some(best.topic()),
            rest.iter().take(MAX_RELATED).map(|t| t.topic()).collect(),
        ),
        None => (None, topics()),
    };
    HelpAnswer {
        topic,
        related,
        reason,
        detail,
    }
}

#[cfg(test)]
mod tests;
