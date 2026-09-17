//! TypeSafe (System One) client: typed questions against a state, structured
//! answers back, no text to parse.
//!
//! One request evaluates `state` against a map of typed questions — noul
//! (yes/no probability), choice (option + full distribution), score (level
//! + distribution) — in parallel. This crate owns the wire format, retries,
//!   and a convenience helper for the bash gate's yes/no adjudication.
//!
//! # Fail-soft contract
//!
//! Callers in jcode treat any [`EvalError::Service`] as "no decision": the
//! agent keeps working and the gate falls back to its deterministic behavior.
//! Only [`EvalError::NoApiKey`] is an actionable configuration problem worth
//! surfacing.

use anyhow::Error;
use serde::Serialize;
use serde_json::{Map, Value};
use std::time::Duration;

/// Default model alias; TypeSafe documents `jev-latest` as the flagship.
pub const DEFAULT_MODEL: &str = "jev-latest";
/// Default API base URL (the `/v1/systemone` path is appended by the client).
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
/// Default per-request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum retries for retryable statuses (429/529). The docs mandate
/// backoff but not a count; three retries bounds a blocked command's
/// worst-case added latency to the backoff sleeps (~3.5s by default).
pub const MAX_RETRIES: u32 = 3;

/// A System One request: one state, any number of parallel typed questions.
#[derive(Debug, Clone, Serialize)]
pub struct SystemOneRequest {
    pub state: Value,
    pub model: String,
    pub questions: Map<String, Value>,
}

impl SystemOneRequest {
    /// Convenience constructor for the single noul question used by command
    /// adjudication. The id is for code only and is never sent to the model.
    pub fn noul_only(state: Value, id: &str, instructions: impl Into<String>) -> Self {
        let mut questions = Map::new();
        questions.insert(
            id.to_string(),
            serde_json::json!({
                "type": "noul",
                "instructions": instructions.into(),
            }),
        );
        Self {
            state,
            model: DEFAULT_MODEL.to_string(),
            questions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct SystemOneResult {
    pub model: String,
    /// One answer per question id, as returned by the server.
    pub answers: Value,
    pub usage: Usage,
}

impl SystemOneResult {
    /// The `noul` yes-probability for one question id, if it is a noul answer.
    pub fn noul(&self, id: &str) -> Option<f64> {
        self.answers
            .get(id)
            .and_then(|a| a.get("noul"))
            .and_then(Value::as_f64)
    }

    /// The chosen option for one choice question id, if present.
    pub fn choice(&self, id: &str) -> Option<&str> {
        self.answers
            .get(id)
            .and_then(|a| a.get("choice"))
            .and_then(Value::as_str)
    }

    /// A formatted `id: ...` line per answer, for rendering.
    pub fn format_answers(&self) -> String {
        let entries = self
            .answers
            .as_object()
            .map(|map| map.iter())
            .into_iter()
            .flatten()
            .map(|(id, answer)| {
                let mut parts: Vec<String> = Vec::new();
                if let Some(n) = answer.get("noul").and_then(Value::as_f64) {
                    parts.push(format!("{n:.3}"));
                }
                if let Some(c) = answer.get("choice").and_then(Value::as_str) {
                    parts.push(format!("choice={c}"));
                }
                if let Some(s) = answer.get("score").and_then(Value::as_f64) {
                    parts.push(format!("score={s:.3}"));
                }
                if let Some(conf) = answer.get("confidence").and_then(Value::as_f64) {
                    parts.push(format!("(confidence {conf:.2})"));
                }
                format!("{id}: {}", parts.join(" "))
            })
            .collect::<Vec<_>>();
        if entries.is_empty() {
            "(no answers)".to_string()
        } else {
            entries.join("\n")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// No API key configured (env var or config file).
    NoApiKey,
    /// Nothing transient: the decision cannot be made. Callers fall through.
    Service(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::NoApiKey => write!(f, "no TypeSafe API key configured"),
            EvalError::Service(message) => write!(f, "TypeSafe service: {message}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Client options, resolved by callers from jcode config.
#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub timeout: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

pub struct Client {
    http: reqwest::Client,
    options: ClientOptions,
}

impl Client {
    pub fn new(options: ClientOptions) -> Self {
        let http = reqwest::Client::builder()
            .timeout(options.timeout)
            .build()
            // An HTTP client build failure (bad proxy config) must not kill a
            // gate decision; it surfaces as Service on the first call instead.
            .unwrap_or_default();
        Self { http, options }
    }

    /// Evaluate `request` (state + questions), retrying retryable failures
    /// with capped exponential backoff. Returns [`EvalError`] so callers can
    /// distinguish "no key" from "no decision" — both fail soft.
    pub async fn evaluate(&self, request: &SystemOneRequest) -> Result<SystemOneResult, EvalError> {
        let api_key = self.options.api_key.clone().ok_or(EvalError::NoApiKey)?;
        let url = format!(
            "{}/v1/systemone",
            self.options.base_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "state": request.state,
            "model": request.model.as_str(),
            "questions": request.questions,
        });

        let mut last_error: Option<String> = None;
        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_millis(500u64.saturating_mul(1 << (attempt - 1).min(4)));
                tokio::time::sleep(delay).await;
            }
            let attempt_result = self
                .http
                .post(&url)
                .bearer_auth(&api_key)
                .json(&body)
                .send()
                .await;
            let response = match attempt_result {
                Ok(response) => response,
                Err(err) => {
                    let transient = is_transient_transport_error(&err.to_string());
                    last_error = Some(err.to_string());
                    if !transient {
                        return Err(EvalError::Service(err.to_string()));
                    }
                    continue;
                }
            };

            let status = response.status();
            if status.is_success() {
                match response.json::<Value>().await {
                    Ok(value) => {
                        return match parse_result(value) {
                            Ok(result) => Ok(result),
                            Err(err) => Err(EvalError::Service(format!(
                                "decoding TypeSafe response: {err}"
                            ))),
                        };
                    }
                    Err(err) => {
                        return Err(EvalError::Service(format!(
                            "decoding TypeSafe response: {err}"
                        )));
                    }
                }
            }
            if status.as_u16() == 429 || status.as_u16() == 529 {
                last_error = Some(format!("HTTP {status}"));
                continue;
            }
            let text = response.text().await.unwrap_or_default();
            return Err(EvalError::Service(format!(
                "HTTP {}: {}",
                status,
                truncate(&text, 300)
            )));
        }
        Err(EvalError::Service(format!(
            "retryable failure after {MAX_RETRIES} retries: {}",
            last_error.unwrap_or_else(|| "exhausted retries".into())
        )))
    }

    /// Evaluate a single noul question and return the yes-probability.
    ///
    /// This is the convenience the bash gate uses: one typed question, one
    /// number back. Any error returns `None` so the gate falls through to its
    /// deterministic behavior.
    pub async fn evaluate_noul(&self, state: Value, id: &str, instructions: &str) -> Option<f64> {
        let request = SystemOneRequest::noul_only(state, id, instructions);
        self.evaluate(&request).await.ok().and_then(|r| r.noul(id))
    }
}

fn parse_result(value: Value) -> Result<SystemOneResult, Error> {
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let answers = value
        .get("answers")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let usage = Usage {
        input_tokens: value
            .get("usage")
            .and_then(|u| u.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: value
            .get("usage")
            .and_then(|u| u.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
    };
    Ok(SystemOneResult {
        model,
        answers,
        usage,
    })
}

/// Whether an error message describes a transient transport-level fault worth
/// retrying, mirroring the provider runtimes' shared classifier vocabulary so
/// TypeSafe and the LLM providers treat the same faults the same way.
fn is_transient_transport_error(error_str: &str) -> bool {
    let lower = error_str.to_ascii_lowercase();
    lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("connection aborted")
        || lower.contains("broken pipe")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("unexpected eof")
        || lower.contains("close_notify")
        || lower.contains("client error (connect)")
        || lower.contains("connection error")
        || lower.contains("name resolution")
        || lower.contains("dns")
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use tokio::net::TcpListener;

    /// A tiny HTTP server that replays a per-connection status/body sequence
    /// (last entry repeats). Exercises retry paths without a mock crate.
    struct MiniHttp {
        statuses: Vec<u16>,
        bodies: Vec<String>,
        num_requests: std::sync::atomic::AtomicUsize,
        last_body: std::sync::Mutex<Vec<u8>>,
    }

    impl MiniHttp {
        fn with_sequence(statuses: Vec<u16>, bodies: Vec<String>) -> Self {
            assert!(
                !statuses.is_empty() && statuses.len() == bodies.len(),
                "statuses and bodies must pair"
            );
            Self {
                statuses,
                bodies,
                num_requests: std::sync::atomic::AtomicUsize::new(0),
                last_body: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn ok_once(body: &str) -> Self {
            Self::with_sequence(vec![200], vec![body.to_string()])
        }

        /// Binds a loopback listener and serves on a background thread. The
        /// server thread lives until process exit, which is fine for unit
        /// tests. Returns the base URL and an assert handle.
        async fn bind(self) -> (String, std::sync::Arc<Self>) {
            let arc = std::sync::Arc::new(self);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = arc.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    loop {
                        let (mut stream, _peer) = match listener.accept().await {
                            Ok(pair) => pair,
                            Err(_) => break,
                        };
                        let arc = arc.clone();
                        tokio::spawn(async move {
                            use tokio::io::AsyncWriteExt;
                            // Read a full HTTP/1.1 request: headers up to the
                            // blank line, then exactly Content-Length body bytes.
                            // (Reading until EOF would block forever since the
                            // client keeps the connection open awaiting the
                            // response.)
                            let full = match read_full_request(&mut stream).await {
                                Some(bytes) => bytes,
                                None => return,
                            };
                            *arc.last_body.lock().unwrap() = full.clone();
                            let count = arc.num_requests.fetch_add(1, Ordering::SeqCst);
                            let idx = count.min(arc.statuses.len() - 1);
                            let response = format!(
                                "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                arc.statuses[idx],
                                arc.bodies[idx].len(),
                                arc.bodies[idx],
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.flush().await;
                        });
                    }
                });
            });
            (format!("http://{addr}"), handle)
        }
    }

    /// Read a complete HTTP/1.1 request: headers up to the blank line, then
    /// exactly Content-Length body bytes. Reads until EOF would block forever
    /// because the client keeps the connection open while awaiting the
    /// response.
    async fn read_full_request(stream: &mut tokio::net::TcpStream) -> Option<Vec<u8>> {
        use tokio::io::AsyncReadExt;

        // Read up to (and including) the blank line that terminates headers.
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                    if buf.len() > 1 << 20 {
                        break;
                    }
                }
                Err(_) => return None,
            }
        }
        let header_end = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(0)
            .saturating_add(4);
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (k, v) = line.split_once(":")?;
                if k.trim() == "content-length" {
                    v.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);
        let mut body = buf[header_end..].to_vec();
        while body.len() < content_length {
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => body.extend_from_slice(&chunk[..n]),
                Err(_) => return None,
            }
        }
        body.truncate(content_length);
        Some(body)
    }

    fn client_for(base: &str, api_key: Option<&str>) -> Client {
        Client::new(ClientOptions {
            api_key: api_key.map(str::to_string),
            base_url: base.to_string(),
            model: DEFAULT_MODEL.into(),
            timeout: Duration::from_secs(5),
        })
    }

    fn noul_request() -> SystemOneRequest {
        SystemOneRequest::noul_only(
            Value::String("Help! My payouts have been failing for 3 days.".into()),
            "is_urgent",
            "Does this convey urgency?",
        )
    }

    #[tokio::test]
    async fn success_parses_answers() {
        let (base, handle) = MiniHttp::ok_once(
            r#"{
                "model": "jev-latest",
                "answers": {
                    "is_urgent": {"type": "noul", "noul": 0.92},
                    "department": {
                        "type": "choice",
                        "choice": "technical",
                        "probabilities": {"billing": 0.15, "technical": 0.84, "sales": 0.01},
                        "confidence": 0.596
                    },
                    "frustration": {
                        "type": "score",
                        "score": 1.035,
                        "legend": {"0": "Calm", "1": "Frustrated", "2": "Angry"},
                        "probabilities": {"0": 0.05, "1": 0.3, "2": 0.65},
                        "confidence": 0.842
                    }
                },
                "usage": {"input_tokens": 312, "output_tokens": 48}
            }"#,
        )
        .bind()
        .await;

        let client = client_for(&base, Some("test-key"));
        let result = client
            .evaluate(&noul_request())
            .await
            .expect("request should succeed");

        assert_eq!(result.model, "jev-latest");
        assert_eq!(result.noul("is_urgent"), Some(0.92));
        assert_eq!(result.choice("department"), Some("technical"));
        assert_eq!(result.usage.input_tokens, 312);

        // The wire body must carry the resolved model and the question.
        let guard = handle.last_body.lock().unwrap();
        let sent = String::from_utf8_lossy(guard.as_slice());
        assert!(sent.contains(r#""model":"jev-latest""#));
        assert!(sent.contains(r#""type":"noul""#));
        assert!(sent.contains("Does this convey urgency?"));
    }

    #[tokio::test]
    async fn no_api_key_fails_fast_without_request() {
        // Point at a closed port: if the client ever made a request, it would
        // time out; NoApiKey must short-circuit before any I/O.
        let client = client_for("http://127.0.0.1:1", None);
        let request = SystemOneRequest::noul_only(Value::String("x".into()), "q", "is it x?");
        let err = client
            .evaluate(&request)
            .await
            .expect_err("no key must fail");
        assert!(matches!(err, EvalError::NoApiKey));

        // evaluate_noul collapses any error to None so callers fall through.
        assert_eq!(
            client
                .evaluate_noul(Value::String("x".into()), "q", "is it x?")
                .await,
            None
        );
    }

    #[tokio::test]
    async fn retryable_status_retries_then_succeeds() {
        // First response is a 529 (overloaded, retryable per the API docs),
        // then the real answer.
        let (base, handle) = MiniHttp::with_sequence(
            vec![529, 200],
            vec![
                r#"{"detail":"overloaded"}"#.into(),
                r#"{"model":"jev-latest","answers":{"q":{"type":"noul","noul":0.5}},
                    "usage":{"input_tokens":1,"output_tokens":1}}"#
                    .into(),
            ],
        )
        .bind()
        .await;

        let client = client_for(&base, Some("k"));
        let request = SystemOneRequest::noul_only(Value::String("x".into()), "q", "is it?");
        let result = client
            .evaluate(&request)
            .await
            .expect("should succeed after retry");
        assert_eq!(result.noul("q"), Some(0.5));
        assert_eq!(
            handle.num_requests.load(Ordering::SeqCst),
            2,
            "529 must be retried"
        );
    }

    #[tokio::test]
    async fn retry_exhausted_reports_service_error() {
        // Every response is a 529; the client must give up after MAX_RETRIES
        // retries (initial attempt plus MAX_RETRIES retries total).
        let (base, handle) =
            MiniHttp::with_sequence(vec![529], vec![r#"{"detail":"overloaded"}"#.into()])
                .bind()
                .await;

        let client = client_for(&base, Some("k"));
        let request = SystemOneRequest::noul_only(Value::String("x".into()), "q", "is it?");
        let err = client
            .evaluate(&request)
            .await
            .expect_err("must exhaust retries");
        assert!(matches!(err, EvalError::Service(_)));
        assert!(err.to_string().contains("retries"));
        assert_eq!(
            handle.num_requests.load(Ordering::SeqCst),
            (MAX_RETRIES as usize) + 1,
            "initial attempt plus MAX_RETRIES retries"
        );
    }

    #[tokio::test]
    async fn non_retryable_status_is_immediate_service_error() {
        let (base, handle) =
            MiniHttp::with_sequence(vec![401], vec![r#"{"detail":"invalid key"}"#.into()])
                .bind()
                .await;
        let client = client_for(&base, Some("bad"));
        let request = SystemOneRequest::noul_only(Value::String("x".into()), "q", "is it?");
        let err = client.evaluate(&request).await.expect_err("401 must fail");
        assert!(matches!(err, EvalError::Service(_)));
        assert_eq!(
            handle.num_requests.load(Ordering::SeqCst),
            1,
            "non-retryable status must not be retried"
        );
    }

    #[test]
    fn transient_fault_classification() {
        assert!(is_transient_transport_error(
            "error sending request: connection refused"
        ));
        assert!(is_transient_transport_error("operation timed out"));
        assert!(!is_transient_transport_error("HTTP 401 unauthorized"));
        assert!(!is_transient_transport_error("bad gateway"));
    }

    #[test]
    fn format_answers_renders_all_kinds() {
        let result = SystemOneResult {
            model: "jev-latest".into(),
            answers: serde_json::json!({
                "n1": {"type":"noul","noul":0.9},
                "c1": {"type":"choice","choice":"a","probabilities":{"a":0.8,"b":0.2},"confidence":0.8},
                "s1": {"type":"score","score":1.5,"legend":{"0":"low","1":"mid","2":"high"},"probabilities":{"0":0.1,"1":0.4,"2":0.5},"confidence":0.7}
            }),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 2,
            },
        };
        let text = result.format_answers();
        assert!(text.contains("n1: 0.900"));
        assert!(text.contains("c1: choice=a"));
        assert!(text.contains("s1: score=1.500"));
        assert!(text.contains("confidence 0.80"));
    }

    #[test]
    fn request_serialization_shape() {
        let request = SystemOneRequest::noul_only(Value::String("x".into()), "q", "is it?");
        let value: Value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["state"], Value::String("x".into()));
        assert_eq!(value["model"], Value::String("jev-latest".into()));
        assert_eq!(
            value["questions"]["q"]["type"],
            Value::String("noul".into())
        );
        assert_eq!(
            value["questions"]["q"]["instructions"],
            Value::String("is it?".into())
        );
    }
}
