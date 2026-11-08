//! Groups (protocol 7): people on a relay who chose to work together.
//!
//! - The relay keeps the groups (`GroupRequest` → `GroupReply`, see
//!   `nandi_protocol::group`); this service makes the calls, signs the
//!   policies, and keeps the gold copy of each group this Nandi is in
//!   (`.nandi/groups/<id>.md`, written only through
//!   `repos::GroupRepository`), plus my access level per group
//!   (`.nandi/groups/access.yaml`, "Ask first" on joining).
//! - Copies are refreshed after every registration (list, then get each of
//!   my groups) and on every `GroupChanged`; then
//!   `network://group_changed` tells the screens.
//! - **Signatures.** The creating Nandi picks the group ID (`g_` + 16 hex),
//!   so even version 1 is signed over the real ID:
//!   `GroupPolicy::signed_bytes(id, version, text)` with this device's key.
//!   Every version is checked against its author's pinned key; one that
//!   doesn't check out refuses the group's work ([`enforce`]).
//! - Incoming work from someone I share groups with goes through
//!   [`enforce::Enforcer::decide`] (the executor's intake and the gateway).
//!
//! Without a relay every call answers `RelayUnavailable`; the copies still
//! apply to work that arrives later.

pub mod enforce;

use std::sync::Arc;

use nandi_protocol::group::{
    GROUP_INVALID, GROUP_INVITE_INVALID, GROUP_LAST_ADMIN, GROUP_LIMIT, GROUP_NAME_MAX,
    GROUP_NOT_ADMIN, GROUP_NOT_FOUND, GROUP_NOT_MEMBER, GROUP_POLICY_MAX_BYTES, GROUP_POLICY_STALE,
};
use nandi_protocol::{
    is_group_id, GroupChange, GroupDetail, GroupInvite, GroupOp, GroupOutcome, GroupPolicy,
    GroupRole, GroupSummary, PublicKeys,
};
use nandi_seal::DeviceKeys;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::Db;
use crate::services::domain::group::{
    verify, AccessLevel, Approval, CopyMember, GroupCopy, GroupPolicyText, POLICY_TEMPLATE,
};
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::network::{self, peer_keys};
use crate::services::relay::client_api::GroupSignal;
use crate::services::relay::device_keys::DeviceKeyring;
use crate::services::relay::{RelayClient, RelayUnavailable, RelayVisibility};
use crate::services::repos::GroupRepository;
use crate::services::AppState;

/// `network://group_changed`: a group I'm in (or was invited to) changed,
/// or (`group_id: None`) everything was listed again after connecting
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GroupsChanged {
    pub group_id: Option<String>,
}

impl NandiEvent for GroupsChanged {
    const NAME: &'static str = "network://group_changed";
}

/// Network › Groups: every group on the relay and the invites for me
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GroupsOverview {
    /// `false`: no relay (the screen asks to connect; the lists are empty)
    pub connected: bool,
    pub visibility: Option<RelayVisibility>,
    pub groups: Vec<GroupSummary>,
    pub invites: Vec<GroupInvite>,
    /// What a new group's policy starts from
    pub policy_template: String,
}

/// Whether a group's policy is its own
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PolicyCheck {
    pub verified: bool,
    /// Why not, when it isn't
    pub problem: Option<String>,
    pub version: u32,
    pub author: String,
    /// The author's name from the member list (empty: unknown)
    pub author_name: String,
    /// The policy's `approval:` (`None`: the text doesn't read)
    pub approval: Option<Approval>,
}

/// One group as its member sees it here
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub group: GroupDetail,
    pub policy: PolicyCheck,
    pub my_role: Option<GroupRole>,
    pub my_access: AccessLevel,
    /// This Nandi's instance ID
    pub me: String,
}

/// A new invite: for one person, or a code for whoever has it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InviteRequest {
    pub instance_id: Option<String>,
    pub max_uses: u32,
    /// Epoch ms; `None`: until revoked
    pub expires_at: Option<f64>,
}

/// This device's keys, and its public half (cached)
pub type KeySource = Arc<dyn Fn() -> Result<DeviceKeys, String> + Send + Sync>;
pub type PublicKeySource = Arc<dyn Fn() -> Result<PublicKeys, String> + Send + Sync>;

/// The group calls, over one relay connection and the repo
#[derive(Clone)]
pub struct Groups {
    relay: Option<Arc<dyn RelayClient>>,
    repo: GroupRepository,
    db: Db,
    me: String,
    keys: KeySource,
    public_keys: PublicKeySource,
    bus: Option<EventBus>,
}

impl Groups {
    pub fn new(
        relay: Option<Arc<dyn RelayClient>>,
        repo: GroupRepository,
        db: Db,
        me: String,
        keys: KeySource,
        public_keys: PublicKeySource,
        bus: Option<EventBus>,
    ) -> Self {
        Self {
            relay,
            repo,
            db,
            me,
            keys,
            public_keys,
            bus,
        }
    }

    pub fn of(state: &AppState) -> Result<Self, String> {
        let keyring = Arc::new(DeviceKeyring::new(state.secrets().clone()));
        let public = keyring.clone();
        Ok(Self::new(
            state.relay_client().cloned(),
            state.store().repos().groups.clone(),
            state.db().clone(),
            network::local_instance_id(state.db())?,
            Arc::new(move || keyring.device_keys()),
            Arc::new(move || public.public_keys()),
            Some(state.bus().clone()),
        ))
    }

    fn connected(&self) -> Result<&Arc<dyn RelayClient>, String> {
        self.relay
            .as_ref()
            .filter(|relay| relay.status().connected)
            .ok_or_else(|| RelayUnavailable.into())
    }

    /// One call; a refusal stays an `Ok` for the caller to read
    async fn call(&self, op: GroupOp) -> Result<GroupOutcome, String> {
        self.connected()?.group_request(op).await
    }

    /// One call that must succeed: a refusal becomes its sentence
    async fn call_ok(&self, op: GroupOp) -> Result<GroupOutcome, String> {
        match self.call(op).await? {
            GroupOutcome::Refused { code, message } => Err(refusal(&code, &message)),
            outcome => Ok(outcome),
        }
    }

    fn sign(&self, group_id: &str, version: u32, text: &str) -> Result<GroupPolicy, String> {
        let keys = (self.keys)()?;
        Ok(GroupPolicy {
            version,
            text: text.to_string(),
            author: self.me.clone(),
            signature: keys.sign(&GroupPolicy::signed_bytes(group_id, version, text)),
            updated_at: 0.0,
        })
    }

    fn emit(&self, group_id: Option<String>) {
        if let Some(bus) = &self.bus {
            events::emit(bus, &GroupsChanged { group_id });
        }
    }

    // -----------------------------------------------------------------
    // Reading
    // -----------------------------------------------------------------

    pub async fn overview(&self) -> Result<GroupsOverview, String> {
        let Ok(relay) = self.connected() else {
            return Ok(GroupsOverview {
                connected: false,
                visibility: None,
                groups: Vec::new(),
                invites: Vec::new(),
                policy_template: POLICY_TEMPLATE.to_string(),
            });
        };
        let visibility = relay.status().visibility;
        match self.call_ok(GroupOp::List).await? {
            GroupOutcome::List { groups, invites } => Ok(GroupsOverview {
                connected: true,
                visibility,
                groups,
                invites,
                policy_template: POLICY_TEMPLATE.to_string(),
            }),
            other => Err(unexpected(&other)),
        }
    }

    /// One group in full (members only); its copy is refreshed on the way
    pub async fn get(&self, group_id: &str) -> Result<GroupView, String> {
        match self
            .call(GroupOp::Get {
                group_id: group_id.to_string(),
            })
            .await?
        {
            GroupOutcome::Group { group } => {
                self.keep(&group).await?;
                Ok(self.view(group).await)
            }
            GroupOutcome::Refused { code, message } => {
                if code == GROUP_NOT_MEMBER || code == GROUP_NOT_FOUND {
                    self.forget(group_id)?;
                }
                Err(refusal(&code, &message))
            }
            other => Err(unexpected(&other)),
        }
    }

    async fn view(&self, group: GroupDetail) -> GroupView {
        let my_role = group
            .members
            .iter()
            .find(|m| m.instance_id == self.me)
            .map(|m| m.role);
        let policy = &group.policy;
        let check = match self.author_keys(&policy.author).await {
            Ok(keys) => verify(&group.summary.id, policy, Some(&keys)),
            Err(e) => Err(e),
        };
        let author_name = group
            .members
            .iter()
            .find(|m| m.instance_id == policy.author)
            .map(|m| m.display_name.clone())
            .unwrap_or_default();
        let approval = GroupPolicyText::parse(&group.summary.id, &policy.text)
            .ok()
            .map(|p| p.requests.approval);
        let my_access = self
            .repo
            .load_access()
            .map(|a| a.level(&group.summary.id))
            .unwrap_or_default();
        GroupView {
            policy: PolicyCheck {
                verified: check.is_ok(),
                problem: check.err(),
                version: policy.version,
                author: policy.author.clone(),
                author_name,
                approval,
            },
            my_role,
            my_access,
            me: self.me.clone(),
            group,
        }
    }

    /// The author's key: mine, the pinned one, else the relay's (pinned now)
    async fn author_keys(&self, author: &str) -> Result<PublicKeys, String> {
        if author == self.me {
            return (self.public_keys)();
        }
        if let Some(keys) = peer_keys::pinned(&self.db, author)? {
            return Ok(keys);
        }
        match &self.relay {
            // Pinned here too, so enforcement can check it offline
            Some(relay) => {
                let keys = relay.public_keys_of(author).await?;
                peer_keys::learn(&self.db, author, &keys)
            }
            None => Err(format!("no known key for {author}")),
        }
    }

    // -----------------------------------------------------------------
    // The copies
    // -----------------------------------------------------------------

    /// Writes the copy of a group I'm in (its author's key learned first,
    /// so enforcement can check it later without the relay)
    async fn keep(&self, group: &GroupDetail) -> Result<(), String> {
        let id = &group.summary.id;
        if !group.members.iter().any(|m| m.instance_id == self.me) {
            return self.forget(id);
        }
        if let Err(e) = self.author_keys(&group.policy.author).await {
            log::info!("Groups: {id}'s policy author has no key here yet: {e}");
        }
        let copy = GroupCopy {
            group_id: id.clone(),
            name: group.summary.name.clone(),
            version: group.policy.version,
            author: group.policy.author.clone(),
            signature: group.policy.signature.clone(),
            updated_at: group.policy.updated_at,
            members: group
                .members
                .iter()
                .map(|m| CopyMember {
                    instance_id: m.instance_id.clone(),
                    role: m.role,
                })
                .collect(),
            text: group.policy.text.clone(),
        };
        self.repo.save(&copy).map_err(|e| e.to_string())?;
        let mut access = self.repo.load_access().map_err(|e| e.to_string())?;
        if !access.has(id) {
            access
                .set(id, AccessLevel::default())
                .map_err(|e| e.to_string())?;
            self.repo.save_access(&access).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// I'm not in `group_id` any more: its copy and my level go
    fn forget(&self, group_id: &str) -> Result<(), String> {
        self.repo.delete(group_id).map_err(|e| e.to_string())?;
        let mut access = self.repo.load_access().map_err(|e| e.to_string())?;
        if access.remove(group_id) {
            self.repo.save_access(&access).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Lists again and refreshes the copy of each of my groups; copies of
    /// groups I left go
    pub async fn refresh_all(&self) -> Result<(), String> {
        let groups = match self.call_ok(GroupOp::List).await? {
            GroupOutcome::List { groups, .. } => groups,
            other => return Err(unexpected(&other)),
        };
        let mine: Vec<&GroupSummary> = groups.iter().filter(|g| g.role.is_some()).collect();
        for group in &mine {
            if let Err(e) = self.refresh_group(&group.id).await {
                log::warn!("Groups: couldn't refresh {}: {e}", group.id);
            }
        }
        let (copies, broken) = self.repo.load_all().map_err(|e| e.to_string())?;
        let stale = copies
            .iter()
            .map(|c| c.group_id.clone())
            .chain(broken.into_iter().map(|b| b.group_id))
            .filter(|id| !mine.iter().any(|g| &g.id == id));
        for id in stale.collect::<Vec<_>>() {
            self.forget(&id)?;
        }
        self.emit(None);
        Ok(())
    }

    /// Refreshes one group's copy (dropped when I'm no longer in it)
    pub async fn refresh_group(&self, group_id: &str) -> Result<(), String> {
        match self.get(group_id).await {
            Ok(_) => Ok(()),
            // `get` dropped the copy already
            Err(_) if self.repo.load(group_id).ok().flatten().is_none() => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Group news from the relay
    pub async fn on_signal(&self, signal: GroupSignal) {
        match signal {
            GroupSignal::Registered => {
                if let Err(e) = self.refresh_all().await {
                    log::warn!("Groups: couldn't refresh after connecting: {e}");
                }
            }
            GroupSignal::Changed { group_id, change } => {
                let mine = self.repo.load(&group_id).ok().flatten().is_some();
                let invite_only = matches!(
                    change,
                    GroupChange::Invited { .. } | GroupChange::InviteRevoked { .. }
                );
                if mine || !invite_only {
                    if let Err(e) = self.refresh_group(&group_id).await {
                        log::warn!("Groups: couldn't refresh {group_id}: {e}");
                    }
                }
                self.emit(Some(group_id));
            }
        }
    }

    // -----------------------------------------------------------------
    // Changing
    // -----------------------------------------------------------------

    /// A new group, me its first admin; its v1 policy signed over the ID I
    /// pick. A taken ID is retried once with another.
    pub async fn create(
        &self,
        name: &str,
        description: &str,
        policy_text: &str,
    ) -> Result<GroupView, String> {
        check_text(name, policy_text)?;
        let mut last = String::new();
        for _ in 0..2 {
            let group_id = new_group_id();
            let policy = self.sign(&group_id, 1, policy_text)?;
            match self
                .call(GroupOp::Create {
                    group_id,
                    name: name.trim().to_string(),
                    description: description.trim().to_string(),
                    policy,
                })
                .await?
            {
                GroupOutcome::Group { group } => {
                    self.keep(&group).await?;
                    self.emit(Some(group.summary.id.clone()));
                    return Ok(self.view(group).await);
                }
                GroupOutcome::Refused { code, message } if code == GROUP_INVALID => {
                    last = refusal(&code, &message);
                }
                GroupOutcome::Refused { code, message } => return Err(refusal(&code, &message)),
                other => return Err(unexpected(&other)),
            }
        }
        Err(last)
    }

    /// Renames or redescribes (admins)
    pub async fn update(
        &self,
        group_id: &str,
        name: &str,
        description: &str,
    ) -> Result<GroupView, String> {
        check_name(name)?;
        self.call_ok(GroupOp::Update {
            group_id: group_id.to_string(),
            name: name.trim().to_string(),
            description: description.trim().to_string(),
        })
        .await?;
        self.get(group_id).await
    }

    /// The next policy version, signed by me (admins). Another admin's
    /// change in between: signed again over the version after theirs.
    pub async fn set_policy(&self, group_id: &str, text: &str) -> Result<GroupView, String> {
        GroupPolicyText::parse(group_id, text).map_err(|e| e.to_string())?;
        check_policy_size(text)?;
        for attempt in 0..2 {
            let current = self.get(group_id).await?;
            let version = current
                .group
                .summary
                .policy_version
                .max(current.policy.version)
                + 1;
            let policy = self.sign(group_id, version, text)?;
            match self
                .call(GroupOp::SetPolicy {
                    group_id: group_id.to_string(),
                    policy,
                })
                .await?
            {
                GroupOutcome::Done => return self.get(group_id).await,
                GroupOutcome::Refused { code, .. }
                    if code == GROUP_POLICY_STALE && attempt == 0 => {}
                GroupOutcome::Refused { code, message } => return Err(refusal(&code, &message)),
                other => return Err(unexpected(&other)),
            }
        }
        Err(refusal(GROUP_POLICY_STALE, ""))
    }

    pub async fn invite(
        &self,
        group_id: &str,
        request: InviteRequest,
    ) -> Result<GroupInvite, String> {
        match self
            .call_ok(GroupOp::Invite {
                group_id: group_id.to_string(),
                instance_id: request.instance_id.filter(|id| !id.trim().is_empty()),
                max_uses: request.max_uses.max(1),
                expires_at: request.expires_at,
            })
            .await?
        {
            GroupOutcome::Invite { invite } => Ok(invite),
            other => Err(unexpected(&other)),
        }
    }

    pub async fn revoke_invite(&self, code: &str) -> Result<(), String> {
        self.call_ok(GroupOp::RevokeInvite {
            code: code.trim().to_string(),
        })
        .await
        .map(|_| ())
    }

    /// Joins with a code; my level for it starts at "Ask first"
    pub async fn join(&self, code: &str) -> Result<GroupView, String> {
        match self
            .call_ok(GroupOp::Join {
                code: code.trim().to_string(),
            })
            .await?
        {
            GroupOutcome::Group { group } => {
                self.keep(&group).await?;
                self.emit(Some(group.summary.id.clone()));
                Ok(self.view(group).await)
            }
            other => Err(unexpected(&other)),
        }
    }

    pub async fn leave(&self, group_id: &str) -> Result<(), String> {
        self.call_ok(GroupOp::Leave {
            group_id: group_id.to_string(),
        })
        .await?;
        self.forget(group_id)?;
        self.emit(Some(group_id.to_string()));
        Ok(())
    }

    pub async fn remove_member(
        &self,
        group_id: &str,
        instance_id: &str,
    ) -> Result<GroupView, String> {
        self.call_ok(GroupOp::Remove {
            group_id: group_id.to_string(),
            instance_id: instance_id.to_string(),
        })
        .await?;
        self.get(group_id).await
    }

    pub async fn set_role(
        &self,
        group_id: &str,
        instance_id: &str,
        role: GroupRole,
    ) -> Result<GroupView, String> {
        self.call_ok(GroupOp::SetRole {
            group_id: group_id.to_string(),
            instance_id: instance_id.to_string(),
            role,
        })
        .await?;
        self.get(group_id).await
    }

    /// My level for a group ("Ask first" when I never picked one)
    pub fn access(&self, group_id: &str) -> Result<AccessLevel, String> {
        Ok(self
            .repo
            .load_access()
            .map_err(|e| e.to_string())?
            .level(group_id))
    }

    /// Picks my level for a group; works offline (it's my own file)
    pub fn set_access(&self, group_id: &str, level: AccessLevel) -> Result<AccessLevel, String> {
        let mut access = self.repo.load_access().map_err(|e| e.to_string())?;
        access.set(group_id, level).map_err(|e| e.to_string())?;
        self.repo.save_access(&access).map_err(|e| e.to_string())?;
        self.emit(Some(group_id.to_string()));
        Ok(level)
    }
}

/// `g_` + 16 random lowercase hex characters
pub fn new_group_id() -> String {
    let hex = uuid::Uuid::new_v4().simple().to_string();
    let id = format!("g_{}", &hex[..16]);
    debug_assert!(is_group_id(&id));
    id
}

fn check_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the group a name".to_string());
    }
    if name.chars().count() > GROUP_NAME_MAX {
        return Err(format!(
            "A group's name is at most {GROUP_NAME_MAX} characters"
        ));
    }
    Ok(())
}

fn check_policy_size(text: &str) -> Result<(), String> {
    if text.len() > GROUP_POLICY_MAX_BYTES {
        return Err(format!(
            "A group's policy is at most {} KB",
            GROUP_POLICY_MAX_BYTES / 1024
        ));
    }
    Ok(())
}

fn check_text(name: &str, policy_text: &str) -> Result<(), String> {
    check_name(name)?;
    check_policy_size(policy_text)?;
    GroupPolicyText::parse("new", policy_text).map_err(|e| e.to_string())?;
    Ok(())
}

/// A relay's refusal in words
fn refusal(code: &str, message: &str) -> String {
    let said = match code {
        GROUP_NOT_FOUND => "That group doesn't exist any more",
        GROUP_NOT_MEMBER => "Only the group's members can do that",
        GROUP_NOT_ADMIN => "Only the group's admins can do that",
        GROUP_INVITE_INVALID => "That invite code isn't valid: it may have expired or been used up",
        GROUP_POLICY_STALE => "Someone changed the group's policy at the same time; try again",
        GROUP_LAST_ADMIN => "A group needs an admin: make someone else an admin first",
        GROUP_LIMIT => return format!("The relay's limit for groups is reached: {message}"),
        _ if message.trim().is_empty() => return format!("The relay refused it ({code})"),
        _ => return message.to_string(),
    };
    said.to_string()
}

fn unexpected(outcome: &GroupOutcome) -> String {
    format!("The relay answered something unexpected: {outcome:?}")
}

/// Refreshes the copies on the relay's group news
pub fn watch(state: &AppState, client: &Arc<dyn RelayClient>) {
    let task_state = state.clone();
    let runtime = state.runtime().clone();
    let handler = Box::new(move |signal: GroupSignal| {
        let state = task_state.clone();
        runtime.spawn(async move {
            match Groups::of(&state) {
                Ok(groups) => groups.on_signal(signal).await,
                Err(e) => log::warn!("Groups: {e}"),
            }
        });
    });
    let client = client.clone();
    state
        .runtime()
        .spawn(async move { client.on_groups(handler).await });
}

#[cfg(test)]
mod tests;
