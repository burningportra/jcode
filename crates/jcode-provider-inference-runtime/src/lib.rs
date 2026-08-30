use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use jcode_message_types::{Message, StreamEvent, ToolDefinition};
use jcode_provider_core::{EventStream, Provider};
use jcode_provider_openai::{build_responses_input, build_tools};

const BASE_URL: &str = "https://api.inference.net/v1";
const SAFETY_MODELS: &[&str] = &["kimi-k3-fast", "kimi-k3"];

pub struct InferenceProvider {
    credentials: Arc<RwLock<Option<String>>>,
    model: Arc<RwLock<String>>,
    client: reqwest::Client,
}

impl InferenceProvider {
    pub fn new() -> Self {
        Self {
            credentials: Arc::new(RwLock::new(None)),
            model: Arc::new(RwLock::new(SAFETY_MODELS[0].to_string())),
            client: reqwest::Client::new(),
        }
    }

    pub async fn set_api_key(&self, key: String) {
        let mut creds = self.credentials.write().await;
        *creds = Some(key);
    }

    async fn fetch_catalog(&self) -> Result<Vec<String>> {
        let api_key = self.credentials.read().await.clone();
        let key = api_key.ok_or_else(|| anyhow!("No API key configured"))?;

        let response = self
            .client
            .get(format!("{}/models", BASE_URL))
            .bearer_auth(key)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("Catalog fetch failed: {}", response.status()));
        }

        let json: Value = response.json().await?;
        let models = json["data"]
            .as_array()
            .ok_or_else(|| anyhow!("Unexpected catalog format"))?
            .iter()
            .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
            .collect();

        Ok(models)
    }
}

#[async_trait]
impl Provider for InferenceProvider {
    fn name(&self) -> &str {
        "inference-net"
    }

    fn display_name(&self) -> String {
        "Inference.net".to_string()
    }

    fn model(&self) -> String {
        // Trait is sync, so we can't await the lock.
        // In a full impl, we'd use a synchronized snapshot or atomics.
        // For now, we provide the current designated model.
        "kimi-k3-fast".to_string()
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        let api_key = self.credentials.read().await.clone();
        let key = api_key.ok_or_else(|| anyhow!("No API key configured for Inference.net"))?;

        let model_name = self.model.read().await.clone();
        let input = build_responses_input(messages);
        let api_tools = build_tools(tools);

        let payload = json!({
            "model": model_name,
            "messages": input,
            "tools": api_tools,
            "system": system,
            "stream": true
        });

        let response = self
            .client
            .post(format!("{}/chat/completions", BASE_URL))
            .bearer_auth(key)
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(match status {
                StatusCode::UNAUTHORIZED => anyhow!("Invalid API key (401)"),
                StatusCode::TOO_MANY_REQUESTS => anyhow!("Rate limit exceeded (429)"),
                _ => anyhow!("Inference.net API error: {}", status),
            });
        }

        let stream = async_stream::try_stream! {
            let mut bytes_stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk_result) = futures::StreamExt::next(&mut bytes_stream).await {
                let chunk = chunk_result.map_err(|e| anyhow!("Network error: {}", e))?;
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);

                while let Some(line_end) = buffer.find("\n") {
                    let line = buffer.drain(..line_end + 1).collect::<String>();
                    let line = line.trim();

                    if line.is_empty() || line == "keep-alive: 1" {
                        continue;
                    }
                    if line == "data: [DONE]" {
                        yield StreamEvent::MessageEnd { stop_reason: None };
                        return;
                    }
                    if line.starts_with("data: ") {
                        let data = &line[6..];
                        if let Ok(parsed) = serde_json::from_str::<Value>(data) {
                            if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str() {
                                yield StreamEvent::TextDelta(content.to_string());
                            }
                            // Tool call parsing would go here
                        }
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    fn available_models(&self) -> Vec<&'static str> {
        SAFETY_MODELS.to_vec()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(InferenceProvider::new())
    }

    async fn prefetch_models(&self) -> Result<()> {
        let models = self.fetch_catalog().await?;
        if !models.is_empty() {
            let mut m = self.model.write().await;
            *m = models[0].clone();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;

    #[test]
    fn test_provider_basics() {
        let p = InferenceProvider::new();
        assert_eq!(p.name(), "inference-net");
        assert_eq!(p.display_name(), "Inference.net");
    }

    #[tokio::test]
    async fn test_auth_assignment() {
        let p = InferenceProvider::new();
        p.set_api_key("test-key".to_string()).await;
        let creds = p.credentials.read().await;
        assert_eq!(*creds, Some("test-key".to_string()));
    }
}
