//! One connected mailbox, whichever provider is behind it: the same calls
//! and shapes for Outlook and Gmail.

use serde_json::Value;

use super::api::{MailApi, MailError};
use super::gmail::Gmail;
use super::mail::{Changes, Header, Identity, Message};
use super::outlook::Outlook;
use super::provider::{MailProvider, ProviderSetup};
use crate::services::secrets::Secrets;

pub struct Mailbox {
    provider: MailProvider,
    api: MailApi,
    endpoints: super::provider::Endpoints,
}

impl Mailbox {
    pub fn new(provider: MailProvider, setup: &ProviderSetup, secrets: &Secrets) -> Self {
        Self {
            provider,
            api: MailApi::new(provider, setup, secrets),
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

    /// New mail since `cursor`, bulk marked (not yet filtered)
    pub async fn changes(&self, cursor: &Value, since: i64) -> Result<Changes, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().changes(cursor, since).await,
            MailProvider::Gmail => self.gmail().changes(cursor, since).await,
        }
    }

    /// Looks up the list headers Outlook leaves out of its change list
    pub async fn mark_bulk(&self, headers: &mut [Header]) {
        if self.provider() == MailProvider::Outlook {
            self.outlook().mark_bulk(headers).await;
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

    pub async fn recent(&self, since: i64, limit: u32) -> Result<Vec<Header>, MailError> {
        match self.provider() {
            MailProvider::Outlook => self.outlook().recent(since, limit).await,
            MailProvider::Gmail => self.gmail().recent(since, limit).await,
        }
    }
}
