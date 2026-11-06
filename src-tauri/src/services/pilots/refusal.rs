//! Which browser calls a policy refuses (`browser.refuse`,
//! `domain::browser_rule`), from what the call and the page say.
//!
//! Pure. `browser::call` asks the page before a checked call: a snapshot of
//! just the target (`browser_snapshot {target, depth: 1}`), which gives the
//! page's URL and the control's own role and name, or the tab list when the
//! call has no target. The model's description of a control can add to
//! what's refused, never take away: only the page's own name can match a
//! rule's `except`.

use serde_json::Value;

use super::browser::Action;
use crate::services::domain::browser_rule::BrowserCall;
use crate::services::domain::policy::Refusal;

/// The tools whose calls a refusal can stop: the ones that act on the page
const CHECKED: &[&str] = &[
    "browser_click",
    "browser_type",
    "browser_press_key",
    "browser_fill_form",
    "browser_select_option",
    "browser_handle_dialog",
    "browser_drag",
    "browser_file_upload",
];

/// Tools that press a control (a rule's `controls` apply to these)
const PRESSES: &[&str] = &["browser_click", "browser_drag"];

/// Roles that take text: a field named "Post text" isn't a Post button
const FIELDS: &[&str] = &["textbox", "searchbox", "combobox", "spinbutton", "slider"];

pub fn is_checked(tool: &str) -> bool {
    CHECKED.contains(&tool)
}

/// The control a call acts on, as the page names it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub role: String,
    pub name: String,
}

/// The page's URL in a Playwright MCP reply: `- Page URL: …`, or the current
/// tab of a tab list (`- 0: (current) [title](url)`)
pub fn page_url(reply: &str) -> Option<String> {
    reply.lines().find_map(|line| {
        let line = line.trim();
        if let Some(url) = line.strip_prefix("- Page URL: ") {
            return Some(url.trim().to_string());
        }
        line.contains(": (current) [")
            .then(|| line.rsplit_once("](")?.1.strip_suffix(')'))
            .flatten()
            .map(str::to_string)
    })
}

/// The index of the current tab in a tab list
pub fn current_tab(reply: &str) -> Option<usize> {
    reply.lines().find_map(|line| {
        let (index, rest) = line.trim().strip_prefix("- ")?.split_once(':')?;
        rest.trim_start()
            .starts_with("(current)")
            .then(|| index.parse().ok())
            .flatten()
    })
}

/// The element `target` names in a snapshot: its line (`- button "Post"
/// [ref=e21]`), or the first element of a snapshot of just that target
pub fn element(reply: &str, target: &str) -> Option<Element> {
    let marker = format!("[ref={target}]");
    let yaml = reply
        .split_once("```yaml")
        .map(|(_, rest)| rest)
        .unwrap_or(reply);
    let line = yaml
        .lines()
        .find(|line| line.contains(&marker))
        .or_else(|| {
            yaml.lines()
                .find(|line| line.trim_start().starts_with("- "))
        })?;
    parse_element(line)
}

/// `- button "Post now" [active] [ref=e3]` → (`button`, `Post now`)
fn parse_element(line: &str) -> Option<Element> {
    let item = line.trim().strip_prefix("- ")?;
    let role: String = item
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if role.is_empty() {
        return None;
    }
    let rest = &item[role.len()..];
    let name = rest
        .trim_start()
        .strip_prefix('"')
        .and_then(|quoted| {
            // The name ends at the last quote before the attributes
            let end = quoted.find("\" [").or_else(|| quoted.rfind('"'))?;
            Some(quoted[..end].replace("\\\"", "\""))
        })
        .unwrap_or_default();
    Some(Element { role, name })
}

/// The word a rule's `kinds` use for `action` (`None`: never refused)
pub fn kind_word(action: Action) -> Option<&'static str> {
    match action {
        Action::Read | Action::Navigate | Action::HandOver => None,
        Action::Interact => Some("interact"),
        Action::Submit => Some("submit"),
        Action::Secret => Some("secrets"),
        Action::Upload => Some("upload"),
        Action::Pay => Some("pay"),
    }
}

/// What names the control a call presses: the agent's description and
/// target, and the page's own name. Empty for calls that press nothing and
/// for fields.
fn labels(tool: &str, args: &Value, page: Option<&Element>) -> Vec<String> {
    if !PRESSES.contains(&tool) || page.is_some_and(|e| FIELDS.contains(&e.role.as_str())) {
        return Vec::new();
    }
    let mut labels: Vec<String> = ["element", "target", "startElement", "endElement"]
        .iter()
        .filter_map(|key| args[*key].as_str())
        // A bare ref (`e21`) says nothing; a selector (`text=Post`) does
        .filter(|text| !is_ref(text))
        .map(str::to_string)
        .collect();
    if let Some(element) = page {
        labels.push(element.name.clone());
    }
    labels
}

/// Playwright's element refs: `e21`, or `f1e3` inside a frame
fn is_ref(text: &str) -> bool {
    let text = text.trim();
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    match text.strip_prefix('f') {
        Some(framed) => framed
            .split_once('e')
            .is_some_and(|(frame, element)| digits(frame) && digits(element)),
        None => text.strip_prefix('e').is_some_and(digits),
    }
}

/// Why `refusals` stop this call on the page at `url`, if they do. `page`
/// is the control as the page names it, when the page said.
pub fn refused(
    refusals: &[Refusal],
    url: &str,
    tool: &str,
    args: &Value,
    action: Action,
    page: Option<&Element>,
) -> Option<String> {
    let kind = kind_word(action)?;
    if !is_checked(tool) {
        return None;
    }
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let labels = labels(tool, args, page);
    let call = BrowserCall {
        host,
        port: parsed.port_or_known_default(),
        labels: &labels,
        name: page.map(|e| e.name.as_str()),
        kind,
    };
    let refusal = refusals.iter().find(|r| r.rule.refuses(&call))?;
    let what = page
        .map(|e| e.name.clone())
        .filter(|name| !name.is_empty())
        .or_else(|| labels.first().cloned())
        .map(|name| format!("'{name}'"))
        .unwrap_or_else(|| format!("this {kind}"));
    let mut message = format!(
        "Refused by the policy '{}': this agent may not use {what} on {host}.",
        refusal.by
    );
    if let Some(say) = &refusal.rule.say {
        message.push(' ');
        message.push_str(say);
    }
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::policy::Policy;
    use serde_json::json;

    /// The social pack's policy, as it ships (`nandi-social-pack`)
    const SOCIAL: &str = r#"---
name: Social posting stays with a person
applies_to: [agent:social-manager]
browser:
  refuse:
    - controls: [post, publish, share, send, tweet, reply, repost, schedule]
      kinds: [submit]
      sites: [linkedin.com, x.com, twitter.com, facebook.com, instagram.com, youtube.com]
      except: [start a post]
      say: Hand it over with browser_hand_over; the person presses it.
    - controls: [like, react, follow, connect, message, subscribe, invite]
      say: Never automated. Leave it to the person.
---
"#;

    fn social() -> Vec<Refusal> {
        let policy = Policy::parse("social", SOCIAL).unwrap();
        policy
            .rules()
            .browser
            .as_ref()
            .unwrap()
            .refuse
            .iter()
            .map(|rule| Refusal {
                rule: rule.clone(),
                by: "social".into(),
            })
            .collect()
    }

    fn button(name: &str) -> Element {
        Element {
            role: "button".into(),
            name: name.into(),
        }
    }

    fn click(url: &str, target: &str, page: Option<Element>) -> Option<String> {
        let args = json!({ "element": "the control", "target": target });
        refused(
            &social(),
            url,
            "browser_click",
            &args,
            Action::Interact,
            page.as_ref(),
        )
    }

    #[test]
    fn reads_the_page_from_playwright_replies() {
        let snapshot = "### Page\n- Page URL: https://www.linkedin.com/feed/\n- Page Title: Feed\n### Snapshot\n```yaml\n- button \"Post\" [ref=e21]\n```";
        assert_eq!(
            page_url(snapshot).as_deref(),
            Some("https://www.linkedin.com/feed/")
        );
        assert_eq!(element(snapshot, "e21"), Some(button("Post")));
        let full = "```yaml\n- generic [ref=e1]:\n  - textbox \"Post text\" [active] [ref=e18]\n  - button \"Post now\" [ref=e21]\n```";
        assert_eq!(
            element(full, "e18"),
            Some(Element {
                role: "textbox".into(),
                name: "Post text".into()
            })
        );
        assert_eq!(element(full, "e21"), Some(button("Post now")));
        let unnamed = "```yaml\n- article [ref=e5]\n```";
        assert_eq!(element(unnamed, "e5").unwrap().name, "");
        let tabs = "### Result\n- 0: [Home](https://x.com/home)\n- 1: (current) [Compose](https://x.com/compose/post)";
        assert_eq!(
            page_url(tabs).as_deref(),
            Some("https://x.com/compose/post")
        );
        assert_eq!(current_tab(tabs), Some(1));
        assert_eq!(current_tab("No open tabs."), None);
    }

    #[test]
    fn refs_say_nothing_and_selectors_do() {
        for r in ["e1", "e21", "f1e3"] {
            assert!(is_ref(r), "{r}");
        }
        for not in ["Post", "text=Post", "e", "ex1", "#post"] {
            assert!(!is_ref(not), "{not}");
        }
    }

    #[test]
    fn linkedin_publish_controls_are_refused_and_the_composer_opens() {
        let feed = "https://www.linkedin.com/feed/";
        let refusal = click(feed, "e21", Some(button("Post"))).unwrap();
        assert!(refusal.contains("'Post'") && refusal.contains("browser_hand_over"));
        assert!(click(feed, "e30", Some(button("Send"))).is_some());
        assert!(click(feed, "e31", Some(button("Repost"))).is_some());
        assert!(click(feed, "e4", Some(button("Start a post"))).is_none());
        assert!(click(feed, "e9", Some(button("See more"))).is_none());
        // The page's name decides, whatever the agent calls the control
        let args = json!({ "element": "Start a post", "target": "e21" });
        assert!(refused(
            &social(),
            feed,
            "browser_click",
            &args,
            Action::Interact,
            Some(&button("Post"))
        )
        .is_some());
    }

    #[test]
    fn x_publish_controls_and_keyboard_submits_are_refused() {
        let compose = "https://x.com/compose/post";
        assert!(click(compose, "e8", Some(button("Post"))).is_some());
        assert!(click(compose, "e9", Some(button("Reply"))).is_some());
        assert!(click("https://twitter.com/home", "e8", Some(button("Tweet"))).is_some());
        // Ctrl+Enter posts on X: any submit there is refused
        let key = json!({ "key": "Control+Enter" });
        assert!(refused(
            &social(),
            compose,
            "browser_press_key",
            &key,
            Action::Submit,
            None
        )
        .is_some());
        // Typing into the composer is fine; clicking into it too
        let text = Element {
            role: "textbox".into(),
            name: "Post text".into(),
        };
        let typing = json!({ "target": "e5", "text": "Hello" });
        assert!(refused(
            &social(),
            compose,
            "browser_type",
            &typing,
            Action::Interact,
            Some(&text)
        )
        .is_none());
        assert!(click(compose, "e5", Some(text)).is_none());
    }

    #[test]
    fn facebook_instagram_and_youtube_publish_controls_are_refused() {
        for (url, name) in [
            ("https://business.facebook.com/latest/composer", "Publish"),
            ("https://www.facebook.com/", "Post"),
            ("https://www.instagram.com/", "Share"),
            ("https://studio.youtube.com/channel/x/videos", "Publish"),
            ("https://www.youtube.com/channel/x/community", "Post"),
            ("https://studio.youtube.com/", "Schedule"),
        ] {
            assert!(
                click(url, "e1", Some(button(name))).is_some(),
                "{url} {name}"
            );
        }
    }

    #[test]
    fn engagement_controls_are_refused_everywhere() {
        for name in [
            "Like",
            "React Like",
            "Follow Grace",
            "Invite Ada to connect",
            "Message",
            "Subscribe",
        ] {
            assert!(
                click("https://example.com/", "e1", Some(button(name))).is_some(),
                "{name}"
            );
        }
        // On another site a Post button is someone else's business
        assert!(click("https://example.com/", "e1", Some(button("Post"))).is_none());
        // The agent's own description is enough to refuse
        let args = json!({ "element": "Like button", "target": "e1" });
        assert!(refused(
            &social(),
            "https://example.com/",
            "browser_click",
            &args,
            Action::Interact,
            None
        )
        .is_some());
    }

    #[test]
    fn reading_and_moving_are_never_refused() {
        let args = json!({ "element": "Post", "target": "e21" });
        for action in [Action::Read, Action::Navigate, Action::HandOver] {
            assert!(refused(
                &social(),
                "https://x.com/",
                "browser_click",
                &args,
                action,
                None
            )
            .is_none());
        }
        assert!(refused(
            &social(),
            "https://x.com/",
            "browser_snapshot",
            &args,
            Action::Interact,
            None
        )
        .is_none());
        assert!(refused(
            &social(),
            "about:blank",
            "browser_click",
            &args,
            Action::Interact,
            None
        )
        .is_none());
    }
}
