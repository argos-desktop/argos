use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, RequestBuilder, Response, header::RETRY_AFTER};
use serde_json::{Value, json};
use std::{fmt, time::Duration};

#[cfg(test)]
mod tests;

pub const CLAUDE_URL: &str = "https://api.anthropic.com";
pub const CLAUDE_MODEL: &str = "claude-haiku-5-5";
pub const CLAUDE_QUALITY_MODEL: &str = "claude-sonnet-5-5";
pub const CODEX_URL: &str = "https://api.openai.com";
pub const CODEX_MODEL: &str = "gpt-6.1-terra";

const ANTHROPIC_VERSION: &str = "2026-10-21";
const MAX_TOKENS: u32 = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

pub struct Turn<'a> {
    pub instructions: &'a str,
    pub history: &'a [(Role, String)],
    pub transcript: &'a str,
    pub jpeg: Option<&'a [u8]>,
}

// Carries no payload from the request or response, so it is always safe to log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Auth,
    Billing,
    RateLimited { retry_after: Option<Duration> },
    Transient,
    Invalid,
}

impl Error {
    pub fn permanent(&self) -> bool {
        matches!(self, Error::Auth | Error::Billing | Error::Invalid)
    }

    pub fn message(&self) -> &'static str {
        match self {
            Error::Auth => "The provider rejected this API key.",
            Error::Billing => "This account has no remaining credit or reached its spend limit.",
            Error::RateLimited { .. } => "The provider is rate limiting this account.",
            Error::Transient => "The provider is temporarily unavailable.",
            Error::Invalid => "The provider rejected the request. Check the model name.",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for Error {}

pub async fn suggest(
    http: &Client,
    base_url: &str,
    provider: Provider,
    key: &str,
    model: &str,
    turn: Turn<'_>,
    mut on_delta: impl FnMut(&str),
) -> Result<(), Error> {
    let (path, body) = match provider {
        Provider::Claude => ("/v1/messages", claude_body(model, &turn)),
        Provider::Codex => ("/v1/responses", codex_body(model, &turn)),
    };
    let request = authed(http.post(format!("{base_url}{path}")), provider, key).json(&body);
    let mut response = check(provider, request.send().await).await?;
    let mut sse = Sse::default();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transient)? {
        for data in sse.feed(&chunk) {
            let Ok(event) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            if let Some(done) = handle(provider, &event, &mut on_delta) {
                return done;
            }
        }
    }
    Err(Error::Transient)
}

pub async fn validate_key(
    http: &Client,
    base_url: &str,
    provider: Provider,
    key: &str,
) -> Result<(), Error> {
    let request = authed(http.get(format!("{base_url}/v1/models")), provider, key);
    check(provider, request.send().await).await.map(drop)
}

fn authed(request: RequestBuilder, provider: Provider, key: &str) -> RequestBuilder {
    match provider {
        Provider::Claude => request
            .header("x-api-key", key)
            .header("anthropic-version", ANTHROPIC_VERSION),
        Provider::Codex => request.bearer_auth(key),
    }
}

fn claude_body(model: &str, turn: &Turn) -> Value {
    fn push(messages: &mut Vec<Value>, role: Role, block: Value) {
        match messages.last_mut() {
            Some(last) if last["role"] == role.as_str() => {
                last["content"].as_array_mut().unwrap().push(block)
            }
            _ => messages.push(json!({"role": role.as_str(), "content": [block]})),
        }
    }
    let cache = json!({"type": "ephemeral"});
    let mut messages = Vec::new();
    for (role, text) in turn.history.iter().filter(|(_, text)| !text.is_empty()) {
        push(&mut messages, *role, json!({"type": "text", "text": text}));
    }
    if let Some(block) = messages
        .last_mut()
        .and_then(|last| last["content"].as_array_mut()?.last_mut())
    {
        block["cache_control"] = cache.clone();
    }
    if !turn.transcript.is_empty() {
        let block = json!({"type": "text", "text": turn.transcript});
        push(&mut messages, Role::User, block);
    }
    if let Some(jpeg) = turn.jpeg {
        let source =
            json!({"type": "base64", "media_type": "image/jpeg", "data": STANDARD.encode(jpeg)});
        push(
            &mut messages,
            Role::User,
            json!({"type": "image", "source": source}),
        );
    }
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "output_config": {"effort": "low"},
        "messages": messages,
    });
    if !turn.instructions.is_empty() {
        body["system"] =
            json!([{"type": "text", "text": turn.instructions, "cache_control": cache}]);
    }
    // Each model accepts only its own way of turning thinking off; others keep their default.
    match model {
        CLAUDE_MODEL => body["thinking"] = json!({"type": "disabled"}),
        CLAUDE_QUALITY_MODEL => body["thinking"] = json!({"type": "between_tools"}),
        _ => {}
    }
    body
}

fn codex_body(model: &str, turn: &Turn) -> Value {
    let mut input: Vec<Value> = turn
        .history
        .iter()
        .map(|(role, text)| json!({"role": role.as_str(), "content": text}))
        .collect();
    let mut content = vec![json!({"type": "input_text", "text": turn.transcript})];
    if let Some(jpeg) = turn.jpeg {
        let url = format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg));
        content.push(json!({"type": "input_image", "image_url": url, "detail": "high"}));
    }
    input.push(json!({"role": "user", "content": content}));
    json!({
        "model": model,
        "stream": true,
        "store": false,
        "instructions": turn.instructions,
        "reasoning": {"effort": "none"},
        "input": input,
    })
}

async fn check(provider: Provider, sent: reqwest::Result<Response>) -> Result<Response, Error> {
    let response = sent.map_err(|e| match e.is_builder() {
        true => Error::Invalid,
        false => Error::Transient,
    })?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    // shortcut: only the seconds form of Retry-After is read, add HTTP dates if a provider sends them.
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok()?.trim().parse().ok())
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok());
    let body = response.json::<Value>().await.unwrap_or_default();
    Err(classify(
        provider,
        status.as_u16(),
        retry_after,
        &body["error"],
    ))
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn classify(provider: Provider, status: u16, retry_after: Option<Duration>, err: &Value) -> Error {
    let (kind, code) = (text(&err["type"]), text(&err["code"]));
    let billing = match provider {
        Provider::Claude => {
            status == 402
                || kind == "billing_error"
                || text(&err["details"]["error_code"]) == "enforced_spend_limit_reached"
                // A spend limit the user set is a plain 400 that only its message identifies.
                || text(&err["message"]).starts_with("You have reached your specified")
        }
        Provider::Codex => {
            kind == "insufficient_quota"
                || matches!(
                    code,
                    "insufficient_quota"
                        | "billing_hard_limit_reached"
                        | "credit_balance_exhausted"
                )
                || code.ends_with("_spend_limit_exceeded")
                || code.ends_with("_usage_limit_exceeded")
        }
    };
    match status {
        _ if billing => Error::Billing,
        401 | 403 => Error::Auth,
        429 => Error::RateLimited { retry_after },
        408 | 409 | 500.. => Error::Transient,
        _ => Error::Invalid,
    }
}

fn stream_error(provider: Provider, err: &Value) -> Error {
    let status = match (text(&err["type"]), text(&err["code"])) {
        ("authentication_error" | "permission_error", _) | (_, "invalid_api_key") => 401,
        ("rate_limit_error", _) | (_, "rate_limit_exceeded") => 429,
        ("invalid_request_error" | "not_found_error" | "request_too_large", _) => 400,
        _ => 500,
    };
    classify(provider, status, None, err)
}

fn handle(
    provider: Provider,
    event: &Value,
    on_delta: &mut impl FnMut(&str),
) -> Option<Result<(), Error>> {
    match (provider, text(&event["type"])) {
        (Provider::Claude, "content_block_delta") if event["delta"]["type"] == "text_delta" => {
            on_delta(text(&event["delta"]["text"]))
        }
        (Provider::Claude, "message_stop") => return Some(Ok(())),
        (Provider::Claude, "error") => return Some(Err(stream_error(provider, &event["error"]))),
        (Provider::Codex, "response.output_text.delta") => on_delta(text(&event["delta"])),
        (Provider::Codex, "response.completed") => return Some(Ok(())),
        (Provider::Codex, "response.failed") => {
            return Some(Err(stream_error(provider, &event["response"]["error"])));
        }
        // A reply cut short still carries usable text.
        (Provider::Codex, "response.incomplete") => return Some(Ok(())),
        (Provider::Codex, "error") => return Some(Err(stream_error(provider, event))),
        _ => {}
    }
    None
}

#[derive(Default)]
struct Sse {
    buf: Vec<u8>,
    data: Option<String>,
}

impl Sse {
    // Returns the data payload of every event completed by this chunk.
    fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                events.extend(self.data.take());
            } else if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                match &mut self.data {
                    Some(data) => {
                        data.push('\n');
                        data.push_str(value);
                    }
                    None => self.data = Some(value.to_owned()),
                }
            }
        }
        events
    }
}
