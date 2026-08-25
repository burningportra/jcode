use crate::agent::Agent;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

const SYSTEM_PROMPT: &str = r#"You generate one concise next prompt suggestion for a coding assistant composer.
Return only plain text for the user to send next.
Return NO_SUGGESTION if there is no useful next prompt.
Do not use Markdown fences, bullets, quotes, explanations, or labels."#;

const MAX_CONTEXT_CHARS: usize = 6_000;
const NO_SUGGESTION_SENTINELS: &[&str] = &["NO_SUGGESTION", "NO SUGGESTION", "NONE", "N/A"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PromptSuggestionUpdate {
    pub session_id: String,
    pub generation: u64,
    pub suggestion: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct PromptSuggestionSnapshot {
    pub session_id: String,
    pub transcript: String,
    pub max_chars: usize,
}

type CompleteFn = Arc<dyn Fn(String, String) -> CompletionFuture + Send + Sync>;
type CompletionFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>;
type PublishFn = Arc<dyn Fn(PromptSuggestionUpdate) + Send + Sync>;

struct SessionGeneration {
    generation: u64,
    handle: Option<tokio::task::JoinHandle<()>>,
    latest: Option<PromptSuggestionUpdate>,
}

#[derive(Clone)]
pub(super) struct PromptSuggestionService {
    sessions: Arc<Mutex<HashMap<String, SessionGeneration>>>,
    complete: CompleteFn,
    publish: PublishFn,
}

impl PromptSuggestionService {
    pub(super) fn new(publish: impl Fn(PromptSuggestionUpdate) + Send + Sync + 'static) -> Self {
        Self::with_completion(publish, |system, prompt| {
            Box::pin(async move {
                let sidecar = jcode_base::sidecar::Sidecar::new();
                sidecar.complete(&system, &prompt).await
            })
        })
    }

    pub(super) fn with_completion(
        publish: impl Fn(PromptSuggestionUpdate) + Send + Sync + 'static,
        complete: impl Fn(String, String) -> CompletionFuture + Send + Sync + 'static,
    ) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            complete: Arc::new(complete),
            publish: Arc::new(publish),
        }
    }

    pub(super) async fn latest(&self, session_id: &str) -> Option<PromptSuggestionUpdate> {
        self.sessions
            .lock()
            .await
            .get(session_id)
            .and_then(|session| session.latest.clone())
    }

    pub(super) async fn cancel(&self, session_id: &str) {
        let update = {
            let mut sessions = self.sessions.lock().await;
            let session =
                sessions
                    .entry(session_id.to_string())
                    .or_insert_with(|| SessionGeneration {
                        generation: 0,
                        handle: None,
                        latest: None,
                    });
            session.generation = session.generation.saturating_add(1);
            if let Some(handle) = session.handle.take() {
                handle.abort();
            }
            let update = PromptSuggestionUpdate {
                session_id: session_id.to_string(),
                generation: session.generation,
                suggestion: None,
            };
            session.latest = Some(update.clone());
            update
        };
        (self.publish)(update);
    }

    pub(super) async fn remove_session(&self, session_id: &str) {
        if let Some(mut session) = self.sessions.lock().await.remove(session_id)
            && let Some(handle) = session.handle.take()
        {
            handle.abort();
        }
    }

    pub(super) async fn generate_after_success(&self, snapshot: PromptSuggestionSnapshot) -> u64 {
        let generation = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .entry(snapshot.session_id.clone())
                .or_insert_with(|| SessionGeneration {
                    generation: 0,
                    handle: None,
                    latest: None,
                });
            session.generation = session.generation.saturating_add(1);
            if let Some(handle) = session.handle.take() {
                handle.abort();
            }
            session.generation
        };

        let service = self.clone();
        let session_id = snapshot.session_id.clone();
        let task_session_id = session_id.clone();
        let handle = tokio::spawn(async move {
            let result = service.generate(snapshot).await;
            service
                .finish_generation(task_session_id, generation, result)
                .await;
        });

        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&session_id)
            && session.generation == generation
        {
            session.handle = Some(handle);
        } else {
            handle.abort();
        }
        generation
    }

    async fn generate(&self, snapshot: PromptSuggestionSnapshot) -> Result<Option<String>> {
        let prompt = build_prompt(&snapshot);
        let raw = (self.complete)(SYSTEM_PROMPT.to_string(), prompt).await?;
        normalize_suggestion(&raw, snapshot.max_chars)
    }

    async fn finish_generation(
        &self,
        session_id: String,
        generation: u64,
        result: Result<Option<String>>,
    ) {
        let suggestion = match result {
            Ok(suggestion) => suggestion,
            Err(error) => {
                crate::logging::event_debug(
                    "PROMPT_SUGGESTION_GENERATION_FAILED",
                    vec![
                        ("session_id", session_id.clone()),
                        ("generation", generation.to_string()),
                        (
                            "error_kind",
                            format!("{}", error).chars().take(120).collect(),
                        ),
                    ],
                );
                None
            }
        };

        let update = {
            let mut sessions = self.sessions.lock().await;
            let Some(session) = sessions.get_mut(&session_id) else {
                return;
            };
            if session.generation != generation {
                return;
            }
            session.handle = None;
            let update = PromptSuggestionUpdate {
                session_id,
                generation,
                suggestion,
            };
            session.latest = Some(update.clone());
            update
        };
        (self.publish)(update);
    }
}

pub(super) fn snapshot_from_agent(
    session_id: &str,
    agent: &Agent,
    client_supports_prompt_suggestions: bool,
) -> Option<PromptSuggestionSnapshot> {
    let workspace = agent.working_dir().unwrap_or_default();
    let config = jcode_base::config::config()
        .prompt_suggestions
        .for_workspace(workspace);
    let eligibility = jcode_base::prompt_suggestions::PromptSuggestionEligibility {
        config_enabled: config.enabled,
        interactive: client_supports_prompt_suggestions,
        successful_turn: true,
        headless: false,
        scripted: false,
        debug: false,
    };
    if !eligibility.is_eligible() {
        return None;
    }
    if agent.last_visible_conversation_role() != Some(jcode_message_types::Role::Assistant) {
        return None;
    }
    let transcript =
        truncate_utf8_tail(&agent.build_transcript_for_extraction(), MAX_CONTEXT_CHARS);
    if transcript.trim().is_empty() {
        return None;
    }
    Some(PromptSuggestionSnapshot {
        session_id: session_id.to_string(),
        transcript,
        max_chars: config.max_chars,
    })
}

fn build_prompt(snapshot: &PromptSuggestionSnapshot) -> String {
    format!(
        "Conversation transcript:\n{}\n\nSuggest the next short user prompt. Keep it under {} characters.",
        snapshot.transcript, snapshot.max_chars
    )
}

fn normalize_suggestion(raw: &str, max_chars: usize) -> Result<Option<String>> {
    let text = raw.trim().trim_matches(['\'', '"']);
    if text.is_empty() || is_no_suggestion(text) {
        return Ok(None);
    }
    if text.contains("```") || text.lines().count() > 3 {
        return Ok(None);
    }
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() || is_no_suggestion(&collapsed) {
        return Ok(None);
    }
    Ok(Some(
        truncate_utf8(&collapsed, max_chars).context("truncate suggestion")?,
    ))
}

fn is_no_suggestion(text: &str) -> bool {
    let normalized = text.trim().trim_matches('.').to_ascii_uppercase();
    NO_SUGGESTION_SENTINELS
        .iter()
        .any(|sentinel| normalized == *sentinel)
}

fn truncate_utf8(text: &str, max_chars: usize) -> Result<String> {
    if text.chars().count() <= max_chars {
        return Ok(text.to_string());
    }
    Ok(text.chars().take(max_chars).collect())
}

fn truncate_utf8_tail(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }
    text.chars().skip(len - max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;

    fn test_service(
        response: &'static str,
    ) -> (
        PromptSuggestionService,
        mpsc::UnboundedReceiver<PromptSuggestionUpdate>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let service = PromptSuggestionService::with_completion(
            move |update| {
                let _ = tx.send(update);
            },
            move |_system, _prompt| Box::pin(async move { Ok(response.to_string()) }),
        );
        (service, rx)
    }

    fn snapshot(session_id: &str) -> PromptSuggestionSnapshot {
        PromptSuggestionSnapshot {
            session_id: session_id.to_string(),
            transcript: "User: hi\nAssistant: hello".to_string(),
            max_chars: 20,
        }
    }

    #[tokio::test]
    async fn publishes_successful_suggestion() {
        let (service, mut rx) = test_service("Try adding tests");
        let generation = service.generate_after_success(snapshot("s1")).await;
        let update = rx.recv().await.unwrap();
        assert_eq!(generation, 1);
        assert_eq!(update.session_id, "s1");
        assert_eq!(update.generation, 1);
        assert_eq!(update.suggestion.as_deref(), Some("Try adding tests"));
        assert_eq!(service.latest("s1").await.unwrap(), update);
    }

    #[tokio::test]
    async fn sentinel_clears_suggestion() {
        let (service, mut rx) = test_service("NO_SUGGESTION");
        service.generate_after_success(snapshot("s1")).await;
        let update = rx.recv().await.unwrap();
        assert_eq!(update.suggestion, None);
    }

    #[tokio::test]
    async fn oversized_output_is_utf8_truncated() {
        let (service, mut rx) = test_service("😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀");
        service.generate_after_success(snapshot("s1")).await;
        let update = rx.recv().await.unwrap();
        assert_eq!(update.suggestion.unwrap().chars().count(), 20);
    }

    #[tokio::test]
    async fn cancellation_suppresses_stale_completion_and_publishes_clear() {
        let (publish_tx, mut rx) = mpsc::unbounded_channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let release_rx = Arc::new(Mutex::new(Some(release_rx)));
        let service = PromptSuggestionService::with_completion(
            move |update| {
                let _ = publish_tx.send(update);
            },
            move |_system, _prompt| {
                let release_rx = Arc::clone(&release_rx);
                Box::pin(async move {
                    let rx = release_rx.lock().await.take().unwrap();
                    let _ = rx.await;
                    Ok("late".to_string())
                })
            },
        );
        service.generate_after_success(snapshot("s1")).await;
        service.cancel("s1").await;
        let clear = rx.recv().await.unwrap();
        assert_eq!(clear.generation, 2);
        assert_eq!(clear.suggestion, None);
        let _ = release_tx.send(());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn newer_generation_supersedes_older_work() {
        let (publish_tx, mut rx) = mpsc::unbounded_channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = PromptSuggestionService::with_completion(
            move |update| {
                let _ = publish_tx.send(update);
            },
            move |_system, _prompt| {
                let calls = Arc::clone(&calls);
                Box::pin(async move {
                    let call = calls.fetch_add(1, Ordering::SeqCst);
                    if call == 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        Ok("old".to_string())
                    } else {
                        Ok("new".to_string())
                    }
                })
            },
        );
        service.generate_after_success(snapshot("s1")).await;
        tokio::task::yield_now().await;
        service.generate_after_success(snapshot("s1")).await;
        let update = rx.recv().await.unwrap();
        assert_eq!(update.generation, 2);
        assert_eq!(update.suggestion.as_deref(), Some("new"));
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert!(rx.try_recv().is_err());
    }
}
