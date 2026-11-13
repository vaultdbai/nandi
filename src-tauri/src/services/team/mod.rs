//! Teams: a shared git registry repo for sharing agents and packs among
//! 5–20 trusted people. The registry syncs to `~/.nandi/repo/teams/<id>/`
//! (see `registry` for its layout). There is no GitHub (or any other) API:
//! a registry is any git URL.
//!
//! - Teams and members are machine-local rows (`teams`, `team_members`).
//! - What's shared lives in the registry itself (`index.json`); "shared by
//!   me" is derived from it (entries whose publisher is my handle).
//! - Changes are pushed to `nandi/<my handle>`, never to the default branch
//!   (wave final: Nandi never pushes to main or master).

pub mod registry;
// Wave final, Track E
pub mod roster;
pub mod trust;
pub mod trust_requests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::future::BoxFuture;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::Mutex;

use super::domain::agent_card::{card_from_agent, OwnerRef};
use super::domain::{Agent, AgentCard, AgentId};
use super::events::{self as bus_events, EventBus, NandiEvent};
use super::network;
use super::remote::DEFAULT_BRANCH_PREFIX;
use super::repo_store::RepoStore;
use super::repos::agent_locator;
use super::AppState;
use super::{auth, remote};
use crate::db::{self, Db};
use crate::model::Timestamp;
use crate::services::secrets::Secrets;

use registry::{IndexAgent, Publisher};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    /// The registry URL's last path segment, lowercased
    pub id: String,
    pub name: String,
    pub github_org: String,
    /// The registry's git URL
    pub registry_repo: String,
    pub members: Vec<TeamMember>,
    pub created_at: Timestamp,
    /// Where membership comes from
    pub source: TeamSource,
}

/// Where a team's membership comes from
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamSource {
    pub kind: TeamSourceKind,
    /// The registry's git URL
    pub identifier: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TeamSourceKind {
    /// A git registry
    Git,
}

impl TeamSource {
    pub fn git(url: &str) -> Self {
        Self {
            kind: TeamSourceKind::Git,
            identifier: url.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    pub github_handle: String,
    pub display_name: String,
    pub role: MemberRole,
    pub joined_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MemberRole {
    Owner,
    Admin,
    Member,
}

impl MemberRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "owner" => Self::Owner,
            "admin" => Self::Admin,
            _ => Self::Member,
        }
    }
}

/// A team with what its registry holds, for the team detail view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamDetail {
    pub team: Team,
    pub agents: Vec<AgentCard>,
    /// Pack IDs in the registry
    pub packs: Vec<String>,
    pub last_synced_at: Option<Timestamp>,
    /// My shares pushed but not merged yet (not in the team's index)
    #[serde(default)]
    pub pending_shares: Vec<PendingShare>,
}

/// An agent I pushed to a team's share branch, waiting for a team admin
/// to merge it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingShare {
    pub agent_id: String,
    pub branch: String,
    pub shared_at: Timestamp,
}

/// What sharing did: pushed `branch` (an admin must merge it), or nothing
/// because the team already has this version (`None`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ShareResult {
    pub branch: Option<String>,
}

/// `local_settings` key for a team's pending shares
fn pending_shares_key(team_id: &str) -> String {
    format!("team.pending_shares.{team_id}")
}

fn read_pending_shares(
    conn: &rusqlite::Connection,
    key: &str,
) -> rusqlite::Result<Vec<PendingShare>> {
    Ok(db::get_setting(conn, key)?
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default())
}

fn write_pending_shares(
    conn: &rusqlite::Connection,
    key: &str,
    shares: &[PendingShare],
) -> rusqlite::Result<()> {
    let value = serde_json::to_string(shares).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO local_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// The agent ID in a team index card's ID (`<instance>/<agent>`, or bare)
fn card_agent_id(card_id: &str) -> String {
    card_id
        .rsplit_once('/')
        .map_or(card_id, |(_, id)| id)
        .to_string()
}

/// Pending shares whose agent the merged index doesn't list yet
fn still_pending(shares: Vec<PendingShare>, merged: &[String]) -> Vec<PendingShare> {
    shares
        .into_iter()
        .filter(|share| !merged.iter().any(|id| id == &share.agent_id))
        .collect()
}

/// One of my agents shared to a team.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamShare {
    pub team_id: String,
    pub agent_id: String,
    pub shared_at: Timestamp,
}

pub trait TeamManager: Send + Sync {
    fn list_teams(&self) -> BoxFuture<'_, Result<Vec<Team>, String>>;
    /// Cards for the agents in a team's synced registry
    fn team_agents<'a>(&'a self, team_id: &'a str)
        -> BoxFuture<'a, Result<Vec<AgentCard>, String>>;
    /// My agents shared to any team
    fn shares_by_me(&self) -> BoxFuture<'_, Result<Vec<TeamShare>, String>>;
}

/// `team://changed`
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamChanged {
    pub team_id: String,
}

impl NandiEvent for TeamChanged {
    const NAME: &'static str = "team://changed";
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// One registry git operation at a time (they share checkouts)
static REGISTRY_LOCK: Mutex<()> = Mutex::const_new(());

/// Where teams live. Injectable so tests use a temporary folder.
#[derive(Debug, Clone)]
pub struct TeamConfig {
    /// `~/.nandi/repo/teams`
    pub teams_root: PathBuf,
    /// My handle; `None` reads the configured one
    pub username: Option<String>,
}

impl TeamConfig {
    pub fn for_state(state: &AppState) -> Self {
        Self {
            teams_root: state.store().layout().root.join("teams"),
            username: None,
        }
    }
}

/// The `TeamManager` (plus the detail and org lookups the views need).
/// Holds no `AppState`, so its slot in `AppState` makes no cycle.
pub struct TeamService {
    db: Db,
    bus: EventBus,
    store: RepoStore,
    /// For the GitHub sign-in's token (github.com registries)
    secrets: Secrets,
    config: TeamConfig,
}

/// How a registry change reached the remote
enum Published {
    Unchanged,
    /// Pushed to this branch (`nandi/<handle>`)
    Branch(String),
}

struct TeamRow {
    id: String,
    name: String,
    github_org: String,
    /// The registry's git URL
    registry_repo: String,
    created_at: i64,
}

const TEAM_COLUMNS: &str = "id, name, github_org, registry_repo, created_at";

fn team_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TeamRow> {
    Ok(TeamRow {
        id: row.get(0)?,
        name: row.get(1)?,
        github_org: row.get(2)?,
        registry_repo: row.get(3)?,
        created_at: row.get(4)?,
    })
}

impl TeamRow {
    fn source(&self) -> TeamSource {
        TeamSource::git(&self.registry_repo)
    }
}

fn now() -> i64 {
    db::now_ms()
}

/// A team ID from its registry URL: the last path segment without `.git`,
/// lowercased
fn team_id_from_url(url: &str) -> Result<String, String> {
    let segments: Vec<&str> = url
        .split(['/', '\\', ':'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let stem = |s: &str| s.strip_suffix(".git").unwrap_or(s).to_lowercase();
    let id: String = segments
        .last()
        .map(|s| stem(s))
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let id = id.trim_matches(|c| c == '-' || c == '.').to_string();
    if id.is_empty() {
        return Err(format!("Can't name a team after '{url}'"));
    }
    Ok(id)
}

impl TeamService {
    pub fn new(state: &AppState, config: TeamConfig) -> Self {
        Self {
            db: state.db().clone(),
            bus: state.bus().clone(),
            store: state.store().clone(),
            secrets: state.secrets().clone(),
            config,
        }
    }

    /// The GitHub sign-in's token, for a github.com registry
    fn registry_token(&self, url: &str) -> Option<String> {
        auth::github::token_for_url(&self.secrets, url)
    }

    fn me(&self) -> Result<String, String> {
        if let Some(name) = &self.config.username {
            return Ok(name.clone());
        }
        auth::user_slug_at(&self.store.layout().root)
            .ok_or_else(|| "Set your name in Settings › Account first".to_string())
    }

    /// My display name for registries and member rows
    fn my_display_name(&self, me: &str) -> String {
        if self.config.username.is_some() {
            return me.to_string();
        }
        auth::display_name_at(&self.store.layout().root).unwrap_or_else(|| me.to_string())
    }

    fn dir(&self, team_id: &str) -> PathBuf {
        registry::checkout_dir(&self.config.teams_root, team_id)
    }

    fn emit(&self, team_id: &str) {
        bus_events::emit(
            &self.bus,
            &TeamChanged {
                team_id: team_id.to_string(),
            },
        );
    }

    // -- Database ----------------------------------------------------------

    async fn find_team_row(&self, team_id: &str) -> Result<Option<TeamRow>, String> {
        let id = team_id.to_lowercase();
        self.db
            .call(move |conn| {
                conn.query_row(
                    &format!("SELECT {TEAM_COLUMNS} FROM teams WHERE id = ?1"),
                    [&id],
                    team_row,
                )
                .optional()
            })
            .await
    }

    async fn team_row(&self, team_id: &str) -> Result<TeamRow, String> {
        self.find_team_row(team_id)
            .await?
            .ok_or_else(|| format!("You're not in the team {team_id}"))
    }

    async fn team_rows(&self) -> Result<Vec<TeamRow>, String> {
        self.db
            .call(|conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {TEAM_COLUMNS} FROM teams ORDER BY name COLLATE NOCASE"
                ))?;
                let rows = stmt.query_map([], team_row)?;
                rows.collect()
            })
            .await
    }

    async fn members(&self, team_id: &str) -> Result<Vec<TeamMember>, String> {
        let id = team_id.to_string();
        self.db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT github_handle, display_name, role, joined_at FROM team_members
                     WHERE team_id = ?1 ORDER BY github_handle COLLATE NOCASE",
                )?;
                let rows = stmt.query_map([&id], |row| {
                    Ok(TeamMember {
                        github_handle: row.get(0)?,
                        display_name: row.get(1)?,
                        role: MemberRole::parse(&row.get::<_, String>(2)?),
                        joined_at: db::ts(row.get(3)?),
                    })
                })?;
                rows.collect()
            })
            .await
    }

    async fn to_team(&self, row: TeamRow) -> Result<Team, String> {
        let members = self.members(&row.id).await?;
        Ok(Team {
            source: row.source(),
            id: row.id,
            name: row.name,
            github_org: row.github_org,
            registry_repo: row.registry_repo,
            members,
            created_at: db::ts(row.created_at),
        })
    }

    // -- Git ---------------------------------------------------------------

    async fn sync_checkout(&self, team: &TeamRow) -> Result<registry::Synced, String> {
        let dir = self.dir(&team.id);
        let url = team.registry_repo.clone();
        let token = self.registry_token(&url);
        let teams_root = self.config.teams_root.clone();
        let _guard = REGISTRY_LOCK.lock().await;
        tokio::task::spawn_blocking(move || {
            registry::exclude_from_parent(&teams_root);
            registry::sync(&dir, &url, token.as_deref(), db::ts(now()))
        })
        .await
        .map_err(|e| format!("Sync task failed: {e}"))?
    }

    /// Syncs, applies `change` to the checkout, commits, and pushes it to
    /// `branch` (never the default branch). A failure leaves the checkout as
    /// the remote has it.
    async fn publish<F>(
        &self,
        team: &TeamRow,
        branch: String,
        message: String,
        change: F,
    ) -> Result<Published, String>
    where
        F: FnOnce(&Path) -> Result<(), String> + Send + 'static,
    {
        let dir = self.dir(&team.id);
        let url = team.registry_repo.clone();
        let token = self.registry_token(&url);
        let me = self.me()?;
        let teams_root = self.config.teams_root.clone();
        let _guard = REGISTRY_LOCK.lock().await;
        tokio::task::spawn_blocking(move || {
            registry::exclude_from_parent(&teams_root);
            registry::sync(&dir, &url, token.as_deref(), db::ts(now()))?;
            let result = (|| {
                change(&dir)?;
                if registry::commit(&dir, &message, Some(&me))?.is_none() {
                    return Ok(Published::Unchanged);
                }
                registry::push_head(&dir, &branch, token.as_deref(), true)?;
                registry::reset_to_origin(&dir)?;
                Ok(Published::Branch(branch))
            })();
            if result.is_err() {
                let _ = registry::reset_to_origin(&dir);
            }
            result
        })
        .await
        .map_err(|e| format!("Registry task failed: {e}"))?
    }

    // -- Operations --------------------------------------------------------

    pub async fn list_teams(&self) -> Result<Vec<Team>, String> {
        let mut teams = Vec::new();
        for row in self.team_rows().await? {
            teams.push(self.to_team(row).await?);
        }
        Ok(teams)
    }

    pub async fn get_team(&self, team_id: &str) -> Result<TeamDetail, String> {
        let row = self.team_row(team_id).await?;
        let dir = self.dir(&row.id);
        let agents = self.team_agents(&row.id).await?;
        let pending_shares = self.pending_shares(&row.id, &agents).await?;
        let team = self.to_team(row).await?;
        Ok(TeamDetail {
            team,
            agents,
            packs: registry::packs(&dir),
            last_synced_at: registry::last_synced(&dir),
            pending_shares,
        })
    }

    async fn add_member(
        &self,
        team_id: &str,
        handle: &str,
        role: MemberRole,
    ) -> Result<(), String> {
        let (id, handle) = (team_id.to_string(), handle.to_string());
        let display_name = self.my_display_name(&handle);
        let joined_at = now();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO team_members
                         (team_id, github_handle, display_name, role, joined_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![id, handle, display_name, role.as_str(), joined_at],
                )
            })
            .await?;
        Ok(())
    }

    async fn delete_team_row(&self, team_id: &str) -> Result<(), String> {
        let id = team_id.to_string();
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM team_members WHERE team_id = ?1", [&id])?;
                conn.execute("DELETE FROM teams WHERE id = ?1", [&id])
            })
            .await?;
        Ok(())
    }

    /// A team from any git registry URL (no org lookup, no membership
    /// check). The ID is the URL's last path segment without `.git`.
    /// Missing layout is scaffolded and pushed to `nandi/<me>`, never to the default branch.
    pub async fn create_team(&self, registry_url: &str) -> Result<Team, String> {
        let url = registry_url.trim().to_string();
        remote::validate_git_url(&url)?;
        let me = self.me()?;
        let id = team_id_from_url(&url)?;
        if self.find_team_row(&id).await?.is_some() {
            return Err(format!("You're already in a team called {id}"));
        }
        let row = TeamRow {
            id: id.clone(),
            name: id.clone(),
            github_org: id.clone(),
            registry_repo: url.clone(),
            created_at: now(),
        };
        {
            let (id, url, created_at) = (row.id.clone(), url.clone(), row.created_at);
            self.db
                .call(move |conn| {
                    conn.execute(
                        "INSERT INTO teams
                             (id, name, github_org, registry_repo, created_at,
                              source_kind, source_identifier)
                         VALUES (?1, ?1, ?1, ?2, ?3, 'git', ?2)",
                        rusqlite::params![id, url, created_at],
                    )
                })
                .await?;
        }
        let set_up = self
            .publish(
                &row,
                format!("{DEFAULT_BRANCH_PREFIX}/{me}"),
                "Set up the Nandi registry".to_string(),
                |dir| registry::scaffold(dir).map(|_| ()),
            )
            .await;
        if let Err(e) = set_up {
            // Nothing half-made stays behind
            let _ = self.delete_team_row(&row.id).await;
            let dir = self.dir(&row.id);
            let _guard = REGISTRY_LOCK.lock().await;
            if dir.exists() {
                let _ = std::fs::remove_dir_all(&dir);
            }
            return Err(e);
        }
        self.add_member(&row.id, &me, MemberRole::Owner).await?;
        self.emit(&row.id);
        self.to_team(row).await
    }

    /// Re-syncs a team that's already here (and lists me as a member).
    /// New teams come in through `create_team` with the registry's URL:
    /// with no directory service, an ID alone can't be resolved.
    pub async fn join_team(&self, team_id: &str) -> Result<(), String> {
        let Some(row) = self.find_team_row(team_id).await? else {
            return Err(format!(
                "There's no team {team_id} here. Add it with its registry's git URL instead."
            ));
        };
        let me = self.me()?;
        self.sync_checkout(&row).await?;
        self.add_member(&row.id, &me, MemberRole::Member).await?;
        self.emit(&row.id);
        Ok(())
    }

    /// Forgets the team here and removes its checkout. Nothing on the
    /// remote changes.
    pub async fn leave_team(&self, team_id: &str) -> Result<(), String> {
        let row = self.team_row(team_id).await?;
        let id = row.id.clone();
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM team_members WHERE team_id = ?1", [&id])?;
                conn.execute("DELETE FROM teams WHERE id = ?1", [&id])
            })
            .await?;
        let dir = self.dir(&row.id);
        let _guard = REGISTRY_LOCK.lock().await;
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("Failed to remove {}: {e}", dir.display()))?;
        }
        self.emit(&row.id);
        Ok(())
    }

    pub async fn sync_team_registry(&self, team_id: &str) -> Result<(), String> {
        let row = self.team_row(team_id).await?;
        self.sync_checkout(&row).await?;
        self.emit(&row.id);
        Ok(())
    }

    /// Pushes the agent to a share branch. Nothing is merged: the share is
    /// pending until a team admin merges the branch (`TeamDetail::pending_shares`).
    pub async fn share_agent(&self, team_id: &str, agent_id: &str) -> Result<ShareResult, String> {
        let team = self.team_row(team_id).await?;
        let me = self.me()?;
        let place = agent_locator::locate(self.store.layout(), agent_id)
            .ok_or_else(|| format!("No agent '{agent_id}'"))?;
        if place.origin.is_vendored() {
            return Err(format!(
                "'{agent_id}' was installed from the marketplace. Fork it to share your own copy."
            ));
        }
        let skill = std::fs::read_to_string(place.dir.join(super::repos::layout::SKILL_FILE))
            .map_err(|e| format!("Failed to read {agent_id}: {e}"))?;
        let agent =
            Agent::from_skill_md(AgentId::parse(agent_id).map_err(|e| e.to_string())?, &skill)
                .map_err(|e| e.to_string())?;
        let entry = IndexAgent {
            id: agent_id.to_string(),
            name: agent.name().to_string(),
            publisher: me.clone(),
            instance_id: network::local_instance_id(&self.db).ok(),
            shared_at: db::ts(now()),
        };
        let publisher = Publisher {
            github_handle: me.clone(),
            display_name: self.my_display_name(&me),
            instance_id: entry.instance_id.clone(),
        };
        let source = place.dir;
        let id = agent_id.to_string();
        let published = self
            .publish(
                &team,
                format!("{DEFAULT_BRANCH_PREFIX}/share-{me}-{agent_id}"),
                format!("Share agent {agent_id}"),
                move |dir| {
                    let mut index = registry::read_index(dir);
                    if let Some(other) = index
                        .agents
                        .iter()
                        .find(|a| a.id == id && !a.publisher.eq_ignore_ascii_case(&entry.publisher))
                    {
                        return Err(format!(
                            "{} already shared an agent called '{id}' with this team",
                            other.publisher
                        ));
                    }
                    registry::scaffold(dir)?;
                    registry::copy_agent(&source, dir, &id)?;
                    index.agents.retain(|a| a.id != id);
                    index.agents.push(entry);
                    index.agents.sort_by(|a, b| a.id.cmp(&b.id));
                    registry::write_index(dir, &index)?;
                    registry::write_publisher(dir, &publisher)
                },
            )
            .await?;
        let branch = match published {
            Published::Branch(branch) => {
                log::info!("Pushed the {} registry change to {branch}", team.id);
                self.remember_pending_share(&team.id, agent_id, &branch)
                    .await?;
                Some(branch)
            }
            Published::Unchanged => None,
        };
        self.emit(&team.id);
        Ok(ShareResult { branch })
    }

    /// Remembers a pushed share until the team index lists it (merged)
    async fn remember_pending_share(
        &self,
        team_id: &str,
        agent_id: &str,
        branch: &str,
    ) -> Result<(), String> {
        let key = pending_shares_key(team_id);
        let share = PendingShare {
            agent_id: agent_id.to_string(),
            branch: branch.to_string(),
            shared_at: db::ts(now()),
        };
        self.db
            .call(move |conn| {
                let mut shares = read_pending_shares(conn, &key)?;
                shares.retain(|s| s.agent_id != share.agent_id);
                shares.push(share);
                write_pending_shares(conn, &key, &shares)
            })
            .await
    }

    /// Shares still waiting to be merged; merged ones are forgotten
    async fn pending_shares(
        &self,
        team_id: &str,
        agents: &[AgentCard],
    ) -> Result<Vec<PendingShare>, String> {
        let key = pending_shares_key(team_id);
        let merged: Vec<String> = agents.iter().map(|card| card_agent_id(&card.id)).collect();
        self.db
            .call(move |conn| {
                let shares = read_pending_shares(conn, &key)?;
                let waiting = still_pending(shares.clone(), &merged);
                if waiting != shares {
                    write_pending_shares(conn, &key, &waiting)?;
                }
                Ok(waiting)
            })
            .await
    }

    pub async fn unshare_agent(&self, team_id: &str, agent_id: &str) -> Result<(), String> {
        let team = self.team_row(team_id).await?;
        let me = self.me()?;
        let id = agent_id.to_string();
        let published = self
            .publish(
                &team,
                format!("{DEFAULT_BRANCH_PREFIX}/unshare-{me}-{agent_id}"),
                format!("Unshare agent {agent_id}"),
                move |dir| {
                    let mut index = registry::read_index(dir);
                    if let Some(other) = index
                        .agents
                        .iter()
                        .find(|a| a.id == id && !a.publisher.eq_ignore_ascii_case(&me))
                    {
                        return Err(format!("'{id}' was shared by {}, not you", other.publisher));
                    }
                    index.agents.retain(|a| a.id != id);
                    registry::remove_agent(dir, &id)?;
                    if dir.join(registry::INDEX_FILE).is_file() {
                        registry::write_index(dir, &index)?;
                    }
                    Ok(())
                },
            )
            .await?;
        if let Published::Branch(branch) = &published {
            log::info!("Pushed the {} registry change to {branch}", team.id);
        }
        self.emit(&team.id);
        Ok(())
    }

    pub async fn team_agents(&self, team_id: &str) -> Result<Vec<AgentCard>, String> {
        let row = self.team_row(team_id).await?;
        let dir = self.dir(&row.id);
        let members: HashMap<String, String> = self
            .members(&row.id)
            .await?
            .into_iter()
            .map(|m| (m.github_handle.to_lowercase(), m.display_name))
            .collect();
        let index = registry::read_index(&dir);
        let mut cards = Vec::new();
        for (id, skill) in registry::agent_folders(&dir) {
            let agent = match AgentId::parse(id.clone())
                .and_then(|agent_id| Agent::from_skill_md(agent_id, &skill))
            {
                Ok(agent) => agent,
                Err(e) => {
                    log::warn!("Skipping {id} in the {} registry: {e}", row.id);
                    continue;
                }
            };
            let entry = index.agents.iter().find(|a| a.id == id);
            let handle = entry.map(|a| a.publisher.clone()).unwrap_or_default();
            let publisher = if handle.is_empty() {
                None
            } else {
                registry::read_publisher(&dir, &handle)
            };
            let instance = entry
                .and_then(|a| a.instance_id.clone())
                .or_else(|| publisher.as_ref().and_then(|p| p.instance_id.clone()))
                .unwrap_or_else(|| format!("team:{}", row.id));
            let display_name = members
                .get(&handle.to_lowercase())
                .cloned()
                .or_else(|| publisher.map(|p| p.display_name))
                .unwrap_or_else(|| handle.clone());
            // Where the relay reaches it is Track B's business: unknown here
            let card = card_from_agent(&agent, &instance, "")
                .with_owner(OwnerRef {
                    github_handle: handle,
                    display_name,
                })
                .with_issued_at(entry.map_or(0.0, |a| a.shared_at));
            cards.push(card);
        }
        Ok(cards)
    }

    pub async fn shares_by_me(&self) -> Result<Vec<TeamShare>, String> {
        let me = self.me()?;
        let mut shares = Vec::new();
        for row in self.team_rows().await? {
            let index = registry::read_index(&self.dir(&row.id));
            shares.extend(
                index
                    .agents
                    .into_iter()
                    .filter(|a| a.publisher.eq_ignore_ascii_case(&me))
                    .map(|a| TeamShare {
                        team_id: row.id.clone(),
                        agent_id: a.id,
                        shared_at: a.shared_at,
                    }),
            );
        }
        Ok(shares)
    }
}

impl TeamManager for TeamService {
    fn list_teams(&self) -> BoxFuture<'_, Result<Vec<Team>, String>> {
        Box::pin(TeamService::list_teams(self))
    }

    fn team_agents<'a>(
        &'a self,
        team_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<AgentCard>, String>> {
        Box::pin(TeamService::team_agents(self, team_id))
    }

    fn shares_by_me(&self) -> BoxFuture<'_, Result<Vec<TeamShare>, String>> {
        Box::pin(TeamService::shares_by_me(self))
    }
}

/// The team service. No network here: teams talk to their registry only
/// when the user acts (or syncs).
pub fn init(state: &AppState) -> Option<Arc<dyn TeamManager>> {
    Some(Arc::new(TeamService::new(
        state,
        TeamConfig::for_state(state),
    )))
}

fn service(state: &AppState) -> TeamService {
    TeamService::new(state, TeamConfig::for_state(state))
}

pub async fn list_teams(state: &AppState) -> Result<Vec<Team>, String> {
    service(state).list_teams().await
}

pub async fn get_team(state: &AppState, team_id: &str) -> Result<TeamDetail, String> {
    service(state).get_team(team_id).await
}

pub async fn create_team(state: &AppState, registry_url: &str) -> Result<Team, String> {
    service(state).create_team(registry_url).await
}

pub async fn join_team(state: &AppState, team_id: &str) -> Result<(), String> {
    service(state).join_team(team_id).await
}

pub async fn leave_team(state: &AppState, team_id: &str) -> Result<(), String> {
    service(state).leave_team(team_id).await
}

pub async fn sync_team_registry(state: &AppState, team_id: &str) -> Result<(), String> {
    service(state).sync_team_registry(team_id).await
}

pub async fn share_agent(
    state: &AppState,
    team_id: &str,
    agent_id: &str,
) -> Result<ShareResult, String> {
    service(state).share_agent(team_id, agent_id).await
}

pub async fn unshare_agent(state: &AppState, team_id: &str, agent_id: &str) -> Result<(), String> {
    service(state).unshare_agent(team_id, agent_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use git2::Repository;

    use super::registry::test_support::{bare_registry, push_files};
    use crate::services::domain::EntityKind;
    use crate::services::repos::Repository as _;

    fn team_service(state: &AppState, root: &Path, username: Option<&str>) -> TeamService {
        TeamService::new(
            state,
            TeamConfig {
                teams_root: root.join("teams"),
                username: username.map(str::to_string),
            },
        )
    }

    fn remote_branches(bare: &Path) -> Vec<String> {
        let repo = Repository::open(bare).unwrap();
        let mut names: Vec<String> = repo
            .branches(Some(git2::BranchType::Local))
            .unwrap()
            .flatten()
            .filter_map(|(b, _)| b.name().ok().flatten().map(str::to_string))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn team_ids_come_from_the_url() {
        for (url, id) in [
            ("https://github.com/acme/agents.git", "agents"),
            (
                "https://git.example.com/Team/Shared-Agents.git",
                "shared-agents",
            ),
            ("git@host:group/registry.git/", "registry"),
            ("file:///srv/git/Design Team.git", "design-team"),
            (r"C:\repos\Acme.git", "acme"),
        ] {
            assert_eq!(team_id_from_url(url).unwrap(), id, "{url}");
        }
        assert!(team_id_from_url("https://host/.git").is_err());
    }

    #[tokio::test]
    async fn create_team_scaffolds_an_empty_registry_on_a_nandi_branch() {
        let state = AppState::for_tests(vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let bare = bare_registry(tmp.path(), "acme");
        let url = bare.to_string_lossy().into_owned();
        let teams = team_service(&state, tmp.path(), Some("alice"));

        let team = teams.create_team(&url).await.unwrap();
        assert_eq!(team.id, "acme");
        assert_eq!(team.source, TeamSource::git(&url));
        assert_eq!(team.registry_repo, url);
        assert_eq!(team.members.len(), 1);
        assert_eq!(team.members[0].github_handle, "alice");
        assert_eq!(team.members[0].role, MemberRole::Owner);

        // The layout went to nandi/alice, never to main
        assert_eq!(remote_branches(&bare), ["nandi/alice"]);
        let dir = tmp.path().join("teams").join("acme");
        assert!(dir.join(registry::INDEX_FILE).is_file());
        assert!(dir.join(registry::AGENTS_DIR).is_dir());

        // Listed and re-syncable
        let listed = teams.list_teams().await.unwrap();
        assert_eq!(listed.len(), 1);
        teams.sync_team_registry("acme").await.unwrap();
        teams.join_team("ACME").await.unwrap();
        assert_eq!(teams.list_teams().await.unwrap()[0].members.len(), 1);

        // A second create of the same team is refused
        assert!(teams.create_team(&url).await.is_err());
    }

    #[tokio::test]
    async fn create_team_leaves_a_scaffolded_registry_alone() {
        let state = AppState::for_tests(vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let bare = bare_registry(tmp.path(), "acme");
        push_files(
            &bare,
            &tmp.path().join("scratch"),
            &[
                ("index.json", "{\"version\":1,\"agents\":[],\"packs\":[]}\n"),
                ("agents/.gitkeep", ""),
                ("packs/.gitkeep", ""),
                ("publishers/.gitkeep", ""),
            ],
        );
        let teams = team_service(&state, tmp.path(), Some("alice"));
        teams.create_team(&bare.to_string_lossy()).await.unwrap();
        assert_eq!(remote_branches(&bare), ["main"]);
    }

    #[tokio::test]
    async fn create_team_rejects_bad_urls_and_rolls_back_failures() {
        let state = AppState::for_tests(vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let teams = team_service(&state, tmp.path(), Some("alice"));
        assert!(teams.create_team("not a url").await.is_err());
        // A git URL nobody answers at
        let missing = tmp.path().join("nowhere").join("gone.git");
        let missing = format!("file:///{}", missing.to_string_lossy().replace('\\', "/"));
        assert!(teams.create_team(&missing).await.is_err());
        assert!(teams.list_teams().await.unwrap().is_empty());
        assert!(!tmp.path().join("teams").join("gone").exists());
    }

    #[tokio::test]
    async fn join_team_needs_a_known_team() {
        let state = AppState::for_tests(vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let teams = team_service(&state, tmp.path(), Some("alice"));
        let err = teams.join_team("acme").await.unwrap_err();
        assert!(err.contains("git URL"), "{err}");
    }

    #[tokio::test]
    async fn me_is_the_identity_slug() {
        let state = AppState::for_tests(vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let _ = std::fs::remove_file(auth::manifest_path(&state));
        let teams = team_service(&state, tmp.path(), None);
        assert!(teams.me().is_err(), "no identity yet");
        auth::set_display_name(&state, "Alice Chen".into())
            .await
            .unwrap();
        assert_eq!(teams.me().unwrap(), "alice-chen");

        let bare = bare_registry(tmp.path(), "acme");
        let team = teams.create_team(&bare.to_string_lossy()).await.unwrap();
        assert_eq!(team.members[0].github_handle, "alice-chen");
        assert_eq!(team.members[0].display_name, "Alice Chen");
        assert_eq!(remote_branches(&bare), ["nandi/alice-chen"]);
    }

    #[tokio::test]
    async fn shares_push_a_share_branch() {
        let state = AppState::for_tests(vec![]);
        let tmp = tempfile::tempdir().unwrap();
        let bare = bare_registry(tmp.path(), "acme");
        let teams = team_service(&state, tmp.path(), Some("alice"));
        teams.create_team(&bare.to_string_lossy()).await.unwrap();

        let agent = Agent::from_skill_md(
            AgentId::parse("papers").unwrap(),
            "---\nname: Papers\n---\nFind papers.\n",
        )
        .unwrap();
        state.store().repos().agents.save(&agent).unwrap();
        state.store().commit(EntityKind::Agent, "papers").unwrap();

        let shared = teams.share_agent("acme", "papers").await.unwrap();
        assert_eq!(
            remote_branches(&bare),
            ["nandi/alice", "nandi/share-alice-papers"]
        );
        // Not merged: the team view says so, with the branch to merge
        assert_eq!(shared.branch.as_deref(), Some("nandi/share-alice-papers"));
        let detail = teams.get_team("acme").await.unwrap();
        assert_eq!(detail.pending_shares.len(), 1);
        assert_eq!(detail.pending_shares[0].agent_id, "papers");
        assert_eq!(detail.pending_shares[0].branch, "nandi/share-alice-papers");
    }

    #[test]
    fn merged_shares_stop_being_pending() {
        let share = |agent: &str| PendingShare {
            agent_id: agent.into(),
            branch: format!("nandi/share-me-{agent}"),
            shared_at: 1.0,
        };
        let waiting = still_pending(
            vec![share("papers"), share("notes")],
            &["papers".to_string()],
        );
        assert_eq!(waiting, [share("notes")]);
        assert_eq!(card_agent_id("desktop-1/papers"), "papers");
        assert_eq!(card_agent_id("papers"), "papers");
    }
}
