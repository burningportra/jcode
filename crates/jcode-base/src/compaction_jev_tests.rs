use super::*;
use crate::config::{TypesafeCompactionConfig, TypesafeConfig};
use crate::provider::{EventStream, Provider};
use std::sync::Arc;

struct MockSummaryProvider;

#[async_trait::async_trait]
impl Provider for MockSummaryProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[crate::message::ToolDefinition],
        _system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }

    fn name(&self) -> &str {
        "mock-summary"
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(MockSummaryProvider)
    }

    async fn complete_simple(&self, prompt: &str, _system: &str) -> Result<String> {
        Ok(format!("fallback_summary({} chars)", prompt.len()))
    }
}

fn make_text_message(role: Role, text: &str) -> Message {
    Message {
        role,
        content: vec![ContentBlock::Text {
            text: text.to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }
}

#[tokio::test]
async fn test_jev_compaction_disabled_falls_back_to_plain() {
    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let messages = vec![
        make_text_message(Role::User, "test message 1"),
        make_text_message(Role::Assistant, "response 1"),
    ];

    let cfg = TypesafeConfig {
        enabled: false,
        api_key: None,
        api_key_env: "JCODE_TYPESAFE_API_KEY".to_string(),
        model: "jev-latest".to_string(),
        base_url: "https://api.typesafe.ai".to_string(),
        timeout_secs: 30,
        risk: Default::default(),
        auto: Default::default(),
        compaction: TypesafeCompactionConfig {
            enabled: false, // Disabled
            min_confidence: 0.6,
        },
    };

    let result = generate_compaction_artifact(provider, messages, None, Some(&cfg))
        .await
        .expect("should fall back successfully");

    assert!(
        result.summary_text.starts_with("fallback_summary"),
        "should use plain summarization when Jev is disabled"
    );
}

#[tokio::test]
async fn test_jev_compaction_no_api_key_falls_back() {
    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let messages = vec![
        make_text_message(Role::User, "test message 1"),
        make_text_message(Role::Assistant, "response 1"),
    ];

    let cfg = TypesafeConfig {
        enabled: true,
        api_key: None, // No API key
        api_key_env: "NONEXISTENT_VAR".to_string(),
        model: "jev-latest".to_string(),
        base_url: "https://api.typesafe.ai".to_string(),
        timeout_secs: 30,
        risk: Default::default(),
        auto: Default::default(),
        compaction: TypesafeCompactionConfig {
            enabled: true,
            min_confidence: 0.6,
        },
    };

    let result = generate_compaction_artifact(provider, messages, None, Some(&cfg))
        .await
        .expect("should fall back successfully");

    assert!(
        result.summary_text.starts_with("fallback_summary"),
        "should fall back to plain summarization when no API key is available"
    );
}

#[tokio::test]
async fn test_jev_compaction_none_config_falls_back() {
    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let messages = vec![
        make_text_message(Role::User, "test message 1"),
        make_text_message(Role::Assistant, "response 1"),
    ];

    // Pass None as TypesafeConfig
    let result = generate_compaction_artifact(provider, messages, None, None)
        .await
        .expect("should fall back successfully");

    assert!(
        result.summary_text.starts_with("fallback_summary"),
        "should fall back to plain summarization when no config is provided"
    );
}

#[test]
fn test_truncate_str_boundary_respects_char_boundaries() {
    let s = "Hello 世界";
    let truncated = truncate_str_boundary(s, 8);
    // "世界" starts at byte 6, so truncating at 8 should give us "Hello "
    assert_eq!(truncated, "Hello ");

    let short = "Hi";
    let not_truncated = truncate_str_boundary(short, 100);
    assert_eq!(not_truncated, "Hi");

    let exact = "12345";
    let exact_truncated = truncate_str_boundary(exact, 5);
    assert_eq!(exact_truncated, "12345");
}

#[tokio::test]
async fn test_generate_compaction_preserves_backward_compat() {
    // This test verifies that the new signature still works exactly like the old one
    // when typesafe_cfg is None (backward compatibility).
    let provider: Arc<dyn Provider> = Arc::new(MockSummaryProvider);
    let mut messages = Vec::new();
    for i in 0..10 {
        messages.push(make_text_message(
            Role::User,
            &format!("Message {}", i),
        ));
    }

    let result = generate_compaction_artifact(provider.clone(), messages.clone(), None, None)
        .await
        .expect("should succeed");

    assert!(result.summary_text.starts_with("fallback_summary"));
    assert_eq!(result.covers_up_to_turn, 10);
    assert!(result.openai_encrypted_content.is_none());
}
