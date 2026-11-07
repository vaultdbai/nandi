//! Calendars and files against a mock Graph and Google on `127.0.0.1` (no
//! network): sign-in identity and grant, the calendar change feed and one
//! Chief of Staff run per check, the tools, the file change feed, and that
//! every request is a `GET` to the account's own API.

use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::response::IntoResponse;
use axum::Router;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::*;
use crate::services::email::provider::Endpoints;
use crate::services::email::Cloud;

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

async fn connected(state: &AppState, kind: AccountKind) {
    let token = json!({"access_token": "tok", "refresh_token": "ref",
        "expires_at": db::now_ms() / 1000 + 3600});
    state
        .secrets()
        .set(&kind.token_entry(), &token.to_string())
        .unwrap();
    connect(state, kind).await.unwrap();
}

#[tokio::test]
async fn connecting_asks_only_for_that_accounts_scopes_and_grants_the_chief_of_staff() {
    let oauth = setup("http://x").account_oauth(
        Cloud::Microsoft,
        "outlook-calendar",
        AccountKind::OutlookCalendar.scopes(),
    );
    assert!(oauth.scopes.contains(&"Calendars.Read".to_string()));
    assert!(
        !oauth.scopes.iter().any(|s| s.contains("Mail")),
        "incremental consent"
    );

    let mock = mock().await;
    let (state, _) = state_with(&mock).await;
    connected(&state, AccountKind::OutlookCalendar).await;
    let status = status(&state).await;
    let calendar = status
        .accounts
        .iter()
        .find(|a| a.kind == AccountKind::OutlookCalendar)
        .unwrap();
    assert!(calendar.connected);
    assert_eq!(calendar.account_email.as_deref(), Some("dev@contoso.com"));
    assert!(state
        .grants()
        .allows(CHIEF_OF_STAFF, "outlook-calendar", GrantAccess::Read));

    disconnect(&state, AccountKind::OutlookCalendar)
        .await
        .unwrap();
    assert!(!state
        .grants()
        .allows(CHIEF_OF_STAFF, "outlook-calendar", GrantAccess::Read));
    assert_eq!(
        state
            .secrets()
            .get(&AccountKind::OutlookCalendar.token_entry())
            .unwrap(),
        None
    );
    state.shutdown();
}

#[tokio::test]
async fn one_chief_of_staff_run_per_calendar_check_then_only_changes() {
    let mock = mock().await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, AccountKind::OutlookCalendar).await;
    connected(&state, AccountKind::GoogleCalendar).await;

    let first = check_calendars(&state).await;
    assert_eq!(first.found, 3);
    let runs = recorder.0.lock().unwrap().clone();
    assert_eq!(runs.len(), 1, "both calendars in one run");
    let (title, prompt) = &runs[0];
    assert_eq!(title, "Calendar: 3 changes");
    assert!(
        prompt.contains("outlook-calendar:e2") && prompt.contains("[time off, all day]"),
        "{prompt}"
    );
    assert!(prompt.contains("google-calendar:g1"));
    assert!(prompt.contains("<calendar-data>") && prompt.contains("first read"));

    // The next check reads from the feeds' cursors: nothing new, no run
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
async fn the_tools_read_events_free_busy_and_file_text() {
    let mock = mock().await;
    let (state, _) = state_with(&mock).await;
    connected(&state, AccountKind::OutlookCalendar).await;
    connected(&state, AccountKind::OneDrive).await;
    connected(&state, AccountKind::GoogleDrive).await;
    let allowed: Vec<String> = ["outlook-calendar", "onedrive", "google-drive"]
        .map(String::from)
        .to_vec();

    let names: Vec<String> = tool::offered(&state, CHIEF_OF_STAFF, &[])
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
        "named in skill.md: offered, asks on use"
    );

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
    assert!(events.result.contains("untrusted data"));
    let busy = tool::call(
        &state,
        &allowed,
        "calendar_free_busy",
        &json!({"start": "2026-11-09", "end": "2026-11-10"}),
    )
    .await;
    assert!(busy.result.contains("busy"), "{}", busy.result);

    let text = tool::call(
        &state,
        &allowed,
        "files_read_text",
        &json!({"id": "onedrive:f1"}),
    )
    .await;
    assert!(text.result.contains("Ship on Friday"), "{}", text.result);
    let search = tool::call(
        &state,
        &allowed,
        "files_search",
        &json!({"query": "budget"}),
    )
    .await;
    assert!(
        search.result.contains("google-drive:d1"),
        "{}",
        search.result
    );

    // The change feed: the first call starts it, the next lists changes
    let onedrive = ["onedrive".to_string()];
    let started = tool::call(&state, &onedrive, "files_changes", &json!({})).await;
    assert!(!started.result.contains("Plan.md"), "{}", started.result);
    let changed = tool::call(&state, &onedrive, "files_changes", &json!({})).await;
    assert!(changed.result.contains("Plan.md"), "{}", changed.result);

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
