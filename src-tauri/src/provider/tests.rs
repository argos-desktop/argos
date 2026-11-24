use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

const KEY: &str = "sk-test-not-a-real-key";
const BOTH: [Provider; 2] = [Provider::Claude, Provider::Codex];

// Serves one canned response and resolves to the raw request it received.
async fn mock(response: String) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let request = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut raw = Vec::new();
        let mut buf = [0; 4096];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            raw.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&raw);
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")?
                            .trim()
                            .parse()
                            .ok()
                    })
                    .unwrap_or(0);
                if body.len() >= length {
                    break;
                }
            }
            assert!(n > 0, "request ended early");
        }
        socket.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8(raw).unwrap()
    });
    (url, request)
}

fn http(status: u16, headers: &str, body: &str) -> String {
    format!("HTTP/1.1 {status} X\r\n{headers}connection: close\r\n\r\n{body}")
}

fn stream(events: &[&str]) -> String {
    let body: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
    http(200, "content-type: text/event-stream\r\n", &body)
}

fn client() -> Client {
    Client::builder().no_proxy().build().unwrap()
}

fn turn<'a>(history: &'a [(Role, String)], jpeg: Option<&'a [u8]>) -> Turn<'a> {
    Turn {
        instructions: "Be brief.",
        history,
        transcript: "What is your rate?",
        jpeg,
    }
}

async fn run(provider: Provider, response: String) -> (Result<(), Error>, String, String) {
    let (url, request) = mock(response).await;
    let mut out = String::new();
    let result = suggest(
        &client(),
        &url,
        provider,
        KEY,
        "some-model",
        turn(&[], None),
        |delta| out.push_str(delta),
    )
    .await;
    (result, out, request.await.unwrap())
}

async fn fails(provider: Provider, response: String) -> Error {
    run(provider, response).await.0.unwrap_err()
}

fn split(request: &str) -> (String, Value) {
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    (
        head.to_ascii_lowercase(),
        serde_json::from_str(body).unwrap(),
    )
}

#[tokio::test]
async fn claude_streams_deltas_and_sends_expected_request() {
    let history = [
        (Role::User, "Hello".to_owned()),
        (Role::Assistant, "Greet them.".to_owned()),
    ];
    let (url, request) = mock(stream(&[
        r#"{"type":"message_start","message":{}}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hidden"}}"#,
        r#"{"type":"ping"}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Say "}}"#,
        r#"{"type":"some_future_event"}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"hi."}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        r#"{"type":"message_stop"}"#,
    ]))
    .await;
    let mut deltas = Vec::new();
    suggest(
        &client(),
        &url,
        Provider::Claude,
        KEY,
        CLAUDE_MODEL,
        turn(&history, Some(b"jpg")),
        |delta| deltas.push(delta.to_owned()),
    )
    .await
    .unwrap();
    assert_eq!(deltas, ["Say ", "hi."]);

    let (head, body) = split(&request.await.unwrap());
    assert!(head.starts_with("post /v1/messages http/1.1"));
    assert!(head.contains(&format!("x-api-key: {KEY}")));
    assert!(head.contains("anthropic-version: 2026-10-21"));
    assert_eq!(body["model"], CLAUDE_MODEL);
    assert_eq!(body["stream"], true);
    assert_eq!(body["thinking"]["type"], "disabled");
    assert_eq!(body["output_config"]["effort"], "low");
    assert_eq!(body["system"][0]["text"], "Be brief.");
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    for field in ["temperature", "top_p", "top_k"] {
        assert!(body.get(field).is_none(), "{field}");
    }
    let messages = body["messages"].as_array().unwrap();
    let roles: Vec<_> = messages.iter().map(|m| text(&m["role"])).collect();
    assert_eq!(roles, ["user", "assistant", "user"]);
    assert_eq!(
        messages[1]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    let last = messages[2]["content"].as_array().unwrap();
    assert_eq!(last[0]["text"], "What is your rate?");
    assert_eq!(last[1]["type"], "image");
    assert_eq!(last[1]["source"]["media_type"], "image/jpeg");
    assert_eq!(last[1]["source"]["data"], "anBn");
    assert_eq!(last.len(), 2);
}

#[test]
fn claude_merges_consecutive_user_turns_and_picks_thinking_per_model() {
    let history = [(Role::User, "Hello".to_owned())];
    let body = claude_body(CLAUDE_QUALITY_MODEL, &turn(&history, None));
    assert_eq!(body["thinking"]["type"], "between_tools");
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 2);
    assert!(
        claude_body("other", &turn(&[], None))
            .get("thinking")
            .is_none()
    );
}

#[tokio::test]
async fn codex_streams_deltas_and_sends_expected_request() {
    let history = [
        (Role::User, "Hello".to_owned()),
        (Role::Assistant, "Greet them.".to_owned()),
    ];
    let (url, request) = mock(stream(&[
        r#"{"type":"response.created","response":{}}"#,
        r#"{"type":"response.output_text.delta","delta":"Say "}"#,
        r#"{"type":"response.some_future_event"}"#,
        r#"{"type":"response.output_text.delta","delta":"hi."}"#,
        r#"{"type":"response.completed","response":{}}"#,
    ]))
    .await;
    let mut deltas = Vec::new();
    suggest(
        &client(),
        &url,
        Provider::Codex,
        KEY,
        CODEX_MODEL,
        turn(&history, Some(b"jpg")),
        |delta| deltas.push(delta.to_owned()),
    )
    .await
    .unwrap();
    assert_eq!(deltas, ["Say ", "hi."]);

    let (head, body) = split(&request.await.unwrap());
    assert!(head.starts_with("post /v1/responses http/1.1"));
    assert!(head.contains(&format!("authorization: bearer {KEY}")));
    assert_eq!(body["model"], CODEX_MODEL);
    assert_eq!(body["stream"], true);
    assert_eq!(body["store"], false);
    assert_eq!(body["instructions"], "Be brief.");
    assert_eq!(body["reasoning"]["effort"], "none");
    for field in [
        "max_output_tokens",
        "temperature",
        "top_p",
        "truncation",
        "metadata",
        "user",
        "prompt_cache_retention",
    ] {
        assert!(body.get(field).is_none(), "{field}");
    }
    let input = body["input"].as_array().unwrap();
    assert_eq!(input.len(), 3);
    assert_eq!(input[1]["role"], "assistant");
    let last = input[2]["content"].as_array().unwrap();
    assert_eq!(last[0]["type"], "input_text");
    assert_eq!(last[0]["text"], "What is your rate?");
    assert_eq!(last[1]["type"], "input_image");
    assert_eq!(last[1]["image_url"], "data:image/jpeg;base64,anBn");
    assert_eq!(last[1]["detail"], "high");
    assert_eq!(last.len(), 2);
}

#[tokio::test]
async fn unauthorized_is_auth_and_never_exposes_the_key() {
    for provider in BOTH {
        let body =
            format!(r#"{{"error":{{"type":"authentication_error","message":"bad {KEY}"}}}}"#);
        let error = fails(provider, http(401, "", &body)).await;
        assert_eq!(error, Error::Auth);
        assert!(error.permanent());
        assert!(!format!("{error:?} {error}").contains(KEY));
    }
}

#[test]
fn no_error_text_contains_request_or_response_content() {
    for error in [
        Error::Auth,
        Error::Billing,
        Error::RateLimited {
            retry_after: Some(Duration::from_secs(1)),
        },
        Error::Transient,
        Error::Invalid,
    ] {
        let shown = format!("{error:?} {error}");
        assert!(!shown.contains(KEY) && !shown.contains("http"), "{shown}");
    }
}

#[tokio::test]
async fn rate_limit_carries_retry_after() {
    for provider in BOTH {
        let body = r#"{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded","message":"slow"}}"#;
        let error = fails(provider, http(429, "Retry-After: 7\r\n", body)).await;
        assert_eq!(
            error,
            Error::RateLimited {
                retry_after: Some(Duration::from_secs(7))
            }
        );
        assert!(!error.permanent());
        let error = fails(provider, http(429, "", body)).await;
        assert_eq!(error, Error::RateLimited { retry_after: None });
    }
}

#[tokio::test]
async fn spend_and_quota_limits_are_billing() {
    let cases = [
        (
            Provider::Claude,
            429,
            r#"{"type":"rate_limit_error","message":"cap","details":{"error_code":"enforced_spend_limit_reached"}}"#,
        ),
        (
            Provider::Claude,
            400,
            r#"{"type":"invalid_request_error","message":"You have reached your specified API usage limits."}"#,
        ),
        (
            Provider::Claude,
            402,
            r#"{"type":"billing_error","message":"pay"}"#,
        ),
        (
            Provider::Codex,
            429,
            r#"{"type":"insufficient_quota","code":"insufficient_quota","message":"quota"}"#,
        ),
        (
            Provider::Codex,
            429,
            r#"{"code":"credit_balance_exhausted","message":"credits"}"#,
        ),
        (
            Provider::Codex,
            429,
            r#"{"code":"project_spend_limit_exceeded","message":"limit"}"#,
        ),
    ];
    for (provider, status, error) in cases {
        let body = format!(r#"{{"error":{error}}}"#);
        let error = fails(provider, http(status, "", &body)).await;
        assert_eq!(error, Error::Billing, "{body}");
        assert!(error.permanent());
    }
}

#[tokio::test]
async fn server_errors_are_transient_and_bad_requests_are_invalid() {
    for (provider, status) in [
        (Provider::Claude, 500),
        (Provider::Claude, 504),
        (Provider::Claude, 529),
        (Provider::Codex, 500),
        (Provider::Codex, 503),
    ] {
        let error = fails(provider, http(status, "", "not json")).await;
        assert_eq!(error, Error::Transient, "{status}");
        assert!(!error.permanent());
    }
    for provider in BOTH {
        let body = r#"{"error":{"type":"invalid_request_error","message":"no"}}"#;
        assert_eq!(fails(provider, http(400, "", body)).await, Error::Invalid);
    }
}

#[tokio::test]
async fn mid_stream_errors_are_mapped() {
    let cases = [
        (
            Provider::Claude,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            Error::Transient,
        ),
        (
            Provider::Codex,
            r#"{"type":"error","code":"server_error","message":"oops"}"#,
            Error::Transient,
        ),
        (
            Provider::Codex,
            r#"{"type":"response.failed","response":{"error":{"code":"server_error","message":"oops"}}}"#,
            Error::Transient,
        ),
        (
            Provider::Codex,
            r#"{"type":"response.failed","response":{"error":{"code":"insufficient_quota","message":"quota"}}}"#,
            Error::Billing,
        ),
    ];
    for (provider, event, expected) in cases {
        let delta = match provider {
            Provider::Claude => {
                r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"partial"}}"#
            }
            Provider::Codex => r#"{"type":"response.output_text.delta","delta":"partial"}"#,
        };
        let (result, out, _) = run(provider, stream(&[delta, event])).await;
        assert_eq!(result, Err(expected), "{event}");
        assert_eq!(out, "partial");
    }
}

#[tokio::test]
async fn stream_closed_before_terminal_event_is_an_error() {
    let cases = [
        (
            Provider::Claude,
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"partial"}}"#,
        ),
        (
            Provider::Codex,
            r#"{"type":"response.output_text.delta","delta":"partial"}"#,
        ),
    ];
    for (provider, delta) in cases {
        let (result, out, _) = run(provider, stream(&[delta])).await;
        assert_eq!(result, Err(Error::Transient));
        assert_eq!(out, "partial");
    }
}

#[tokio::test]
async fn unreachable_provider_is_transient() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    for provider in BOTH {
        let error = validate_key(&client(), &url, provider, KEY).await;
        assert_eq!(error, Err(Error::Transient));
    }
}

#[tokio::test]
async fn validate_key_lists_models() {
    for provider in BOTH {
        let (url, request) = mock(http(200, "", r#"{"data":[]}"#)).await;
        assert_eq!(validate_key(&client(), &url, provider, KEY).await, Ok(()));
        let head = request.await.unwrap().to_ascii_lowercase();
        assert!(head.starts_with("get /v1/models http/1.1"));
        assert!(head.contains(match provider {
            Provider::Claude => "x-api-key: sk-test-not-a-real-key",
            Provider::Codex => "authorization: bearer sk-test-not-a-real-key",
        }));

        let (url, _) = mock(http(401, "", r#"{"error":{"message":"bad"}}"#)).await;
        assert_eq!(
            validate_key(&client(), &url, provider, KEY).await,
            Err(Error::Auth)
        );
    }
}

#[test]
fn sse_survives_any_chunk_boundary() {
    let wire = "event: a\r\ndata: {\"x\":\"é\"}\r\n\r\n: comment\n\ndata: one\ndata:two\n\ndata:\ndata: after-empty\n\ndata: unfinished";
    let expected = ["{\"x\":\"é\"}", "one\ntwo", "\nafter-empty"];
    let bytes = wire.as_bytes();
    for size in 1..=bytes.len() {
        let mut sse = Sse::default();
        let events: Vec<String> = bytes
            .chunks(size)
            .flat_map(|chunk| sse.feed(chunk))
            .collect();
        assert_eq!(events, expected, "chunk size {size}");
    }
    for cut in 0..bytes.len() {
        let mut sse = Sse::default();
        let mut events = sse.feed(&bytes[..cut]);
        events.extend(sse.feed(&bytes[cut..]));
        assert_eq!(events, expected, "cut at {cut}");
    }
}
