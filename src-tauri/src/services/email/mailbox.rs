//! One connected mailbox, whichever provider is behind it: the same calls
//! and shapes for Outlook and Gmail.

use serde_json::Value;

use super::api::{MailApi, MailError};
use super::gmail::Gmail;
use super::mail::{Changes, Header, Identity, Message};
use super::outlook::Outlook;
use super::profile::Profile;
use super::provider::{MailProvider, ProviderSetup, Reads};
use super::settings::EmailAccount;
use crate::services::secrets::Secrets;

pub struct Mailbox {
    provider: MailProvider,
    api: MailApi,
    endpoints: super::provider::Endpoints,
}

impl Mailbox {
    /// A connected account's mailbox, on its sign-in (which renews with
    /// everything that sign-in asked for, so its files and calendar keep
    /// working too)
    pub fn new(account: &EmailAccount, setup: &ProviderSetup, secrets: &Secrets) -> Self {
        Self::signing_in(
            account.provider,
            account.id().token_entry(),
            account.reads(),
            setup,
            secrets,
        )
    }

    /// The account a sign-in in progress (asking for `reads`) is for,
    /// before Nandi knows its address
    pub fn signing_in(
        provider: MailProvider,
        entry: String,
        reads: Reads,
        setup: &ProviderSetup,
        secrets: &Secrets,
    ) -> Self {
        Self {
            provider,
            api: MailApi::new(provider, entry, reads, setup, secrets),
            endpoints: setup.endpoints.clone(),
        }
    }

    pub fn provider(&self) -> MailProvider {
        self.provider
    }

    fn outlook(&self) -> Outlook<'_> {
        Outlook::new(&self.api, &self.endpoints.graph)
    }

    fn gmail(&self) -> Gmail<'_> {
        Gmail::new(
            &self.api,
            &self.endpoints.gmail,
            &self.endpoints.google_userinfo,
        )
    }

    pub async fn identity(&self) -> Result<Identity, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().identity().await,
            MailProvider::Gmail => self.gmail().identity().await,
        }
    }

    /// Title, department and manager (Outlook only; best effort)
    pub async fn profile(&self) -> Profile {
        match self.provider() {
            MailProvider::Outlook => self.outlook().profile().await,
            MailProvider::Gmail => Profile::default(),
        }
    }

    /// New mail since `cursor`, not yet filtered
    pub async fn changes(&self, cursor: &Value, since: i64) -> Result<Changes, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().changes(cursor, since).await,
            MailProvider::Gmail => self.gmail().changes(cursor, since).await,
        }
    }

    /// Looks up the list headers Outlook leaves out of its change list
    pub async fn add_list_headers(&self, headers: &mut [Header]) {
        if self.provider() == MailProvider::Outlook {
            self.outlook().add_list_headers(headers).await;
        }
    }

    /// `id` without its `outlook:` / `gmail:` prefix
    pub async fn message(&self, id: &str) -> Result<Message, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().message(id).await,
            MailProvider::Gmail => self.gmail().message(id).await,
        }
    }

    pub async fn thread(&self, id: &str, limit: u32) -> Result<Vec<Header>, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().thread(id, limit).await,
            MailProvider::Gmail => self.gmail().thread(id, limit).await,
        }
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<Header>, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().search(query, limit).await,
            MailProvider::Gmail => self.gmail().search(query, limit).await,
        }
    }

    pub async fn older(
        &self,
        before: i64,
        floor: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Header>, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().older(before, floor, limit).await,
            MailProvider::Gmail => self.gmail().older(before, floor, limit).await,
        }
    }

    pub async fn count(&self, before: i64, floor: Option<i64>) -> Result<u64, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().count(before, floor).await,
            MailProvider::Gmail => self.gmail().count(before, floor).await,
        }
    }

    pub async fn recent(&self, since: i64, limit: u32) -> Result<Vec<Header>, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().recent(since, limit).await,
            MailProvider::Gmail => self.gmail().recent(since, limit).await,
        }
    }
}
