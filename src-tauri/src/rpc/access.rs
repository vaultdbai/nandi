//! Connector access commands (Tauri adapter for `services::access` and
//! `services::grants`): Settings › Connectors › Access, an agent's
//! Connectors panel, and the answer to an access request.

use crate::services::access::{self, AccessChoice};
use crate::services::domain::grant::{GrantAccess, GrantScope};
use crate::services::grants::GrantInfo;

super::commands! {
    /// Every grant, saved and on this computer
    fn list_connector_grants(state) -> Vec<GrantInfo> => Ok(state.grants().list());

    /// Gives a grant up front (it lasts until removed)
    fn grant_connector_access(state, agent_id: String, connector_id: String, access: GrantAccess, scope: GrantScope) -> () => access::grant_up_front(&state, &agent_id, &connector_id, access, scope).await;

    /// Removes a grant; the agent's next call asks again
    fn revoke_connector_access(state, grant: GrantInfo) -> () => access::revoke(&state, grant).await;

    /// Lets a mail grant cover only these mailboxes (empty: every mailbox)
    fn set_grant_mailboxes(state, grant: GrantInfo, mailboxes: Vec<String>) -> () => access::set_grant_mailboxes(&state, grant, mailboxes).await;

    /// The person's answer to a paused run's access request. `subdomains`: a
    /// site request's grant covers the site's subdomains too. `mailboxes`:
    /// the mailboxes an email request's grant covers (`None`: all it showed)
    fn answer_access_request(state, approval_id: String, choice: AccessChoice, subdomains: bool, mailboxes: Option<Vec<String>>) -> () => { let who = crate::services::chat::decider(&state); access::answer(&state, &approval_id, choice, subdomains, mailboxes, who).await };
}
