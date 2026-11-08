//! What the work mailbox says about its owner beyond name and address:
//! job title, department and manager (Outlook, Graph `/me` and
//! `/me/manager`, both inside `User.Read`). It seeds the Chief of Staff's
//! "My role" when a mailbox is connected, and shows in setup and
//! Settings. Gmail gives none of it. Reading it is best effort: a miss
//! leaves the fields empty and never fails the sign-in.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::mail::str_of;
use crate::services::builtin_agents::CHIEF_OF_STAFF;
use crate::services::repo_store::RepoStore;

/// The "My role" line for team and manager
const TEAM_LINE: &str = "- **Team and manager:**";

/// The mailbox owner's manager
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Manager {
    pub name: String,
    pub email: Option<String>,
    pub title: Option<String>,
}

/// Title, department and manager, each only when the directory has it
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub department: Option<String>,
    #[serde(default)]
    pub manager: Option<Manager>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    Some(str_of(value, key).trim().to_string()).filter(|s| !s.is_empty())
}

impl Profile {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.department.is_none() && self.manager.is_none()
    }

    /// From Graph's `/me` (`jobTitle`, `department`)
    pub fn from_me(me: &Value) -> Self {
        Self {
            title: text(me, "jobTitle"),
            department: text(me, "department"),
            manager: None,
        }
    }

    /// The same, with the manager from Graph's `/me/manager` (a manager
    /// without a name counts as none)
    pub fn with_manager(mut self, manager: &Value) -> Self {
        self.manager = text(manager, "displayName").map(|name| Manager {
            name,
            email: [
                str_of(manager, "mail"),
                str_of(manager, "userPrincipalName"),
            ]
            .into_iter()
            .map(str::trim)
            .find(|e| e.contains('@'))
            .map(str::to_ascii_lowercase),
            title: text(manager, "jobTitle"),
        });
        self
    }

    /// "Dev Chechi, Platform lead": what goes before the address in the
    /// "Name and title" line
    pub fn name_and_title(&self, name: Option<&str>) -> Option<String> {
        let name = name.map(str::trim).filter(|n| !n.is_empty())?;
        Some(match self.title.as_deref() {
            Some(title) => format!("{name}, {title}"),
            None => name.to_string(),
        })
    }

    /// "Platform engineering, reports to Priya Rao (VP Engineering)"
    pub fn team_and_manager(&self) -> Option<String> {
        let manager = self.manager.as_ref().map(|m| match m.title.as_deref() {
            Some(title) => format!("reports to {} ({title})", m.name),
            None => format!("reports to {}", m.name),
        });
        let parts: Vec<String> = [self.department.clone(), manager]
            .into_iter()
            .flatten()
            .collect();
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}

/// The Chief of Staff's "My role" with the "Team and manager" line filled
/// in from `profile`. Only the template's placeholder is replaced: a line
/// the user wrote stays theirs. `None`: nothing to change.
pub fn with_team_and_manager(instructions: &str, profile: &Profile) -> Option<String> {
    let team = profile.team_and_manager()?;
    let mut changed = false;
    let lines: Vec<String> = instructions
        .lines()
        .map(|line| match line.trim_start().strip_prefix(TEAM_LINE) {
            Some(rest) if rest.trim().is_empty() || rest.trim().starts_with("(e.g.") => {
                changed = true;
                format!("{TEAM_LINE} {team}")
            }
            _ => line.to_string(),
        })
        .collect();
    if !changed {
        return None;
    }
    let mut out = lines.join("\n");
    if instructions.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

/// Writes the profile into the Chief of Staff's "My role" through the agent
/// repository (the name line is
/// [`crate::services::builtin_agents::note_identity`])
pub fn note_profile(store: &RepoStore, profile: &Profile) -> Result<bool, String> {
    let current = store.skill(CHIEF_OF_STAFF)?.instructions;
    let Some(updated) = with_team_and_manager(&current, profile) else {
        return Ok(false);
    };
    store
        .repos()
        .agents
        .save_instructions(CHIEF_OF_STAFF, &updated)
        .map_err(|e| e.to_string())?;
    store.commit(crate::services::domain::EntityKind::Agent, CHIEF_OF_STAFF)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn me() -> Value {
        json!({
            "displayName": "Dev Chechi",
            "mail": "dev@company.com",
            "jobTitle": " Platform lead ",
            "department": "Platform engineering"
        })
    }

    fn manager() -> Value {
        json!({
            "@odata.type": "#microsoft.graph.user",
            "displayName": "Priya Rao",
            "mail": null,
            "userPrincipalName": "Priya@Company.com",
            "jobTitle": "VP Engineering"
        })
    }

    #[test]
    fn reads_title_and_department_from_me() {
        let profile = Profile::from_me(&me());
        assert_eq!(profile.title.as_deref(), Some("Platform lead"));
        assert_eq!(profile.department.as_deref(), Some("Platform engineering"));
        assert!(profile.manager.is_none());
    }

    #[test]
    fn blank_and_missing_fields_stay_empty() {
        let profile = Profile::from_me(&json!({"jobTitle": "  ", "department": null}));
        assert!(profile.is_empty());
        assert!(Profile::from_me(&json!({})).is_empty());
    }

    #[test]
    fn reads_the_manager_with_a_fallback_address() {
        let profile = Profile::from_me(&me()).with_manager(&manager());
        let m = profile.manager.unwrap();
        assert_eq!(m.name, "Priya Rao");
        assert_eq!(m.email.as_deref(), Some("priya@company.com"));
        assert_eq!(m.title.as_deref(), Some("VP Engineering"));
    }

    #[test]
    fn a_manager_answer_without_a_name_is_no_manager() {
        let profile = Profile::default().with_manager(&json!({"error": {"code": "x"}}));
        assert!(profile.is_empty());
    }

    #[test]
    fn the_role_text_reads_naturally() {
        let profile = Profile::from_me(&me()).with_manager(&manager());
        assert_eq!(
            profile.name_and_title(Some("Dev Chechi")).as_deref(),
            Some("Dev Chechi, Platform lead")
        );
        assert_eq!(
            profile.team_and_manager().as_deref(),
            Some("Platform engineering, reports to Priya Rao (VP Engineering)")
        );
        let department_only = Profile {
            department: Some("Finance".into()),
            ..Profile::default()
        };
        assert_eq!(
            department_only.team_and_manager().as_deref(),
            Some("Finance")
        );
        assert_eq!(Profile::default().team_and_manager(), None);
        assert_eq!(profile.name_and_title(None), None);
        assert_eq!(
            Profile::default().name_and_title(Some("Dev")).as_deref(),
            Some("Dev")
        );
    }

    #[test]
    fn fills_the_placeholder_team_line_only() {
        let profile = Profile::from_me(&me()).with_manager(&manager());
        let template =
            "# My role\n- **Team and manager:** (e.g. \"Platform team\")\n- **What I own:** x\n";
        assert_eq!(
            with_team_and_manager(template, &profile).unwrap(),
            "# My role\n- **Team and manager:** Platform engineering, reports to Priya Rao (VP Engineering)\n- **What I own:** x\n"
        );
        let own = "- **Team and manager:** Core platform, reports to Sam\n";
        assert_eq!(with_team_and_manager(own, &profile), None);
        assert_eq!(with_team_and_manager(template, &Profile::default()), None);
    }
}
