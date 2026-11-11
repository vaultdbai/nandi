//! The gateway client against a fake gateway (axum, in-process) that
//! checks every signature the way the real one does

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use nandi_seal::PublicKeys;
use serde_json::json;

use super::*;
use crate::llm::Message;

const SSE: &str = concat!(
    ": keep-alive\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello \"},\"finish_reason\":null}],\"usage\":null}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"there.\"},\"finish_reason\":\"stop\"}],\"usage\":null}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\n",
    "data: [DONE]\n\n",
);

/// One request the fake gateway saw
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

/// An answer the fake gateway gives: status, `Retry-After`, body
#[derive(Clone)]
struct Scripted {
    status: StatusCode,
    retry_after: Option<u64>,
    content_type: &'static str,
    body: String,
}

fn ok_json(body: serde_json::Value) -> Scripted {
    Scripted {
        status: StatusCode::OK,
        retry_after: None,
        content_type: "application/json",
        body: body.to_string(),
    }
}

fn sse() -> Scripted {
    Scripted {
        status: StatusCode::OK,
        retry_after: None,
        content_type: "text/event-stream",
        body: SSE.to_string(),
    }
}

fn error(status: u16, code: &str, retry_after: Option<u64>, extra: serde_json::Value) -> Scripted {
    let mut error = json!({ "code": code, "message": format!("The gateway says {code}.") });
    if let (Some(error), Some(extra)) = (error.as_object_mut(), extra.as_object()) {
        error.extend(extra.clone());
    }
    Scripted {
        status: StatusCode::from_u16(status).unwrap(),
        retry_after,
        content_type: "application/json",
        body: json!({ "error": error }).to_string(),
    }
}

#[derive(Default)]
struct Fake {
    script: VecDeque<Scripted>,
    seen: Vec<Seen>,
    /// Refuse requests whose time is further than this from `clock` (ms)
    clock: Option<i64>,
}

/// Verifies like the gateway: headers present, signature over the
/// signing string of what was received
fn verify(seen: &Seen) -> Result<i64, String> {
    let header = |name: &str| {
        seen.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or(format!("no {name}"))
    };
    let instance = header(INSTANCE_HEADER)?;
    let key = header(KEY_HEADER)?;
    let machine = header(MACHINE_HEADER)?;
    let time: i64 = header(TIME_HEADER)?.parse().map_err(|_| "time")?;
    let signature = header(SIGNATURE_HEADER)?;
    let message = signing_string(
        &seen.method,
        &seen.path,
        &instance,
        &machine,
        time,
        &seen.body,
    );
    let keys = PublicKeys {
        signing: key,
        sealing: String::new(),
    };
    nandi_seal::verify(&keys, message.as_bytes(), &signature).map_err(|e| e.to_string())?;
    Ok(time)
}

async fn fake_gateway(script: Vec<Scripted>) -> (String, Arc<Mutex<Fake>>) {
    let fake = Arc::new(Mutex::new(Fake {
        script: script.into(),
        ..Fake::default()
    }));
    let state = fake.clone();
    let app = axum::Router::new().fallback(
        move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
            let state = state.clone();
            async move {
                let seen = Seen {
                    method: method.to_string(),
                    path: uri.path().to_string(),
                    headers,
                    body: body.to_vec(),
                };
                let mut fake = state.lock().unwrap();
                let checked = verify(&seen);
                fake.seen.push(seen);
                let time = match checked {
                    Ok(time) => time,
                    Err(e) => {
                        return answer(error(401, "bad_signature", None, json!({ "detail": e })))
                    }
                };
                if let Some(clock) = fake.clock {
                    if (clock - time).abs() > 300_000 {
                        return answer(error(
                            401,
                            "stale_time",
                            None,
                            json!({ "serverTime": clock }),
                        ));
                    }
                }
                match fake.script.pop_front() {
                    Some(scripted) => answer(scripted),
                    None => answer(error(500, "internal", None, json!({}))),
                }
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, fake)
}

fn answer(scripted: Scripted) -> Response {
    let mut response = (
        scripted.status,
        [("content-type", scripted.content_type)],
        scripted.body,
    )
        .into_response();
    if let Some(seconds) = scripted.retry_after {
        response
            .headers_mut()
            .insert("retry-after", seconds.to_string().parse().unwrap());
    }
    response
}

/// A machine hash as the app sends it
const MACHINE: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn client(url: &str) -> GatewayClient {
    GatewayClient::new(
        url.to_string(),
        "alice-laptop-1a2b".into(),
        MACHINE.into(),
        DeviceKeys::generate().unwrap(),
    )
}

fn request() -> LlmRequest {
    LlmRequest {
        model: super::super::TRIAL_MODEL.into(),
        system: Some("Be brief".into()),
        messages: vec![Message::User {
            content: "hi".into(),
        }],
        tools: vec![],
        max_tokens: None,
    }
}

async fn chat(client: &GatewayClient) -> Result<LlmResponse, GatewayError> {
    client
        .chat(
            &reqwest::Client::new(),
            "deepseek-flash",
            &request(),
            256,
            None,
        )
        .await
}

#[test]
fn the_signing_string_matches_the_gateway() {
    // The gateway's own test vector (trial-gateway `auth.rs`, README)
    assert_eq!(
        signing_string("POST", "/v1/chat/completions", "i1", MACHINE, 42, b""),
        "nandi-trial-v2\nPOST\n/v1/chat/completions\ni1\n\
         0000000000000000000000000000000000000000000000000000000000000000\n42\n\
         e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        signing_string(
            "GET",
            "/v1/trial",
            "alice-laptop-1a2b",
            MACHINE,
            1_800_000_000_000,
            b"{}"
        ),
        format!(
            "nandi-trial-v2\nGET\n/v1/trial\nalice-laptop-1a2b\n{MACHINE}\n1800000000000\n\
             44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        )
    );
}

#[test]
fn headers_carry_a_signature_the_gateway_verifies() {
    let client = client("http://unused");
    let headers = client.headers("POST", CHAT_PATH, b"{\"a\":1}");
    let mut map = HeaderMap::new();
    for (name, value) in &headers {
        map.insert(*name, value.parse().unwrap());
    }
    let seen = Seen {
        method: "POST".into(),
        path: CHAT_PATH.into(),
        headers: map.clone(),
        body: b"{\"a\":1}".to_vec(),
    };
    assert!(verify(&seen).is_ok());
    assert_eq!(map[INSTANCE_HEADER], "alice-laptop-1a2b");
    assert_eq!(map[MACHINE_HEADER], MACHINE);
    // Another body (or path) doesn't verify
    let tampered = Seen {
        body: b"{\"a\":2}".to_vec(),
        ..seen
    };
    assert!(verify(&tampered).is_err());
}

#[test]
fn signed_times_only_go_up() {
    let client = client("http://unused");
    let times: Vec<i64> = (0..50)
        .map(|_| {
            client.headers("GET", TRIAL_PATH, b"")[3]
                .1
                .parse::<i64>()
                .unwrap()
        })
        .collect();
    assert!(times.windows(2).all(|pair| pair[1] > pair[0]));
}

#[test]
fn the_gateway_url_defaults_and_can_be_overridden() {
    assert_eq!(gateway_url(None), DEFAULT_GATEWAY_URL);
    assert_eq!(gateway_url(Some("  ")), DEFAULT_GATEWAY_URL);
    assert_eq!(
        gateway_url(Some("http://127.0.0.1:8790/")),
        "http://127.0.0.1:8790"
    );
}

#[tokio::test]
async fn a_chat_streams_through_the_gateway_with_no_key() {
    let (url, fake) = fake_gateway(vec![sse()]).await;
    let client = client(&url);
    let (tx, mut rx) = mpsc::channel(16);
    let response = client
        .chat(
            &reqwest::Client::new(),
            "deepseek-flash",
            &request(),
            256,
            Some(tx),
        )
        .await
        .unwrap();
    assert_eq!(response.content, "Hello there.");
    assert_eq!(response.usage.input_tokens, 12);
    let mut streamed = String::new();
    while let Ok(delta) = rx.try_recv() {
        streamed.push_str(&delta);
    }
    assert_eq!(streamed, "Hello there.");

    let fake = fake.lock().unwrap();
    let seen = &fake.seen[0];
    assert_eq!(seen.path, CHAT_PATH);
    // No bearer key: the gateway holds it
    assert!(seen.headers.get("authorization").is_none());
    // The DeepSeek adapter's body, on the trial's model
    let body: serde_json::Value = serde_json::from_slice(&seen.body).unwrap();
    assert_eq!(body["model"], "deepseek-flash");
    assert_eq!(body["stream"], true);
    assert_eq!(body["max_tokens"], 256);
    assert_eq!(body["messages"][0]["role"], "system");
}

#[tokio::test]
async fn a_short_retry_after_is_waited_out_once() {
    let (url, fake) =
        fake_gateway(vec![error(429, "rate_limited", Some(0), json!({})), sse()]).await;
    let response = chat(&client(&url)).await.unwrap();
    assert_eq!(response.content, "Hello there.");
    assert_eq!(fake.lock().unwrap().seen.len(), 2);
}

#[tokio::test]
async fn a_long_retry_after_is_reported_not_waited() {
    let (url, fake) = fake_gateway(vec![error(503, "upstream_busy", Some(30), json!({}))]).await;
    let error = chat(&client(&url)).await.unwrap_err();
    assert_eq!(error.code(), Some("upstream_busy"));
    assert!(error.user_message().contains("in 30 seconds"));
    assert_eq!(fake.lock().unwrap().seen.len(), 1);
}

#[tokio::test]
async fn a_second_busy_answer_is_not_retried_again() {
    let (url, fake) = fake_gateway(vec![
        error(429, "trial_busy", Some(0), json!({})),
        error(429, "trial_busy", Some(0), json!({})),
    ])
    .await;
    let error = chat(&client(&url)).await.unwrap_err();
    assert_eq!(error.code(), Some("trial_busy"));
    assert_eq!(fake.lock().unwrap().seen.len(), 2);
}

#[tokio::test]
async fn paused_and_used_up_are_never_retried() {
    let (url, fake) = fake_gateway(vec![
        error(503, "trial_paused", Some(5), json!({})),
        error(
            402,
            "trial_used_up",
            None,
            json!({ "used": 100000, "limit": 100000 }),
        ),
    ])
    .await;
    let client = client(&url);
    let paused = chat(&client).await.unwrap_err();
    assert_eq!(paused.code(), Some("trial_paused"));
    assert!(paused.user_message().contains("paused for today"));
    let used_up = chat(&client).await.unwrap_err();
    assert_eq!(used_up.code(), Some("trial_used_up"));
    assert_eq!(used_up.user_message(), super::super::TRIAL_USED_UP);
    assert_eq!(fake.lock().unwrap().seen.len(), 2);
}

#[tokio::test]
async fn a_stale_clock_is_corrected_by_the_servers_and_signed_again() {
    let (url, fake) = fake_gateway(vec![sse()]).await;
    // The server's clock is 20 minutes ahead of this computer's
    let server = local_now_ms() + 20 * 60_000;
    fake.lock().unwrap().clock = Some(server);
    let client = client(&url);
    let response = chat(&client).await.unwrap();
    assert_eq!(response.content, "Hello there.");
    {
        let fake = fake.lock().unwrap();
        assert_eq!(fake.seen.len(), 2, "refused once, then signed again");
        let second: i64 = fake.seen[1].headers[TIME_HEADER]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((second - server).abs() < 60_000);
    }
    // Later calls keep the correction
    fake.lock().unwrap().script.push_back(ok_json(json!({
        "account": null, "used": 0, "limit": 100000, "paused": false, "signIn": ["github"]
    })));
    client.account().await.unwrap();
    assert_eq!(fake.lock().unwrap().seen.len(), 3);
}

#[tokio::test]
async fn the_status_route_reads_the_allowance() {
    let (url, fake) = fake_gateway(vec![ok_json(json!({
        "account": "google:583231", "email": "ada@example.com", "used": 61234,
        "limit": 100000, "model": "deepseek-flash", "paused": false
    }))])
    .await;
    let account = client(&url).account().await.unwrap();
    assert_eq!(
        account,
        TrialAccount {
            account: Some("google:583231".into()),
            email: Some("ada@example.com".into()),
            used: 61_234,
            limit: 100_000,
            paused: false,
        }
    );
    let fake = fake.lock().unwrap();
    assert_eq!(fake.seen[0].method, "GET");
    assert_eq!(fake.seen[0].path, TRIAL_PATH);
    assert!(fake.seen[0].body.is_empty());
}

#[tokio::test]
async fn linking_sends_the_id_token() {
    let (url, fake) = fake_gateway(vec![
        ok_json(json!({
            "account": "google:583231", "email": "ada@example.com", "name": "Ada",
            "used": 61234, "limit": 100000
        })),
        error(401, "id_token_expired", None, json!({})),
    ])
    .await;
    let client = client(&url);
    assert_eq!(
        client.link("eyJ.id.token").await.unwrap(),
        TrialLink {
            account: "google:583231".into(),
            email: "ada@example.com".into(),
            name: "Ada".into(),
            used: 61_234,
            limit: 100_000,
        }
    );
    let expired = client.link("eyJ.old.token").await.unwrap_err();
    assert!(expired.user_message().contains("Settings › Email"));
    let fake = fake.lock().unwrap();
    assert_eq!(fake.seen[0].method, "POST");
    assert_eq!(fake.seen[0].path, LINK_PATH);
    let body: serde_json::Value = serde_json::from_slice(&fake.seen[0].body).unwrap();
    assert_eq!(body, json!({ "idToken": "eyJ.id.token" }));
    assert!(verify(&fake.seen[0]).is_ok());
}

#[test]
fn every_error_code_has_a_clear_message() {
    let cases = [
        ("trial_used_up", None, json!({}), "used up"),
        (
            "trial_not_enough",
            None,
            json!({ "left": 512 }),
            "Only 512 free tokens",
        ),
        ("trial_busy", Some(3), json!({}), "other calls"),
        ("trial_paused", Some(7_200), json!({}), "about 2 hours"),
        ("rate_limited", Some(40), json!({}), "in 40 seconds"),
        ("too_many_devices", None, json!({}), "tomorrow"),
        ("stale_time", None, json!({ "serverTime": 1 }), "clock"),
        ("key_mismatch", None, json!({}), "other keys"),
        ("upstream_busy", Some(5), json!({}), "busy"),
        ("upstream_error", None, json!({}), "refused the request"),
        ("machine_mismatch", None, json!({}), "another computer"),
        ("already_linked", None, json!({}), "another email"),
        ("id_token_invalid", None, json!({}), "Settings › Email"),
        ("email_unverified", None, json!({}), "verified"),
        ("link_unavailable", None, json!({}), "links again later"),
        ("bad_signature", None, json!({}), "bad_signature"),
    ];
    for (code, retry_after, extra, expected) in cases {
        let error = parse_error(
            400,
            retry_after,
            &json!({ "error": { "code": code, "message": "m" } })
                .to_string()
                .replace(
                    "\"message\":\"m\"",
                    &format!("\"message\":\"m\"{}", extra_fields(&extra)),
                ),
        );
        let message = error.user_message();
        assert!(message.contains(expected), "{code}: {message}");
    }
    assert!(GatewayError::Unreachable("x".into())
        .user_message()
        .contains("internet connection"));
    // Not the gateway's shape (a captive portal): still a sentence
    let portal = parse_error(502, None, "<html>Sign in to Wi-Fi</html>");
    assert_eq!(portal.code(), Some("unexpected"));
    assert!(portal.user_message().contains("HTTP 502"));
}

fn extra_fields(extra: &serde_json::Value) -> String {
    extra
        .as_object()
        .map(|fields| {
            fields
                .iter()
                .map(|(k, v)| format!(",\"{k}\":{v}"))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn model_calls_with_nothing_of_the_users_own_go_through_the_gateway() {
    use crate::services::llm_trial::{use_test_trial, Trial};
    use crate::services::secrets::Secrets;

    let (url, fake) = fake_gateway(vec![sse()]).await;
    use_test_trial(Some(Arc::new(Trial::new(url, "nandi-test-1".into()))));
    let secrets = Secrets::in_memory();
    // A Claude model with no key and no sign-in: the trial answers
    let response = crate::llm::complete(
        &secrets,
        &LlmRequest {
            model: "claude:claude-sonnet-5".into(),
            ..request()
        },
        None,
        crate::llm::CallSource::Proxy,
    )
    .await;
    use_test_trial(None);
    assert_eq!(response.unwrap().content, "Hello there.");
    // Signed with this device's keys, made and kept on first use
    let entry = crate::services::relay::device_keys::DEVICE_KEYS_ENTRY;
    assert!(secrets.get(entry).unwrap().is_some());
    let fake = fake.lock().unwrap();
    assert_eq!(fake.seen[0].headers[INSTANCE_HEADER], "nandi-test-1");
    let body: serde_json::Value = serde_json::from_slice(&fake.seen[0].body).unwrap();
    assert_eq!(body["model"], "deepseek-flash");
}
