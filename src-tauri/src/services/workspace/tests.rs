//! Calendars and files against a mock Graph and Google on `127.0.0.1` (no
//! network): each account's grant and keys, the calendar change feed and
//! one Nandi run per check, the tools naming each result's account, each
//! account's file change feed, and that every request is a `GET` to the
//! account's own API.

use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::response::IntoResponse;
use axum::Router;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::*;
use crate::services::email::provider::Endpoints;
use crate::services::email::settings::ReadSignIn;
use crate::services::email::MailProvider;

type Log = Arc<Mutex<Vec<(String, String)>>>;

#[derive(Clone)]
struct Mock {
    log: Log,
    base: Arc<Mutex<String>>,
}

async fn handle(State(mock): State<Mock>, request: Request) -> impl IntoResponse {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let query = request.uri().query().unwrap_or_default().to_string();
    mock.log
        .lock()
        .unwrap()
        .push((method, format!("{path}?{query}")));
    let base = mock.base.lock().unwrap().clone();
    let body = respond(&base, &path, &query);
    (axum::http::StatusCode::OK, axum::Json(body))
}

fn respond(base: &str, path: &str, query: &str) -> Value {
    match path {
        "/graph/me" => json!({"mail": "Dev@Contoso.com", "displayName": "Dev"}),
        "/userinfo" => json!({"email": "dev@gmail.com", "name": "Dev"}),
        "/graph/me/calendarView/delta" if !query.contains("deltatoken") => json!({
            "value": [
                {"id": "e1", "subject": "Quarterly review", "showAs": "busy",
                 "start": {"dateTime": "2026-11-09T15:00:00.0000000"},
                 "end": {"dateTime": "2026-11-09T16:00:00.0000000"},
                 "organizer": {"emailAddress": {"address": "lena@contoso.com"}},
                 "attendees": [{"emailAddress": {"address": "dev@contoso.com"}}]},
                {"id": "e2", "subject": "Vacation", "showAs": "oof", "isAllDay": true,
                 "start": {"dateTime": "2026-11-20T00:00:00.0000000"},
                 "end": {"dateTime": "2026-11-27T00:00:00.0000000"}}
            ],
            "@odata.deltaLink": format!("{base}/graph/me/calendarView/delta?deltatoken=d1")
        }),
        "/graph/me/calendarView/delta" => json!({
            "value": [],
            "@odata.deltaLink": format!("{base}/graph/me/calendarView/delta?deltatoken=d2")
        }),
        "/graph/me/calendarView" => json!({"value": [
            {"id": "e1", "subject": "Quarterly review", "showAs": "busy",
             "start": {"dateTime": "2026-11-09T15:00:00.0000000"},
             "end": {"dateTime": "2026-11-09T16:00:00.0000000"}}
        ]}),
        "/gcal/calendars/primary/events" if query.contains("syncToken") => {
            json!({"items": [], "nextSyncToken": "s2"})
        }
        "/gcal/calendars/primary/events" => json!({
            "items": [{"id": "g1", "summary": "Standup",
                       "start": {"dateTime": "2026-11-10T09:00:00Z"},
                       "end": {"dateTime": "2026-11-10T09:15:00Z"},
                       "organizer": {"email": "dev@gmail.com"}}],
            "nextSyncToken": "s1"
        }),
        "/graph/me/drive/root/delta" if query.contains("token=latest") => json!({
            "value": [],
            "@odata.deltaLink": format!("{base}/graph/me/drive/root/delta?token=t1")
        }),
        "/graph/me/drive/root/delta" => json!({
            "value": [{"id": "f1", "name": "Plan.md", "file": {"mimeType": "text/markdown"},
                       "webUrl": "https://x/Plan.md", "lastModifiedDateTime": "2026-11-05T10:00:00Z"}],
            "@odata.deltaLink": format!("{base}/graph/me/drive/root/delta?token=t2")
        }),
        "/graph/me/drive/items/f1" => json!({"id": "f1", "name": "Plan.md", "size": 20,
            "file": {"mimeType": "text/markdown"}, "webUrl": "https://x/Plan.md"}),
        "/graph/me/drive/items/f1/content" => json!("# Plan\nShip on Friday"),
        "/gdrive/files" => json!({"files": [{"id": "d1", "name": "Budget",
            "mimeType": "application/vnd.google-apps.spreadsheet", "webViewLink": "https://docs/x"}]}),
        _ => json!({}),
    }
}

async fn mock() -> Mock {
    let mock = Mock {
        log: Arc::default(),
        base: Arc::default(),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    *mock.base.lock().unwrap() = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().fallback(handle).with_state(mock.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    mock
}

fn setup(base: &str) -> ProviderSetup {
    ProviderSetup {
        endpoints: Endpoints {
            microsoft_login: format!("{base}/ms"),
            graph: format!("{base}/graph"),
            google_authorize: format!("{base}/gauth"),
            google_token: format!("{base}/gtoken/token"),
            gmail: format!("{base}/gmail"),
            google_userinfo: format!("{base}/userinfo"),
            google_calendar: format!("{base}/gcal"),
            google_drive: format!("{base}/gdrive"),
        },
        microsoft_client_id: Some("ms-client".into()),
        google_client_id: Some("g-client".into()),
        google_client_secret: Some("g-secret".into()),
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, String)>>);

impl CalendarSink for Recorder {
    fn submit<'a>(
        &'a self,
        _: &'a AppState,
        title: String,
        text: String,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let mut runs = self.0.lock().unwrap();
            runs.push((title, text));
            Ok(format!("run-{}", runs.len()))
        })
    }
}

async fn state_with(mock: &Mock) -> (AppState, Arc<Recorder>) {
    let state = AppState::for_tests(vec![]);
    let base = mock.base.lock().unwrap().clone();
    state.workspace().set_setup(setup(&base));
    state.workspace().set_auto_check(false);
    let recorder = Arc::new(Recorder::default());
    state.workspace().set_sink(recorder.clone());
    (state, recorder)
}

fn mailbox(provider: MailProvider, email: &str) -> MailboxId {
    MailboxId::new(provider, email)
}

/// An account that reads `reads`, signed in, as connecting it in Settings ›
/// Email leaves it
async fn reading(state: &AppState, provider: MailProvider, email: &str, reads: Reads) {
    let token = json!({"access_token": "tok", "refresh_token": "ref",
        "expires_at": db::now_ms() / 1000 + 3600});
    state
        .secrets()
        .set(&mailbox(provider, email).token_entry(), &token.to_string())
        .unwrap();
    nandi_config::update_in(state.nandi_dir(), |config| {
        let settings = config.email.get_or_insert_with(EmailSettings::default);
        let mut account = EmailAccount::mailbox(provider, email);
        account.mail = reads.mail;
        for family in AccountFamily::ALL {
            if reads.has(family) {
                account.set_read(family, Some(ReadSignIn::Account));
            }
        }
        settings.accounts.push(account);
    })
    .unwrap();
    for family in AccountFamily::ALL {
        if reads.has(family) {
            started(state, AccountKind::of(provider, family)).unwrap();
        }
    }
}

fn calendar() -> Reads {
    Reads::MAIL.with(AccountFamily::Calendar, true)
}

fn files() -> Reads {
    Reads::MAIL.with(AccountFamily::Files, true)
}

fn source_of(state: &AppState, family: AccountFamily, email: &str) -> Source {
    sources(state, family)
        .into_iter()
        .find(|s| s.account() == email)
        .unwrap()
}

#[tokio::test]
async fn an_accounts_drive_and_calendar_are_nandis_until_the_last_account_stops() {
    let mock = mock().await;
    let (state, _) = state_with(&mock).await;
    reading(&state, MailProvider::Outlook, "dev@contoso.com", calendar()).await;
    reading(
        &state,
        MailProvider::Outlook,
        "dev@fabrikam.com",
        calendar(),
    )
    .await;
    assert!(state
        .grants()
        .allows(DEFAULT_AGENT, "outlook-calendar", GrantAccess::Read));
    assert_eq!(status(&state).await.connected, ["outlook-calendar"]);
    // Each account reads with its own sign-in and keeps its own cursor
    let work = source_of(&state, AccountFamily::Calendar, "dev@contoso.com");
    let other = source_of(&state, AccountFamily::Calendar, "dev@fabrikam.com");
    assert_ne!(work.entry, other.entry);
    assert_ne!(work.cursor_key(), other.cursor_key());
    assert_eq!(
        work.entry,
        mailbox(MailProvider::Outlook, "dev@contoso.com").token_entry()
    );
    assert!(
        work.reads.mail && work.reads.calendar,
        "renews with all it asked for"
    );

    // One account stops: the grant stays for the other
    save_setting(&state, work.cursor_key(), json!("d-1"))
        .await
        .unwrap();
    forget(&state, &work.mailbox, AccountFamily::Calendar)
        .await
        .unwrap();
    nandi_config::update_in(state.nandi_dir(), |config| {
        let settings = config.email.as_mut().unwrap();
        settings.accounts.retain(|a| !a.is(&work.mailbox));
    })
    .unwrap();
    stopped(&state, AccountKind::OutlookCalendar).unwrap();
    assert_eq!(load_cursor(&state, &work).await, Value::Null);
    assert!(state
        .grants()
        .allows(DEFAULT_AGENT, "outlook-calendar", GrantAccess::Read));
    // ...and goes with the last one
    nandi_config::update_in(state.nandi_dir(), |config| {
        config.email = Some(EmailSettings::default());
    })
    .unwrap();
    stopped(&state, AccountKind::OutlookCalendar).unwrap();
    assert!(!state
        .grants()
        .allows(DEFAULT_AGENT, "outlook-calendar", GrantAccess::Read));
    assert!(status(&state).await.connected.is_empty());
    state.shutdown();
}

#[tokio::test]
async fn one_nandi_run_per_calendar_check_naming_each_events_account() {
    let mock = mock().await;
    let (state, recorder) = state_with(&mock).await;
    reading(&state, MailProvider::Outlook, "dev@contoso.com", calendar()).await;
    reading(&state, MailProvider::Gmail, "dev@gmail.com", calendar()).await;
    // Mail only: its calendar isn't read
    reading(&state, MailProvider::Gmail, "home@gmail.com", Reads::MAIL).await;

    let first = check_calendars(&state).await;
    assert_eq!(first.found, 3);
    let runs = recorder.0.lock().unwrap().clone();
    assert_eq!(runs.len(), 1, "both calendars in one run");
    let (title, prompt) = &runs[0];
    assert_eq!(title, "Calendar: 3 changes");
    assert!(
        prompt.contains("outlook-calendar:e2 | account dev@contoso.com |")
            && prompt.contains("[time off, all day]"),
        "{prompt}"
    );
    assert!(prompt.contains("google-calendar:g1 | account dev@gmail.com |"));
    assert!(!prompt.contains("home@gmail.com"), "{prompt}");
    assert!(prompt.contains("<calendar-data>") && prompt.contains("first read"));
    let checked = calendar_check(&state, &mailbox(MailProvider::Gmail, "dev@gmail.com")).await;
    assert!(checked.last_checked_at.is_some() && checked.error.is_none());
    assert_eq!(checked.last_run_id.as_deref(), Some("run-1"));

    // The next check reads from each account's cursor: nothing new, no run
    let second = check_calendars(&state).await;
    assert_eq!(second, CheckOutcome::default());
    assert_eq!(recorder.0.lock().unwrap().len(), 1);
    let log = mock.log.lock().unwrap().clone();
    assert!(
        log.iter().any(|(_, p)| p.contains("deltatoken=d1")),
        "{log:?}"
    );
    assert!(
        log.iter().any(|(_, p)| p.contains("syncToken=s1")),
        "{log:?}"
    );
    assert!(log.iter().all(|(m, _)| m == "GET"), "read-only: {log:?}");
    state.shutdown();
}

#[tokio::test]
async fn a_feed_that_starts_over_sends_only_what_changed() {
    let mock = mock().await;
    let (state, recorder) = state_with(&mock).await;
    reading(&state, MailProvider::Outlook, "dev@contoso.com", calendar()).await;
    reading(&state, MailProvider::Gmail, "dev@gmail.com", calendar()).await;
    assert_eq!(check_calendars(&state).await.found, 3);

    // The weekly window move (or a feed with no token) reads it all again
    let both = sources(&state, AccountFamily::Calendar);
    for source in &both {
        save_setting(&state, source.cursor_key(), Value::Null)
            .await
            .unwrap();
    }
    assert_eq!(check_calendars(&state).await, CheckOutcome::default());
    assert_eq!(recorder.0.lock().unwrap().len(), 1, "no run for old events");
    // ...and the feeds carry on from new cursors
    for source in &both {
        assert!(!load_cursor(&state, source).await.is_null());
    }
    state.shutdown();
}

#[tokio::test]
async fn the_tools_read_events_free_busy_and_file_text_naming_accounts() {
    let mock = mock().await;
    let (state, _) = state_with(&mock).await;
    reading(
        &state,
        MailProvider::Outlook,
        "dev@contoso.com",
        calendar().with(AccountFamily::Files, true),
    )
    .await;
    reading(&state, MailProvider::Gmail, "dev@gmail.com", files()).await;
    let allowed: Vec<String> = ["outlook-calendar", "onedrive", "google-drive"]
        .map(String::from)
        .to_vec();

    let names: Vec<String> = tool::offered(&state, DEFAULT_AGENT, &[])
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(names.contains(&"calendar_events".to_string()));
    assert!(names.contains(&"files_read_text".to_string()));
    assert!(
        tool::offered(&state, "pr-reviewer", &[]).is_empty(),
        "no grant, not named"
    );
    assert_eq!(
        tool::offered(&state, "pr-reviewer", &["onedrive".into()]).len(),
        4,
        "named in agent.md: offered, asks on use"
    );

    // One calendar: its events don't need an account to tell them apart
    let events = tool::call(
        &state,
        &allowed,
        "calendar_events",
        &json!({"start": "2026-11-09"}),
    )
    .await;
    assert!(
        !events.is_error && events.result.contains("Quarterly review"),
        "{}",
        events.result
    );
    assert!(!events.result.contains("\"account\""), "{}", events.result);
    assert!(events.result.contains("untrusted data"));
    let busy = tool::call(
        &state,
        &allowed,
        "calendar_free_busy",
        &json!({"start": "2026-11-09", "end": "2026-11-10"}),
    )
    .await;
    assert!(busy.result.contains("busy"), "{}", busy.result);

    // Two drives: each file names its account
    let text = tool::call(
        &state,
        &allowed,
        "files_read_text",
        &json!({"id": "onedrive:f1"}),
    )
    .await;
    assert!(text.result.contains("Ship on Friday"), "{}", text.result);
    assert!(
        text.result.contains(r#""account":"dev@contoso.com""#),
        "{}",
        text.result
    );
    let search = tool::call(
        &state,
        &allowed,
        "files_search",
        &json!({"query": "budget"}),
    )
    .await;
    assert!(
        search.result.contains("google-drive:d1")
            && search.result.contains(r#""account":"dev@gmail.com""#),
        "{}",
        search.result
    );
    // `account` reads only that account
    let only = tool::call(
        &state,
        &allowed,
        "files_search",
        &json!({"query": "budget", "account": "DEV@contoso.com"}),
    )
    .await;
    assert!(!only.result.contains("google-drive:d1"), "{}", only.result);
    assert!(
        tool::call(
            &state,
            &allowed,
            "files_recent",
            &json!({"account": "nobody@x.com"})
        )
        .await
        .is_error
    );

    // Nothing allowed for this call: refused
    assert!(
        tool::call(&state, &[], "calendar_events", &json!({}))
            .await
            .is_error
    );
    let log = mock.log.lock().unwrap().clone();
    assert!(log.iter().all(|(m, _)| m == "GET"), "{log:?}");
    state.shutdown();
}

#[tokio::test]
async fn each_accounts_drive_has_its_own_change_feed() {
    let mock = mock().await;
    let (state, _) = state_with(&mock).await;
    reading(&state, MailProvider::Outlook, "dev@contoso.com", files()).await;
    reading(&state, MailProvider::Outlook, "dev@fabrikam.com", files()).await;
    let onedrive = ["onedrive".to_string()];

    // The first call starts each feed, the next lists each one's changes
    let started = tool::call(&state, &onedrive, "files_changes", &json!({})).await;
    assert!(!started.result.contains("Plan.md"), "{}", started.result);
    for email in ["dev@contoso.com", "dev@fabrikam.com"] {
        let source = source_of(&state, AccountFamily::Files, email);
        assert!(!load_cursor(&state, &source).await.is_null(), "{email}");
    }
    let changed = tool::call(&state, &onedrive, "files_changes", &json!({})).await;
    assert!(
        changed.result.contains(r#""account":"dev@contoso.com""#)
            && changed.result.contains(r#""account":"dev@fabrikam.com""#),
        "{}",
        changed.result
    );

    // Asking first names every account each connector covers
    let need = tool::access_need(&state, "files_recent", &json!({})).unwrap();
    assert_eq!(need.connectors.len(), 1);
    assert_eq!(
        need.details,
        ["OneDrive: dev@contoso.com, dev@fabrikam.com"]
    );
    state.shutdown();
}
