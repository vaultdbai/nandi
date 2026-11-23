//! The email check against mock Graph and Gmail APIs (no network): delta
//! paging and cursor resume, expired cursors, bulk filtering, one run per
//! check, token refresh and refusal, sign-out, the tools, and that only
//! `GET`s ever reach a mail API.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::response::IntoResponse;
use axum::Router;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::mail::rfc3339_of;
use super::provider::Endpoints;
use super::*;
use crate::services::AppState;

// ---------------------------------------------------------------------------
// The mock
// ---------------------------------------------------------------------------

struct Req {
    method: String,
    path: String,
    query: HashMap<String, String>,
    base: String,
}

type Responder = Arc<dyn Fn(&Req) -> (u16, Value) + Send + Sync>;

#[derive(Clone)]
struct Mock {
    log: Arc<Mutex<Vec<(String, String)>>>,
    respond: Arc<Mutex<Responder>>,
    base: Arc<Mutex<String>>,
}

impl Mock {
    fn requests(&self) -> Vec<(String, String)> {
        self.log.lock().unwrap().clone()
    }

    fn api_requests(&self) -> Vec<(String, String)> {
        self.requests()
            .into_iter()
            .filter(|(_, path)| !path.ends_with("/token"))
            .collect()
    }

    fn clear(&self) {
        self.log.lock().unwrap().clear();
    }
}

async fn handle(State(mock): State<Mock>, request: Request) -> impl IntoResponse {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let query: HashMap<String, String> = request.uri().query().map(url_pairs).unwrap_or_default();
    mock.log
        .lock()
        .unwrap()
        .push((method.clone(), path.clone()));
    let respond = mock.respond.lock().unwrap().clone();
    let base = mock.base.lock().unwrap().clone();
    let (status, body) = respond(&Req {
        method,
        path,
        query,
        base,
    });
    (
        axum::http::StatusCode::from_u16(status).unwrap(),
        axum::Json(body),
    )
}

fn url_pairs(query: &str) -> HashMap<String, String> {
    reqwest::Url::parse(&format!("http://x/?{query}"))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

async fn mock(respond: impl Fn(&Req) -> (u16, Value) + Send + Sync + 'static) -> Mock {
    let mock = Mock {
        log: Arc::default(),
        respond: Arc::new(Mutex::new(Arc::new(respond))),
        base: Arc::default(),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    *mock.base.lock().unwrap() = base;
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

impl BatchSink for Recorder {
    fn submit<'a>(
        &'a self,
        _state: &'a AppState,
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

impl Recorder {
    fn runs(&self) -> Vec<(String, String)> {
        self.0.lock().unwrap().clone()
    }
}

fn unix_now() -> i64 {
    db::now_ms() / 1000
}

/// The mock's Outlook mailbox (Graph says `Dev@Company.com`)
fn outlook() -> MailboxId {
    MailboxId::new(MailProvider::Outlook, "dev@company.com")
}

/// The mock's Gmail mailbox
fn gmail() -> MailboxId {
    MailboxId::new(MailProvider::Gmail, "dev@gmail.com")
}

/// A sign-in at keyring `entry`
fn token(secrets: &crate::services::secrets::Secrets, entry: &str, expires_in: i64) {
    secrets
        .set(
            entry,
            &json!({
                "access_token": "tok-1",
                "refresh_token": "ref-1",
                "expires_at": unix_now() + expires_in,
            })
            .to_string(),
        )
        .unwrap();
}

async fn state_with(mock: &Mock) -> (AppState, Arc<Recorder>) {
    let state = AppState::for_tests(vec![]);
    let base = mock.base.lock().unwrap().clone();
    state.email().set_setup(setup(&base));
    state.email().set_auto_check(false);
    state.workspace().set_auto_check(false);
    let recorder = Arc::new(Recorder::default());
    state.email().set_sink(recorder.clone());
    (state, recorder)
}

/// Signs in to `provider` as far as the browser would, then connects
async fn connected(state: &AppState, provider: MailProvider) {
    let entry = provider.sign_in_entry("test");
    token(state.secrets(), &entry, 3600);
    connect(state, provider, &entry, Reads::MAIL).await.unwrap();
}

// ---------------------------------------------------------------------------
// Outlook
// ---------------------------------------------------------------------------

fn graph_message(id: &str, from: &str, subject: &str, class: &str, hours_ago: i64) -> Value {
    json!({
        "id": id,
        "conversationId": format!("conv-{id}"),
        "subject": subject,
        "from": {"emailAddress": {"name": from, "address": format!("{}@contoso.com", from.to_lowercase())}},
        "toRecipients": [{"emailAddress": {"name": "Dev Chechi", "address": "dev@company.com"}}],
        "receivedDateTime": rfc3339_of(unix_now() - hours_ago * 3600),
        "bodyPreview": format!("{subject} preview"),
        "inferenceClassification": class,
    })
}

/// A Graph mailbox: page 1 → next link → page 2 → delta link, then one
/// more message on the next check
fn outlook_api(req: &Req) -> (u16, Value) {
    let graph = format!("{}/graph", req.base);
    let p = req.path.as_str();
    if req.method != "GET" {
        return (405, json!({}));
    }
    match p {
        "/graph/me" => (
            200,
            json!({"displayName": "Dev Chechi", "mail": "Dev@Company.com", "jobTitle": "Platform lead", "department": "Platform engineering"}),
        ),
        "/graph/me/manager" => (
            200,
            json!({"displayName": "Priya Rao", "mail": "priya@company.com", "jobTitle": "VP Engineering"}),
        ),
        "/graph/me/mailFolders/inbox/messages/delta" => {
            match req.query.get("$deltatoken").map(String::as_str) {
                None if req.query.contains_key("page") => (
                    200,
                    json!({
                        "value": [graph_message("m3", "News", "Weekly digest", "focused", 3)],
                        "@odata.deltaLink": format!("{graph}/me/mailFolders/inbox/messages/delta?$deltatoken=inbox-1"),
                    }),
                ),
                None => {
                    assert!(
                        req.query
                            .get("$filter")
                            .is_some_and(|f| f.starts_with("receivedDateTime ge ")),
                        "the first read asks for the window only"
                    );
                    (
                        200,
                        json!({
                            "value": [
                                graph_message("m1", "Lena", "Q3 audit evidence", "focused", 2),
                                graph_message("m2", "Promo", "Sale", "other", 2),
                            ],
                            "@odata.nextLink": format!("{graph}/me/mailFolders/inbox/messages/delta?page=2"),
                        }),
                    )
                }
                Some("inbox-1") => (
                    200,
                    json!({
                        "value": [graph_message("m4", "Sam", "Budget sign-off by Monday", "focused", 1)],
                        "@odata.deltaLink": format!("{graph}/me/mailFolders/inbox/messages/delta?$deltatoken=inbox-2"),
                    }),
                ),
                Some("expired") => (410, json!({"error": {"code": "SyncStateNotFound"}})),
                Some(_) => (
                    200,
                    json!({"value": [], "@odata.deltaLink": format!("{graph}/me/mailFolders/inbox/messages/delta?$deltatoken=inbox-2")}),
                ),
            }
        }
        "/graph/me/mailFolders/sentitems/messages/delta" => (
            200,
            json!({
                "value": if req.query.contains_key("$deltatoken") { json!([]) } else { json!([{
                    "id": "s1",
                    "conversationId": "conv-s1",
                    "subject": "Re: offsite",
                    "from": {"emailAddress": {"name": "Dev Chechi", "address": "dev@company.com"}},
                    "toRecipients": [{"emailAddress": {"address": "lena@contoso.com"}}],
                    "receivedDateTime": rfc3339_of(unix_now() - 7200),
                    "bodyPreview": "I'll book the room by Thursday",
                }, {
                    "id": "s2",
                    "conversationId": "conv-s2",
                    "subject": "note to self",
                    "from": {"emailAddress": {"address": "dev@company.com"}},
                    "toRecipients": [{"emailAddress": {"address": "dev@company.com"}}],
                    "receivedDateTime": rfc3339_of(unix_now() - 7200),
                }]) },
                "@odata.deltaLink": format!("{graph}/me/mailFolders/sentitems/messages/delta?$deltatoken=sent-1"),
            }),
        ),
        "/graph/me/messages/m3" => (
            200,
            json!({"internetMessageHeaders": [{"name": "List-Unsubscribe", "value": "<mailto:u@news.com>"}]}),
        ),
        "/graph/me/messages/m1" if req.query.get("$select").is_some_and(|s| s.contains("body")) => {
            (200, {
                let mut m = graph_message("m1", "Lena", "Q3 audit evidence", "focused", 2);
                m["body"] = json!({"contentType": "text", "content": format!("Can you send the evidence list by Friday?\n\nOn Mon, Lena wrote:\n> {}", "old text ".repeat(50))});
                m
            })
        }
        _ if p.starts_with("/graph/me/messages/") => (200, json!({"internetMessageHeaders": []})),
        "/graph/me/mailFolders/inbox/messages" | "/graph/me/messages" => (
            200,
            json!({"value": [graph_message("m1", "Lena", "Q3 audit evidence", "focused", 2)]}),
        ),
        "/graph/me/mailFolders/sentitems/messages" => (200, json!({"value": []})),
        _ => (404, json!({"error": "unknown"})),
    }
}

#[tokio::test]
async fn outlook_pages_to_the_delta_link_and_resumes_from_it() {
    let mock = mock(outlook_api).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;

    // Connecting set up the account and Nandi's read grant
    let status = status(&state).await;
    assert!(status.providers[0].connected && status.providers[0].available);
    let outlook = &status.mailboxes[0];
    assert_eq!(outlook.email, "dev@company.com");
    assert_eq!(outlook.name.as_deref(), Some("Dev Chechi"));
    let profile = outlook.profile.as_ref().unwrap();
    assert_eq!(profile.title.as_deref(), Some("Platform lead"));
    assert_eq!(profile.manager.as_ref().unwrap().name, "Priya Rao");
    assert_eq!(outlook.state, EmailAccountState::Ready);
    assert_eq!(granted(&state, DEFAULT_AGENT).len(), 1);

    let first = check_all(&state).await;
    // Lena, and the user's own sent mail; not the "other" promo, the
    // List-Unsubscribe digest or the note to self
    assert_eq!(first.found, 2);
    assert_eq!(first.run_id.as_deref(), Some("run-1"));
    let runs = recorder.runs();
    assert_eq!(runs.len(), 1);
    let text = &runs[0].1;
    assert!(text.contains("first read, of the last 14 days"));
    assert!(text.contains("Q3 audit evidence"));
    assert!(text.contains("Re: offsite"));
    assert!(
        !text.contains("Sale") && !text.contains("Weekly digest") && !text.contains("note to self")
    );
    assert!(text.contains("for Dev Chechi <dev@company.com>"));

    // The next check starts from the stored delta links
    mock.clear();
    let second = check_all(&state).await;
    assert_eq!(second.found, 1);
    let runs = recorder.runs();
    assert_eq!(runs.len(), 2);
    assert!(runs[1].1.contains("Budget sign-off by Monday"));
    assert!(!runs[1].1.contains("first read"));
    assert!(
        mock.requests()
            .iter()
            .all(|(_, path)| !path.ends_with("/token")),
        "the token was still good"
    );

    // Nothing new: no run at all
    let third = check_all(&state).await;
    assert_eq!(third, CheckOutcome::default());
    assert_eq!(recorder.runs().len(), 2);
    let status = super::status(&state).await;
    assert_eq!(status.mailboxes[0].last_found, 0);
    assert!(status.mailboxes[0].last_checked_at.is_some());
    assert!(status.next_check_at.is_some());
    state.shutdown();
}

#[tokio::test]
async fn an_expired_delta_link_reads_the_window_again() {
    let mock = mock(outlook_api).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    let graph = format!("{}/graph", mock.base.lock().unwrap());
    save_setting(
        &state,
        outlook().cursor_key(),
        json!({
            "inbox": format!("{graph}/me/mailFolders/inbox/messages/delta?$deltatoken=expired"),
            "sentitems": format!("{graph}/me/mailFolders/sentitems/messages/delta?$deltatoken=sent-1"),
        }),
    )
    .await
    .unwrap();
    let outcome = check_all(&state).await;
    assert_eq!(outcome.found, 1, "Lena again, from the window");
    assert!(!recorder.runs()[0].1.contains("first read"));
    let cursor: Value = load_setting(&state, outlook().cursor_key()).await;
    assert!(cursor["inbox"].as_str().unwrap().ends_with("inbox-1"));
    state.shutdown();
}

#[tokio::test]
async fn watched_senders_come_through_even_as_bulk() {
    let mock = mock(outlook_api).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    set_settings(
        &state,
        EmailSettingsInput {
            interval_minutes: 10,
            watch: vec!["promo@contoso.com".into()],
            backfill: BackfillRange::Everything,
        },
    )
    .await
    .unwrap();
    assert_eq!(check_all(&state).await.found, 3);
    assert!(recorder.runs()[0].1.contains("Sale"));
    assert_eq!(super::status(&state).await.interval_minutes, 10);
    state.shutdown();
}

/// The mailboxes `agent` has a read grant on (what `services::access`
/// would let a call use)
fn allowed(state: &AppState, agent: &str) -> Vec<String> {
    granted(state, agent)
        .into_iter()
        .map(|a| a.provider.id().to_string())
        .collect()
}

#[tokio::test]
async fn only_gets_ever_reach_the_mail_apis() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    check_all(&state).await;
    check_all(&state).await;
    for name in [tool::LIST_NEW, tool::SEARCH] {
        tool::call(
            &state,
            DEFAULT_AGENT,
            &allowed(&state, DEFAULT_AGENT),
            name,
            &json!({"query": "audit"}),
        )
        .await;
    }
    tool::call(
        &state,
        DEFAULT_AGENT,
        &allowed(&state, DEFAULT_AGENT),
        tool::GET_MESSAGE,
        &json!({"id": "outlook:m1"}),
    )
    .await;
    tool::call(
        &state,
        DEFAULT_AGENT,
        &allowed(&state, DEFAULT_AGENT),
        tool::GET_THREAD,
        &json!({"thread_id": "outlook:conv-m1"}),
    )
    .await;
    let requests = mock.api_requests();
    assert!(requests.len() > 8, "{requests:?}");
    for (method, path) in &requests {
        assert_eq!(method, "GET", "{path}");
        assert!(path.starts_with("/graph/me"), "{path}");
        for verb in ["send", "move", "copy", "reply", "forward", "createReply"] {
            assert!(
                !path
                    .to_ascii_lowercase()
                    .ends_with(&verb.to_ascii_lowercase()),
                "{path}"
            );
        }
    }
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_expired_token_is_refreshed_before_the_check() {
    let mock = mock(|req| {
        if req.path == "/ms/token" {
            assert_eq!(req.method, "POST");
            return (200, json!({"access_token": "tok-2", "expires_in": 3600}));
        }
        outlook_api(req)
    })
    .await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    token(state.secrets(), &outlook().token_entry(), -10);
    assert_eq!(check_all(&state).await.found, 2);
    assert!(mock
        .requests()
        .contains(&("POST".into(), "/ms/token".into())));
    let stored = state
        .secrets()
        .get(&outlook().token_entry())
        .unwrap()
        .unwrap();
    assert!(stored.contains("tok-2") && stored.contains("ref-1"));
    state.shutdown();
}

#[tokio::test]
async fn a_401_renews_the_token_once_and_retries() {
    let renewed = Arc::new(Mutex::new(false));
    let flag = renewed.clone();
    let mock = mock(move |req| {
        if req.path == "/ms/token" {
            *flag.lock().unwrap() = true;
            return (200, json!({"access_token": "tok-2", "expires_in": 3600}));
        }
        if !*flag.lock().unwrap() && req.path != "/graph/me" {
            return (
                401,
                json!({"error": {"code": "InvalidAuthenticationToken"}}),
            );
        }
        outlook_api(req)
    })
    .await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    assert_eq!(check_all(&state).await.found, 2);
    assert!(*renewed.lock().unwrap());
    state.shutdown();
}

#[tokio::test]
async fn a_refused_refresh_asks_for_a_new_sign_in_and_keeps_the_cursor() {
    let mock = mock(|req| {
        if req.path == "/gtoken/token" {
            return (
                400,
                json!({"error": "invalid_grant", "error_description": "Token has been expired or revoked."}),
            );
        }
        gmail_api(req)
    })
    .await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Gmail).await;
    check_all(&state).await;
    let cursor: Value = load_setting(&state, gmail().cursor_key()).await;
    // Google's Testing mode: the sign-in ends after 7 days
    token(state.secrets(), &gmail().token_entry(), -10);
    let outcome = check_all(&state).await;
    assert_eq!(outcome, CheckOutcome::default());
    assert_eq!(recorder.runs().len(), 1, "no run without mail");
    let status = super::status(&state).await;
    let card = &status.mailboxes[0];
    assert_eq!(card.state, EmailAccountState::NeedsSignIn);
    assert!(card.error.as_deref().unwrap().contains("invalid_grant"));
    let after: Value = load_setting(&state, gmail().cursor_key()).await;
    assert_eq!(cursor, after, "the cursor waits for the new sign-in");

    // Signing in again clears it and carries on from the same cursor
    connected(&state, MailProvider::Gmail).await;
    assert_eq!(
        super::status(&state).await.mailboxes[0].state,
        EmailAccountState::Ready
    );
    let resumed: Value = load_setting(&state, gmail().cursor_key()).await;
    assert_eq!(cursor, resumed);
    state.shutdown();
}

#[tokio::test]
async fn disconnecting_wipes_the_sign_in_the_cursor_and_the_grant() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    check_all(&state).await;
    let cursor_key = outlook().cursor_key();
    let cursor: Value = load_setting(&state, cursor_key.clone()).await;
    assert!(!cursor.is_null());

    let status = disconnect(&state, outlook()).await.unwrap();
    assert!(status.mailboxes.is_empty());
    assert!(!status.providers[0].connected);
    assert_eq!(state.secrets().get(&outlook().token_entry()).unwrap(), None);
    let cursor: Value = load_setting(&state, cursor_key).await;
    assert!(cursor.is_null());
    assert!(state.grants().list().is_empty());
    assert!(tool::offered(&state, DEFAULT_AGENT, &[]).is_empty());
    // Nothing to check any more
    mock.clear();
    assert_eq!(check_all(&state).await, CheckOutcome::default());
    assert!(mock.requests().is_empty());
    state.shutdown();
}

#[tokio::test]
async fn the_mail_check_trigger_waits_for_a_mailbox() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    let guard = MailCheckGuard::new(&state);
    assert_eq!(
        guard.hold(MAIL_CHECK).as_deref(),
        Some("Skipped: no mailbox is connected")
    );
    assert_eq!(
        guard.hold(crate::services::builtin_agents::MORNING_DIGEST),
        None,
        "only the mail check"
    );

    connected(&state, MailProvider::Outlook).await;
    assert_eq!(guard.hold(MAIL_CHECK), None);
    state.shutdown();
}

#[tokio::test]
async fn paused_mailboxes_are_not_read() {
    let mock = mock(outlook_api).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    let status = set_paused(&state, outlook(), true).await.unwrap();
    assert_eq!(status.mailboxes[0].state, EmailAccountState::Paused);
    assert_eq!(status.next_check_at, None);
    mock.clear();
    assert_eq!(check_all(&state).await, CheckOutcome::default());
    assert!(mock.requests().is_empty() && recorder.runs().is_empty());
    assert!(set_paused(&state, gmail(), true).await.is_err());
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Gmail, and both together
// ---------------------------------------------------------------------------

fn gmail_message(id: &str, labels: &[&str], from: &str, subject: &str, hours_ago: i64) -> Value {
    json!({
        "id": id,
        "threadId": format!("t-{id}"),
        "labelIds": labels,
        "snippet": format!("{subject} snippet"),
        "internalDate": ((unix_now() - hours_ago * 3600) * 1000).to_string(),
        "payload": {"headers": [
            {"name": "From", "value": from},
            {"name": "To", "value": "Dev Chechi <dev@gmail.com>"},
            {"name": "Subject", "value": subject},
        ]},
    })
}

fn gmail_api(req: &Req) -> (u16, Value) {
    if req.method != "GET" {
        return (405, json!({}));
    }
    match req.path.as_str() {
        "/gmail/users/me/profile" => (
            200,
            json!({"emailAddress": "dev@gmail.com", "historyId": "100"}),
        ),
        "/userinfo" => (200, json!({"name": "Dev Chechi"})),
        "/gmail/users/me/messages" => {
            let q = req.query.get("q").cloned().unwrap_or_default();
            assert!(q.starts_with("after:") || q == "evidence", "{q}");
            (200, json!({"messages": [{"id": "g1"}, {"id": "g2"}]}))
        }
        "/gmail/users/me/history" => match req.query.get("startHistoryId").map(String::as_str) {
            Some("100") => (
                200,
                json!({
                    "historyId": "120",
                    "history": [{"messagesAdded": [
                        {"message": {"id": "g3", "labelIds": ["INBOX"]}},
                        {"message": {"id": "g4", "labelIds": ["DRAFT"]}},
                    ]}],
                }),
            ),
            Some("1") => (404, json!({"error": {"code": 404}})),
            _ => (200, json!({"historyId": "120"})),
        },
        "/gmail/users/me/messages/g1" => (
            200,
            gmail_message(
                "g1",
                &["INBOX"],
                "Lena Park <lena@contoso.com>",
                "Vendor contract due",
                5,
            ),
        ),
        "/gmail/users/me/messages/g2" => (
            200,
            gmail_message(
                "g2",
                &["INBOX", "CATEGORY_PROMOTIONS"],
                "Shop <deals@shop.com>",
                "50% off",
                5,
            ),
        ),
        "/gmail/users/me/messages/g3" => (
            200,
            gmail_message(
                "g3",
                &["INBOX"],
                "Sam <sam@contoso.com>",
                "Can you own the rota?",
                1,
            ),
        ),
        _ => (404, json!({})),
    }
}

#[tokio::test]
async fn gmail_reads_the_window_then_the_history() {
    let mock = mock(gmail_api).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Gmail).await;
    assert_eq!(
        super::status(&state).await.mailboxes[0].name.as_deref(),
        Some("Dev Chechi")
    );
    assert_eq!(
        check_all(&state).await.found,
        1,
        "the promotion is left out"
    );
    assert!(recorder.runs()[0].1.contains("Vendor contract due"));
    let cursor: Value = load_setting(&state, gmail().cursor_key()).await;
    assert_eq!(cursor, json!({"historyId": "100"}));

    assert_eq!(check_all(&state).await.found, 1);
    assert!(recorder.runs()[1].1.contains("Can you own the rota?"));
    let cursor: Value = load_setting(&state, gmail().cursor_key()).await;
    assert_eq!(cursor, json!({"historyId": "120"}));
    // The draft was never read
    assert!(!mock.requests().iter().any(|(_, p)| p.ends_with("/g4")));
    state.shutdown();
}

#[tokio::test]
async fn gmail_history_too_old_reads_the_window_again() {
    let mock = mock(gmail_api).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Gmail).await;
    save_setting(&state, gmail().cursor_key(), json!({"historyId": "1"}))
        .await
        .unwrap();
    assert_eq!(check_all(&state).await.found, 1);
    assert!(
        !recorder.runs()[0].1.contains("first read"),
        "only a new cursor is"
    );
    let cursor: Value = load_setting(&state, gmail().cursor_key()).await;
    assert_eq!(cursor, json!({"historyId": "100"}));
    state.shutdown();
}

#[tokio::test]
async fn a_gmail_history_too_long_for_one_read_moves_the_cursor_only_as_far_as_read() {
    // Every page has one more record and another page after it
    let mock = mock(|req| {
        if req.path != "/gmail/users/me/history" {
            return gmail_api(req);
        }
        let page: u32 = req
            .query
            .get("pageToken")
            .map_or(0, |token| token.parse().unwrap());
        (
            200,
            json!({
                "historyId": "999",
                "history": [{"id": (201 + page).to_string(), "messagesAdded": [
                    {"message": {"id": "g3", "labelIds": ["INBOX"]}},
                ]}],
                "nextPageToken": (page + 1).to_string(),
            }),
        )
    })
    .await;
    let (state, _) = state_with(&mock).await;
    connected(&state, MailProvider::Gmail).await;
    save_setting(&state, gmail().cursor_key(), json!({"historyId": "200"}))
        .await
        .unwrap();

    assert_eq!(check_all(&state).await.found, 1);
    let cursor: Value = load_setting(&state, gmail().cursor_key()).await;
    // Ten pages read: up to the tenth record, not the mailbox's newest
    assert_eq!(cursor, json!({"historyId": "210"}));
    state.shutdown();
}

#[tokio::test]
async fn both_mailboxes_go_to_nandi_in_one_run() {
    let mock = mock(|req| {
        if req.path.starts_with("/graph") {
            outlook_api(req)
        } else {
            gmail_api(req)
        }
    })
    .await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    connected(&state, MailProvider::Gmail).await;
    let outcome = check_all(&state).await;
    assert_eq!(outcome.found, 3);
    let runs = recorder.runs();
    assert_eq!(runs.len(), 1, "one run per check");
    assert_eq!(runs[0].0, "Email: 3 new emails");
    let text = &runs[0].1;
    assert!(text.contains("Dev Chechi <dev@company.com> and Dev Chechi <dev@gmail.com>"));
    assert!(text.contains("id: outlook:m1") && text.contains("id: gmail:g1"));
    let status = super::status(&state).await;
    assert_eq!(status.mailboxes[0].last_found, 2);
    assert_eq!(status.mailboxes[1].last_found, 1);
    assert_eq!(status.mailboxes[1].last_run_id.as_deref(), Some("run-1"));
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Several mailboxes of one provider
// ---------------------------------------------------------------------------

/// The person's second Gmail
fn home() -> MailboxId {
    MailboxId::new(MailProvider::Gmail, "dev.home@gmail.com")
}

/// The mock Gmail, signed in as `address`
fn gmail_as(mock: &Mock, address: &'static str) {
    *mock.respond.lock().unwrap() = Arc::new(move |req| {
        if req.path == "/gmail/users/me/profile" {
            return (200, json!({"emailAddress": address, "historyId": "100"}));
        }
        gmail_api(req)
    });
}

/// Signs in to Gmail with an ID token that names `address`, then connects
async fn connected_gmail_with_id(state: &AppState, address: &str) {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    let claims = json!({"email": address, "email_verified": true});
    let id_token = format!("h.{}.s", URL_SAFE_NO_PAD.encode(claims.to_string()));
    let entry = MailProvider::Gmail.sign_in_entry("test");
    state
        .secrets()
        .set(
            &entry,
            &json!({
                "access_token": "tok-1",
                "refresh_token": "ref-1",
                "expires_at": unix_now() + 3600,
                "id_token": id_token,
            })
            .to_string(),
        )
        .unwrap();
    connect(state, MailProvider::Gmail, &entry, Reads::MAIL)
        .await
        .unwrap();
}

fn addresses(state: &AppState) -> Vec<String> {
    email_settings(state)
        .accounts
        .into_iter()
        .map(|a| a.email)
        .collect()
}

#[tokio::test]
async fn a_second_gmail_is_added_and_the_first_stays_the_identity() {
    let mock = mock(gmail_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected_gmail_with_id(&state, "dev@gmail.com").await;
    check_all(&state).await;
    let work_cursor: Value = load_setting(&state, gmail().cursor_key()).await;
    assert!(!work_cursor.is_null());

    gmail_as(&mock, "dev.home@gmail.com");
    connected_gmail_with_id(&state, "dev.home@gmail.com").await;
    assert_eq!(addresses(&state), ["dev@gmail.com", "dev.home@gmail.com"]);
    // Each sign-in in its own entry; nothing left at the sign-in entry
    for mailbox in [gmail(), home()] {
        assert!(state
            .secrets()
            .get(&mailbox.token_entry())
            .unwrap()
            .is_some());
    }
    assert_eq!(
        state
            .secrets()
            .get(&MailProvider::Gmail.sign_in_entry("test"))
            .unwrap(),
        None
    );
    let identity = crate::services::identity::identity(&state).unwrap();
    assert_eq!(identity.email, "dev@gmail.com");
    // Settings lists both, in the order connected; only the first is the
    // identity
    let listed = super::status(&state).await.mailboxes;
    let shown: Vec<_> = listed
        .iter()
        .map(|m| (m.email.as_str(), m.is_identity))
        .collect();
    assert_eq!(
        shown,
        [("dev@gmail.com", true), ("dev.home@gmail.com", false)]
    );

    // Both are read; the first kept its cursor, the new one got its own
    check_all(&state).await;
    let home_cursor: Value = load_setting(&state, home().cursor_key()).await;
    assert_eq!(home_cursor, json!({"historyId": "100"}));
    assert!(check_state(&state, &gmail())
        .await
        .last_checked_at
        .is_some());
    assert!(check_state(&state, &home()).await.last_checked_at.is_some());
    state.shutdown();
}

#[tokio::test]
async fn reconnecting_the_same_address_keeps_its_place_and_cursor() {
    let mock = mock(gmail_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Gmail).await;
    gmail_as(&mock, "dev.home@gmail.com");
    connected(&state, MailProvider::Gmail).await;
    check_all(&state).await;
    let cursor: Value = load_setting(&state, gmail().cursor_key()).await;
    assert!(!cursor.is_null());

    // The first mailbox again, in another case
    gmail_as(&mock, "Dev@Gmail.com");
    connected(&state, MailProvider::Gmail).await;
    assert_eq!(addresses(&state), ["dev@gmail.com", "dev.home@gmail.com"]);
    let kept: Value = load_setting(&state, gmail().cursor_key()).await;
    assert_eq!(cursor, kept);
    state.shutdown();
}

#[tokio::test]
async fn signing_out_the_identity_mailbox_leaves_the_identity_where_it_is() {
    let mock = mock(gmail_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected_gmail_with_id(&state, "dev@gmail.com").await;
    gmail_as(&mock, "dev.home@gmail.com");
    connected_gmail_with_id(&state, "dev.home@gmail.com").await;

    disconnect(&state, gmail()).await.unwrap();
    // The other mailbox doesn't inherit it: relay and trial ask for the
    // identity's own mailbox to be connected again
    let identity = crate::services::identity::identity(&state).unwrap();
    assert_eq!(identity.email, "dev@gmail.com");
    let listed = super::status(&state).await.mailboxes;
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].is_identity);

    // Connecting it again makes it the identity's mailbox once more
    gmail_as(&mock, "dev@gmail.com");
    connected_gmail_with_id(&state, "dev@gmail.com").await;
    let listed = super::status(&state).await.mailboxes;
    assert_eq!(listed.len(), 2);
    assert!(listed
        .iter()
        .any(|m| m.email == "dev@gmail.com" && m.is_identity));
    state.shutdown();
}

#[tokio::test]
async fn a_sign_in_that_cant_be_saved_leaves_no_secret_behind() {
    let mock = mock(gmail_api).await;
    let (state, _recorder) = state_with(&mock).await;
    let entry = MailProvider::Gmail.sign_in_entry("test");
    token(state.secrets(), &entry, 3600);
    // config.json can't be read, so the account can't be saved
    std::fs::create_dir_all(state.nandi_dir()).unwrap();
    std::fs::write(state.nandi_dir().join("config.json"), "{ not json").unwrap();

    assert!(connect(&state, MailProvider::Gmail, &entry, Reads::MAIL)
        .await
        .is_err());
    assert_eq!(state.secrets().get(&entry).unwrap(), None);
    assert_eq!(state.secrets().get(&gmail().token_entry()).unwrap(), None);
    assert!(super::connected(&state).is_empty());
    state.shutdown();
}

#[tokio::test]
async fn each_sign_in_attempt_has_its_own_entry() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    start_sign_in(&state, MailProvider::Outlook, Reads::MAIL)
        .await
        .unwrap();
    let first = lock(&state.email().sign_ins)[&MailProvider::Outlook]
        .entry
        .clone();
    // Something half-way in the first attempt's entry
    token(state.secrets(), &first, 3600);

    start_sign_in(&state, MailProvider::Outlook, Reads::MAIL)
        .await
        .unwrap();
    let second = lock(&state.email().sign_ins)[&MailProvider::Outlook]
        .entry
        .clone();
    assert_ne!(first, second, "a new attempt never shares an entry");
    assert_eq!(
        state.secrets().get(&first).unwrap(),
        None,
        "the replaced attempt's token is removed"
    );
    // Polling follows the attempt now in progress
    assert_eq!(
        poll_sign_in(&state, MailProvider::Outlook).await.phase,
        SignInPhase::Waiting
    );
    // No attempt for the other provider
    assert_eq!(
        poll_sign_in(&state, MailProvider::Gmail).await.phase,
        SignInPhase::Failed
    );
    // Only the attempt in progress is on the record
    assert_eq!(pending_sign_ins(&state).await, [key_of(&second)]);

    // Nandi exits mid sign-in with a token in the entry: the next start
    // removes it
    token(state.secrets(), &second, 3600);
    let conn = state.db().conn().unwrap();
    assert_eq!(
        super::sign_ins::clean_abandoned(&conn, state.secrets()).unwrap(),
        1
    );
    drop(conn);
    assert_eq!(state.secrets().get(&second).unwrap(), None);
    assert!(pending_sign_ins(&state).await.is_empty());
    state.shutdown();
}

fn key_of(entry: &str) -> String {
    format!("email.sign-ins.{entry}")
}

async fn pending_sign_ins(state: &AppState) -> Vec<String> {
    state
        .db()
        .call(|conn| crate::db::setting_keys_with_prefix(conn, "email.sign-ins."))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_finished_sign_in_leaves_no_record() {
    let mock = mock(gmail_api).await;
    let (state, _recorder) = state_with(&mock).await;
    start_sign_in(&state, MailProvider::Gmail, Reads::MAIL)
        .await
        .unwrap();
    assert_eq!(pending_sign_ins(&state).await.len(), 1);
    let entry = lock(&state.email().sign_ins)[&MailProvider::Gmail]
        .entry
        .clone();
    // The sign-in fails (its flow is gone, no token came): the record goes
    state.email().flows.cancel(&entry);
    assert_eq!(
        poll_sign_in(&state, MailProvider::Gmail).await.phase,
        SignInPhase::Failed
    );
    assert!(pending_sign_ins(&state).await.is_empty());
    // One that connects leaves none either
    start_sign_in(&state, MailProvider::Gmail, Reads::MAIL)
        .await
        .unwrap();
    let entry = lock(&state.email().sign_ins)[&MailProvider::Gmail]
        .entry
        .clone();
    token(state.secrets(), &entry, 3600);
    state.email().flows.cancel(&entry);
    assert_eq!(
        poll_sign_in(&state, MailProvider::Gmail).await.phase,
        SignInPhase::Connected
    );
    assert_eq!(state.secrets().get(&entry).unwrap(), None);
    assert!(pending_sign_ins(&state).await.is_empty());
    state.shutdown();
}

#[tokio::test]
async fn signing_out_one_of_two_keeps_the_other_and_nandis_access() {
    let mock = mock(gmail_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Gmail).await;
    gmail_as(&mock, "dev.home@gmail.com");
    connected(&state, MailProvider::Gmail).await;
    check_all(&state).await;

    disconnect(&state, gmail()).await.unwrap();
    assert_eq!(addresses(&state), ["dev.home@gmail.com"]);
    assert_eq!(state.secrets().get(&gmail().token_entry()).unwrap(), None);
    let gone: Value = load_setting(&state, gmail().cursor_key()).await;
    assert!(gone.is_null());
    assert!(state
        .secrets()
        .get(&home().token_entry())
        .unwrap()
        .is_some());
    let kept: Value = load_setting(&state, home().cursor_key()).await;
    assert!(!kept.is_null());
    assert_eq!(allowed(&state, DEFAULT_AGENT), ["gmail"]);
    // The other one is all Settings lists now
    let listed = super::status(&state).await.mailboxes;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].email, "dev.home@gmail.com");

    // Nandi's access goes with the last Gmail
    disconnect(&state, home()).await.unwrap();
    assert!(addresses(&state).is_empty());
    assert!(state.grants().list().is_empty());
    state.shutdown();
}

#[tokio::test]
async fn an_agent_reads_only_the_mailboxes_its_grant_names() {
    use crate::services::domain::grant::{ConnectorGrant, GrantAccess, GrantScope};
    let mock = mock(gmail_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Gmail).await;
    gmail_as(&mock, "dev.home@gmail.com");
    connected(&state, MailProvider::Gmail).await;
    let read = GrantAccess::Read;
    // Each connect let Nandi read that mailbox
    assert_eq!(
        state.grants().mailboxes(DEFAULT_AGENT, "gmail", read),
        Some(vec![
            "dev.home@gmail.com".to_string(),
            "dev@gmail.com".to_string()
        ])
    );
    let home_only = ConnectorGrant::new("pr-reviewer", "gmail", read)
        .and_then(|g| g.for_mailboxes(&["dev.home@gmail.com".to_string()]))
        .unwrap();
    state.grants().grant(home_only, GrantScope::Saved).unwrap();
    let gmail_ids = ["gmail".to_string()];

    // Without a mailbox: only what the grant covers, so no `mailbox` tag
    let list = tool::call(
        &state,
        "pr-reviewer",
        &gmail_ids,
        tool::LIST_NEW,
        &json!({}),
    )
    .await;
    assert!(!list.is_error, "{}", list.result);
    assert!(!list.result.contains("\"mailbox\""), "{}", list.result);
    // Naming one it doesn't cover asks the person first, showing that one
    let work = json!({"mailbox": "Dev@Gmail.com"});
    use crate::services::access::{self, Decision};
    let decided = access::decide(&state, None, "pr-reviewer", "run-1", tool::LIST_NEW, &work).await;
    let Some(Decision::Ask(request)) = decided else {
        panic!("expected a request: {decided:?}");
    };
    assert_eq!(request.mailboxes, ["dev@gmail.com"]);
    // The one it covers runs at once
    let home_call = json!({"mailbox": "dev.home@gmail.com"});
    let decided = access::decide(
        &state,
        None,
        "pr-reviewer",
        "run-1",
        tool::LIST_NEW,
        &home_call,
    )
    .await;
    assert_eq!(decided, Some(Decision::Allowed(vec!["gmail".to_string()])));
    // A mailbox that isn't connected reaches nothing
    let elsewhere = json!({"mailbox": "someone@else.com"});
    let decided = access::decide(
        &state,
        None,
        "pr-reviewer",
        "run-1",
        tool::LIST_NEW,
        &elsewhere,
    )
    .await;
    assert!(matches!(decided, Some(Decision::Refused(_))), "{decided:?}");
    let need = tool::access_need(&state, tool::LIST_NEW, &work);
    assert_eq!(need.mailbox.as_deref(), Some("dev@gmail.com"));
    assert_eq!(need.mailboxes, ["dev@gmail.com"]);
    assert!(!state
        .grants()
        .covers("pr-reviewer", "gmail", read, "dev@gmail.com"));
    // Nandi reads both, each result saying which
    let both = tool::call(
        &state,
        DEFAULT_AGENT,
        &gmail_ids,
        tool::LIST_NEW,
        &json!({}),
    )
    .await;
    assert!(both.result.contains("\"mailbox\""), "{}", both.result);
    let everything = tool::access_need(&state, tool::LIST_NEW, &json!({}));
    assert_eq!(everything.mailboxes.len(), 2);
    assert_eq!(everything.mailbox, None);

    // Signing the home mailbox out: the reviewer's grant named only it and
    // goes; Nandi's keeps the other
    disconnect(&state, home()).await.unwrap();
    assert!(!state.grants().allows("pr-reviewer", "gmail", read));
    assert_eq!(
        state.grants().mailboxes(DEFAULT_AGENT, "gmail", read),
        Some(vec!["dev@gmail.com".to_string()])
    );
    state.shutdown();
}

#[tokio::test]
async fn pausing_one_of_two_reads_only_the_other() {
    let mock = mock(gmail_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Gmail).await;
    gmail_as(&mock, "dev.home@gmail.com");
    connected(&state, MailProvider::Gmail).await;

    set_paused(&state, gmail(), true).await.unwrap();
    check_all(&state).await;
    assert_eq!(check_state(&state, &gmail()).await.last_checked_at, None);
    assert!(check_state(&state, &home()).await.last_checked_at.is_some());
    let paused: Value = load_setting(&state, gmail().cursor_key()).await;
    assert!(paused.is_null());

    // Resuming reads it again; an address that isn't connected is refused
    set_paused(&state, gmail(), false).await.unwrap();
    check_all(&state).await;
    assert!(check_state(&state, &gmail())
        .await
        .last_checked_at
        .is_some());
    let other = MailboxId::new(MailProvider::Gmail, "someone@gmail.com");
    assert!(set_paused(&state, other, true).await.is_err());
    state.shutdown();
}

#[tokio::test]
async fn a_failed_hand_over_keeps_the_cursor_so_no_mail_is_skipped() {
    struct Refuses;
    impl BatchSink for Refuses {
        fn submit<'a>(
            &'a self,
            _state: &'a AppState,
            _title: String,
            _text: String,
        ) -> BoxFuture<'a, Result<String, String>> {
            Box::pin(async { Err("the agent is missing".to_string()) })
        }
    }
    let mock = mock(gmail_api).await;
    let (state, _recorder) = state_with(&mock).await;
    state.email().set_sink(Arc::new(Refuses));
    connected(&state, MailProvider::Gmail).await;
    assert_eq!(check_all(&state).await.found, 0);
    let cursor: Value = load_setting(&state, gmail().cursor_key()).await;
    assert!(cursor.is_null());
    let gmail = &super::status(&state).await.mailboxes[0];
    assert_eq!(gmail.state, EmailAccountState::Error);
    assert!(gmail.error.as_deref().unwrap().contains("agent is missing"));
    state.shutdown();
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tools_are_only_for_an_agent_with_a_grant() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    assert!(tool::offered(&state, DEFAULT_AGENT, &[]).is_empty());
    connected(&state, MailProvider::Outlook).await;
    let names: Vec<String> = tool::offered(&state, DEFAULT_AGENT, &[])
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(
        names,
        [
            "mail_list_new",
            "mail_get_message",
            "mail_get_thread",
            "mail_search"
        ]
    );
    assert!(tool::offered(&state, "pr-reviewer", &[]).is_empty());
    let refused = tool::call(
        &state,
        "pr-reviewer",
        &allowed(&state, "pr-reviewer"),
        tool::GET_MESSAGE,
        &json!({"id": "outlook:m1"}),
    )
    .await;
    assert!(refused.is_error);
    state.shutdown();
}

#[tokio::test]
async fn reading_a_message_keeps_its_quote_and_marks_it_untrusted() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    let outcome = tool::call(
        &state,
        DEFAULT_AGENT,
        &allowed(&state, DEFAULT_AGENT),
        tool::GET_MESSAGE,
        &json!({"id": "outlook:m1"}),
    )
    .await;
    assert!(!outcome.is_error, "{}", outcome.result);
    assert!(outcome.result.contains("untrusted data"));
    assert!(outcome.result.contains("<email-data>"));
    assert!(outcome.result.contains("evidence list by Friday"));
    // The model tells the new part from the quoted reply itself
    assert!(outcome.result.contains(r"Lena wrote:\n> old text"));
    // Wrong prefixes and missing mailboxes are refused calmly
    let bad = tool::call(
        &state,
        DEFAULT_AGENT,
        &allowed(&state, DEFAULT_AGENT),
        tool::GET_MESSAGE,
        &json!({"id": "m1"}),
    )
    .await;
    assert!(bad.is_error);
    let gmail = tool::call(
        &state,
        DEFAULT_AGENT,
        &allowed(&state, DEFAULT_AGENT),
        tool::GET_MESSAGE,
        &json!({"id": "gmail:g1"}),
    )
    .await;
    assert!(gmail.is_error && gmail.result.contains("isn't connected"));
    let list = tool::call(
        &state,
        DEFAULT_AGENT,
        &allowed(&state, DEFAULT_AGENT),
        tool::LIST_NEW,
        &json!({}),
    )
    .await;
    assert!(!list.is_error, "{}", list.result);
    state.shutdown();
}

#[test]
fn email_runs_get_only_the_notes_and_mail_tools() {
    for allowed in [
        "mail_get_message",
        "filesystem__write_file",
        "filesystem__read_file",
    ] {
        assert!(tool::allowed_in_email_run(allowed));
    }
    for refused in [
        "delegate_to_agent",
        "find_people",
        "web_search__search",
        "github__post_review",
        "run_command",
        "write_file",
    ] {
        assert!(!tool::allowed_in_email_run(refused), "{refused}");
    }
}

// ---------------------------------------------------------------------------
// Sign-in
// ---------------------------------------------------------------------------

#[test]
fn sign_in_problems_are_explained_calmly() {
    let setup = setup("http://x");
    let admin = classify_sign_in_error(
        "Sign-in was refused: access_denied: AADSTS65001: The user or administrator has not consented",
        &setup,
    );
    assert_eq!(admin.kind, SignInProblemKind::AdminApproval);
    assert!(admin
        .admin_consent_url
        .unwrap()
        .contains("client_id=ms-client"));
    assert_eq!(
        classify_sign_in_error("Sign-in was refused: access_denied", &setup).kind,
        SignInProblemKind::Declined
    );
    assert_eq!(
        classify_sign_in_error("Could not reach the sign-in server: timed out", &setup).kind,
        SignInProblemKind::Network
    );
    assert_eq!(
        classify_sign_in_error("The sign-in expired. Please start again.", &setup).kind,
        SignInProblemKind::Expired
    );
    assert_eq!(
        classify_sign_in_error(
            "invalid_client: AADSTS7000218: The request body must contain 'client_secret'",
            &setup
        )
        .kind,
        SignInProblemKind::AppRegistration
    );
    assert_eq!(
        classify_sign_in_error(
            "Signed in, but couldn't save your sign-in (Couldn't save to Windows Credential Manager: too long)",
            &setup
        )
        .kind,
        SignInProblemKind::Keychain
    );
}

#[tokio::test]
async fn a_build_without_client_ids_cant_connect() {
    let state = AppState::for_tests(vec![]);
    state.email().set_setup(ProviderSetup::default());
    let status = super::status(&state).await;
    assert!(status.providers.iter().all(|p| !p.available));
    let error = start_sign_in(&state, MailProvider::Outlook, Reads::MAIL)
        .await
        .unwrap_err();
    assert!(error.contains("isn't available in this build"));
    state.shutdown();
}

#[tokio::test]
async fn sign_in_opens_microsoft_with_read_only_scopes() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    let flow = start_sign_in(&state, MailProvider::Outlook, Reads::MAIL)
        .await
        .unwrap();
    assert!(flow.user_code.is_empty(), "PKCE");
    let url = reqwest::Url::parse(&flow.verification_uri).unwrap();
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(params["client_id"], "ms-client");
    assert!(params["scope"].contains("Mail.Read") && params["scope"].contains("offline_access"));
    assert!(params["redirect_uri"].starts_with("http://localhost:"));
    assert_eq!(
        poll_sign_in(&state, MailProvider::Outlook).await.phase,
        SignInPhase::Waiting
    );
    state.shutdown();
}

// ---------------------------------------------------------------------------
// The background read of older mail
// ---------------------------------------------------------------------------

/// Days ago of the older mail in the mock Inbox, newest first
const OLD_MAIL: [(&str, i64); 3] = [("o1", 20), ("o2", 40), ("o3", 400)];

fn after_marker<'a>(filter: &'a str, marker: &str) -> Option<&'a str> {
    let rest = &filter[filter.find(marker)? + marker.len()..];
    Some(rest.split(" and ").next().unwrap_or(rest))
}

/// `outlook_api`, plus older Inbox mail served one message per request
/// (newest older than the `lt` time, not older than the `ge` floor) and a
/// `$count` answer
fn outlook_with_old_mail(req: &Req) -> (u16, Value) {
    let folder = match req.path.as_str() {
        "/graph/me/mailFolders/inbox/messages" => "inbox",
        "/graph/me/mailFolders/sentitems/messages" => "sentitems",
        _ => return outlook_api(req),
    };
    let filter = req.query.get("$filter").cloned().unwrap_or_default();
    if req.query.contains_key("$count") {
        return (200, json!({"@odata.count": 130, "value": []}));
    }
    let Some(before) = after_marker(&filter, "receivedDateTime lt ") else {
        return outlook_api(req);
    };
    if folder == "sentitems" {
        return (200, json!({"value": []}));
    }
    let before = super::mail::unix_of(before);
    let floor = after_marker(&filter, "receivedDateTime ge ").map(super::mail::unix_of);
    let next = OLD_MAIL.iter().find_map(|(id, days)| {
        let message = graph_message(id, "Ana", &format!("Old item {id}"), "focused", days * 24);
        let at = super::mail::unix_of(message["receivedDateTime"].as_str().unwrap());
        (at < before && floor.is_none_or(|f| at >= f)).then_some(message)
    });
    (
        200,
        json!({ "value": next.into_iter().collect::<Vec<_>>() }),
    )
}

async fn backfill_of(state: &AppState) -> super::settings::BackfillState {
    backfill::load(state, &outlook()).await
}

#[tokio::test]
async fn older_mail_is_read_oldest_ward_one_batch_a_pass_and_resumes() {
    let mock = mock(outlook_with_old_mail).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    set_settings(
        &state,
        EmailSettingsInput {
            interval_minutes: 30,
            watch: Vec::new(),
            backfill: BackfillRange::Everything,
        },
    )
    .await
    .unwrap();
    // Not before the first read
    assert_eq!(backfill::pass(&state).await, None);
    check_all(&state).await;
    let cursor: Value = load_setting(&state, outlook().cursor_key()).await;

    for (index, (id, _)) in OLD_MAIL.iter().enumerate() {
        let run = backfill::pass(&state).await;
        assert!(run.is_some(), "pass {index}");
        let runs = recorder.runs();
        let text = &runs.last().unwrap().1;
        assert!(text.contains("older mail (backfill)"), "{text}");
        assert!(text.starts_with("Older mail: 1 email for"), "{text}");
        assert!(!text.contains("new email"), "older mail is never new");
        assert!(text.contains("never as a to-do"));
        assert!(text.contains(&format!("Old item {id}")));
        // Its own position, saved: a restart carries on from here
        let saved = backfill_of(&state).await;
        assert_eq!(saved.read, u32::try_from(index + 1).unwrap());
    }
    let status = super::status(&state).await;
    assert_eq!(
        status.mailboxes[0].backfill.phase,
        backfill::BackfillPhase::Reading
    );
    assert_eq!(status.mailboxes[0].backfill.read, 3);
    // Nothing older: done
    assert_eq!(backfill::pass(&state).await, None);
    assert!(backfill_of(&state).await.done);
    assert_eq!(
        super::status(&state).await.mailboxes[0].backfill.phase,
        backfill::BackfillPhase::Done
    );
    // The change cursor never moved; only GETs reached Graph
    let after: Value = load_setting(&state, outlook().cursor_key()).await;
    assert_eq!(cursor, after);
    assert!(mock
        .api_requests()
        .iter()
        .all(|(method, _)| method == "GET"));
    state.shutdown();
}

#[tokio::test]
async fn older_mail_starts_where_the_first_read_stopped() {
    let mock = mock(outlook_with_old_mail).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    check_all(&state).await;
    // Not 14 days back: the oldest mail the first read listed (hours ago)
    let before = backfill_of(&state).await.before.unwrap();
    assert!(before > unix_now() - 2 * 86_400, "{before}");
    state.shutdown();
}

/// `outlook_with_old_mail`, but older reads return only a message on the
/// `lt` boundary itself, as a provider that rounds would
fn outlook_boundary_only(req: &Req) -> (u16, Value) {
    let filter = req.query.get("$filter").cloned().unwrap_or_default();
    match after_marker(&filter, "receivedDateTime lt ") {
        Some(before) if req.path == "/graph/me/mailFolders/inbox/messages" => {
            let mut message = graph_message("edge", "Ana", "On the edge", "focused", 0);
            message["receivedDateTime"] = json!(before);
            (200, json!({ "value": [message] }))
        }
        _ => outlook_with_old_mail(req),
    }
}

#[tokio::test]
async fn older_mail_never_hands_over_the_boundary_email_again() {
    let mock = mock(outlook_boundary_only).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    check_all(&state).await;
    let runs = recorder.runs().len();
    let before = backfill_of(&state).await.before.unwrap();
    assert_eq!(backfill::pass(&state).await, None);
    assert_eq!(recorder.runs().len(), runs, "no run for the boundary email");
    assert_eq!(backfill_of(&state).await.before, Some(before - 1));
    state.shutdown();
}

#[tokio::test]
async fn an_expired_cursor_after_a_hand_over_sends_no_old_mail() {
    let mock = mock(outlook_api).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    assert_eq!(check_all(&state).await.found, 2, "Lena and the sent reply");
    let graph = format!("{}/graph", mock.base.lock().unwrap());
    save_setting(
        &state,
        outlook().cursor_key(),
        json!({
            "inbox": format!("{graph}/me/mailFolders/inbox/messages/delta?$deltatoken=expired"),
            "sentitems": format!("{graph}/me/mailFolders/sentitems/messages/delta?$deltatoken=sent-1"),
        }),
    )
    .await
    .unwrap();
    // Pretend the hand-over was long ago enough for Lena to be before it
    let mut check = check_state(&state, &outlook()).await;
    check.caught_up_to = Some(unix_now() + 2 * CATCH_UP_SLACK_SECS);
    save_check_state(&state, &outlook(), &check).await.unwrap();
    assert_eq!(check_all(&state).await.found, 0, "Lena was handed over");
    assert_eq!(recorder.runs().len(), 1);
    state.shutdown();
}

#[test]
fn hand_over_and_backfill_boundaries() {
    assert!(!handed_over_before(100, None));
    assert!(handed_over_before(100, Some(100 + CATCH_UP_SLACK_SECS + 1)));
    assert!(!handed_over_before(100, Some(100 + CATCH_UP_SLACK_SECS)));
    assert_eq!(backfill_start(50, None), 50);
    assert_eq!(
        backfill_start(50, Some(80)),
        81,
        "just past what was left out"
    );
    assert_eq!(backfill_start(50, Some(10)), 50);
}

#[tokio::test]
async fn older_mail_stops_at_the_chosen_range() {
    let mock = mock(outlook_with_old_mail).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    set_settings(
        &state,
        EmailSettingsInput {
            interval_minutes: 30,
            watch: Vec::new(),
            backfill: BackfillRange::Year,
        },
    )
    .await
    .unwrap();
    check_all(&state).await;
    while backfill::pass(&state).await.is_some() {}
    assert!(backfill_of(&state).await.done);
    let texts: String = recorder.runs().into_iter().map(|(_, t)| t).collect();
    assert!(texts.contains("Old item o2"));
    assert!(!texts.contains("Old item o3"), "older than a year");
    state.shutdown();
}

#[tokio::test]
async fn older_mail_waits_while_nandi_is_busy() {
    let mock = mock(outlook_with_old_mail).await;
    let (state, recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    check_all(&state).await;
    let runs_before = recorder.runs().len();
    state
        .db()
        .call(|conn| {
            let conversation = db::ensure_agent_conversation(conn, DEFAULT_AGENT)?;
            conn.execute(
                "INSERT INTO runs (id, agent_id, conversation_id, started_at, status, input, source_type, priority)
                 VALUES ('busy', ?1, ?2, 0, 'running', 'hi', 'user', 10)",
                rusqlite::params![DEFAULT_AGENT, conversation],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    mock.clear();
    assert_eq!(backfill::pass(&state).await, None);
    assert_eq!(recorder.runs().len(), runs_before);
    assert!(mock.requests().is_empty(), "not even read");
    state.shutdown();
}

#[tokio::test]
async fn older_mail_pauses_stops_and_is_forgotten_on_disconnect() {
    let mock = mock(outlook_with_old_mail).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    check_all(&state).await;
    assert!(backfill::pass(&state).await.is_some());

    let status = backfill::set_backfill(&state, outlook(), BackfillAction::Pause)
        .await
        .unwrap();
    assert_eq!(
        status.mailboxes[0].backfill.phase,
        backfill::BackfillPhase::Paused
    );
    assert_eq!(backfill::pass(&state).await, None);
    backfill::set_backfill(&state, outlook(), BackfillAction::Resume)
        .await
        .unwrap();
    assert!(backfill::pass(&state).await.is_some());
    backfill::set_backfill(&state, outlook(), BackfillAction::Stop)
        .await
        .unwrap();
    assert_eq!(backfill::pass(&state).await, None);

    disconnect(&state, outlook()).await.unwrap();
    let position: Value = load_setting(&state, outlook().backfill_key()).await;
    assert!(position.is_null());
    state.shutdown();
}

#[tokio::test]
async fn older_mail_is_estimated_before_it_starts() {
    let mock = mock(outlook_with_old_mail).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    let estimates = backfill::estimate(&state, BackfillRange::Everything).await;
    assert_eq!(estimates.len(), 1);
    // 130 in Inbox and in Sent Items, 60 per run
    assert_eq!(estimates[0].emails, 260);
    assert_eq!(estimates[0].runs, 5);
    assert!(backfill::estimate(&state, BackfillRange::Off)
        .await
        .is_empty());
    state.shutdown();
}

#[test]
fn dmarc_passes_only_from_the_users_provider_for_the_from_domain() {
    use super::mail::{authentication_results, dmarc_verified};
    let gmail = "mx.google.com; dkim=pass header.i=@chase.com; spf=pass smtp.mailfrom=chase.com; \
                 dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=chase.com";
    let outlook = "spf=pass (sender IP is 1.2.3.4) smtp.mailfrom=alerts.chase.com; dkim=pass \
                   header.d=chase.com;dmarc=pass action=none header.from=alerts.chase.com;compauth=pass";
    assert!(dmarc_verified(&[gmail], "alerts@chase.com"));
    assert!(dmarc_verified(&[outlook], "no-reply@alerts.chase.com"));
    // Another domain, a failure, or a pass that came inside the mail (not first)
    assert!(!dmarc_verified(&[gmail], "alerts@evil.io"));
    assert!(!dmarc_verified(
        &["mx.google.com; dmarc=fail header.from=chase.com"],
        "alerts@chase.com"
    ));
    assert!(!dmarc_verified(
        &["mx.google.com; dmarc=none header.from=evil.io", gmail],
        "alerts@chase.com"
    ));
    assert!(!dmarc_verified(&[], "alerts@chase.com"));

    let headers = [
        ("Received", "from x"),
        ("authentication-results", gmail),
        ("Subject", "Statement"),
    ];
    assert_eq!(authentication_results(headers), vec![gmail]);
}

// ---------------------------------------------------------------------------
// Files and calendar with each account
// ---------------------------------------------------------------------------

fn everything() -> Reads {
    Reads {
        mail: true,
        files: true,
        calendar: true,
    }
}

/// Signs in to `provider` asking for `reads`, then connects
async fn connected_reading(state: &AppState, provider: MailProvider, reads: Reads) {
    let entry = provider.sign_in_entry("test");
    token(state.secrets(), &entry, 3600);
    connect(state, provider, &entry, reads).await.unwrap();
}

fn granted_to_nandi(state: &AppState, connector: &str) -> bool {
    state
        .grants()
        .allows(DEFAULT_AGENT, connector, GrantAccess::Read)
}

#[tokio::test]
async fn connecting_asks_only_for_the_ticked_reads_on_one_sign_in() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    let scope_of = |flow: DeviceFlowState| -> String {
        let url = reqwest::Url::parse(&flow.verification_uri).unwrap();
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        params["scope"].clone()
    };
    let mail = scope_of(
        start_sign_in(&state, MailProvider::Outlook, Reads::MAIL)
            .await
            .unwrap(),
    );
    assert!(
        !mail.contains("Files.") && !mail.contains("Calendars."),
        "{mail}"
    );
    let all = scope_of(
        start_sign_in(&state, MailProvider::Outlook, everything())
            .await
            .unwrap(),
    );
    assert!(
        all.contains("Mail.Read")
            && all.contains("Files.Read.All")
            && all.contains("Calendars.Read"),
        "{all}"
    );
    assert!(
        start_sign_in(&state, MailProvider::Outlook, Reads::default())
            .await
            .is_err()
    );

    connected_reading(&state, MailProvider::Outlook, everything()).await;
    let status = status(&state).await;
    assert_eq!(status.mailboxes[0].reads, everything());
    assert_eq!(status.mailboxes[0].reads_apart, Reads::default());
    assert!(status.mailboxes[0].calendar.is_some());
    for connector in ["outlook", "onedrive", "outlook-calendar"] {
        assert!(granted_to_nandi(&state, connector), "{connector}");
    }
    // The drive and calendar read with the mailbox's own sign-in
    let drive = &crate::services::workspace::sources(&state, AccountFamily::Files)[0];
    assert_eq!(drive.mailbox, outlook());
    assert_eq!(drive.account(), "dev@company.com");
    state.shutdown();
}

#[tokio::test]
async fn an_account_drops_its_files_and_keeps_its_mail() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected_reading(&state, MailProvider::Outlook, everything()).await;
    let files_cursor = outlook().read_key(AccountFamily::Files, "cursor");
    save_setting(&state, files_cursor.clone(), json!("t-1"))
        .await
        .unwrap();

    // Adding takes a sign-in; mail stops only by signing out
    assert!(set_reads(&state, outlook(), everything()).await.is_ok());
    let none = Reads::default();
    assert!(set_reads(&state, outlook(), none).await.is_err());
    let calendar_only = Reads::only(AccountFamily::Calendar);
    assert!(set_reads(&state, outlook(), calendar_only).await.is_err());

    let status = set_reads(
        &state,
        outlook(),
        Reads::MAIL.with(AccountFamily::Calendar, true),
    )
    .await
    .unwrap();
    assert!(!status.mailboxes[0].reads.files);
    assert!(status.mailboxes[0].reads.calendar);
    assert_eq!(
        load_setting::<Value>(&state, files_cursor).await,
        Value::Null
    );
    assert!(!granted_to_nandi(&state, "onedrive"), "no account reads it");
    assert!(granted_to_nandi(&state, "outlook"));
    assert!(granted_to_nandi(&state, "outlook-calendar"));
    assert!(set_reads(&state, gmail(), Reads::MAIL).await.is_err());

    // Signing out takes the calendar's grant and keys with it
    let calendar_cursor = outlook().read_key(AccountFamily::Calendar, "cursor");
    save_setting(&state, calendar_cursor.clone(), json!("d-1"))
        .await
        .unwrap();
    disconnect(&state, outlook()).await.unwrap();
    assert!(!granted_to_nandi(&state, "outlook-calendar"));
    assert_eq!(
        load_setting::<Value>(&state, calendar_cursor).await,
        Value::Null
    );
    state.shutdown();
}

#[tokio::test]
async fn a_new_sign_in_takes_over_a_calendar_connected_apart() {
    let mock = mock(outlook_api).await;
    let (state, _recorder) = state_with(&mock).await;
    connected(&state, MailProvider::Outlook).await;
    // As the upgrade from 0.2 leaves a calendar connected on its own
    let apart = outlook().read_token_entry(AccountFamily::Calendar);
    token(state.secrets(), &apart, 3600);
    update_config(&state, |config| {
        let email = config.email.as_mut().unwrap();
        let account = email.account_mut(&outlook()).unwrap();
        account.calendar = Some(ReadSignIn::Apart);
    })
    .unwrap();
    let cursor = outlook().read_key(AccountFamily::Calendar, "cursor");
    save_setting(&state, cursor.clone(), json!("d-1"))
        .await
        .unwrap();
    assert!(status(&state).await.mailboxes[0].reads_apart.calendar);

    // Reconnecting without the calendar ticked keeps it on its own sign-in
    connected_reading(&state, MailProvider::Outlook, Reads::MAIL).await;
    assert!(state.secrets().get(&apart).unwrap().is_some());
    assert!(status(&state).await.mailboxes[0].reads.calendar);

    // Ticking it: the account's sign-in takes over, the cursor stays
    connected_reading(
        &state,
        MailProvider::Outlook,
        Reads::MAIL.with(AccountFamily::Calendar, true),
    )
    .await;
    let account = email_settings(&state).account(&outlook()).cloned().unwrap();
    assert_eq!(account.calendar, Some(ReadSignIn::Account));
    assert_eq!(state.secrets().get(&apart).unwrap(), None);
    assert_eq!(load_setting::<Value>(&state, cursor).await, json!("d-1"));
    assert!(status(&state).await.mailboxes[0].reads_apart.is_empty());
    state.shutdown();
}
