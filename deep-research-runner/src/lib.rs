use deep_research_arbiter::AgentConcurrencyArbiter;
use genai::Client;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatResponse, Tool};
use std::sync::Arc;

#[derive(Clone)]
pub struct OneshotRunner {
    model: String,
    client: Client,
    arbiter: Arc<dyn AgentConcurrencyArbiter>,
    options: ChatOptions,
    request_timeout: std::time::Duration,
}

impl OneshotRunner {
    pub fn new(
        model: String,
        client: Client,
        arbiter: Arc<dyn AgentConcurrencyArbiter>,
        options: ChatOptions,
    ) -> Self {
        Self {
            model,
            client,
            arbiter,
            options,
            request_timeout: std::time::Duration::from_secs(120),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Deadline for one chat request, starting after the model permit is acquired.
    pub fn with_request_timeout(mut self, timeout_secs: u64) -> anyhow::Result<Self> {
        anyhow::ensure!(
            timeout_secs > 0,
            "LLM request_timeout_secs must be positive"
        );
        self.request_timeout = std::time::Duration::from_secs(timeout_secs);
        Ok(self)
    }

    pub async fn run(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<Tool>,
    ) -> anyhow::Result<ChatResponse> {
        let _guard = self.arbiter.acquire(&self.model).await?;

        let request = self.client.exec_chat(
            &self.model,
            ChatRequest::new(messages).with_tools(tools),
            Some(&self.options),
        );
        let res = tokio::time::timeout(self.request_timeout, request)
            .await
            .map_err(|_| anyhow::anyhow!("LLM request timed out for model: {}", self.model))??;

        Ok(res)
    }
}

#[cfg(test)]
mod tests;
