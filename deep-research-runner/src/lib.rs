use deep_research_arbiter::AgentConcurrencyArbiter;
use genai::Client;
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatResponse, Tool, ToolChoice};
use std::sync::Arc;

#[derive(Clone)]
pub struct OneshotRunner {
    model: String,
    client: Client,
    arbiter: Arc<dyn AgentConcurrencyArbiter>,
    options: ChatOptions,
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
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn run(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<Tool>,
    ) -> anyhow::Result<ChatResponse> {
        let _guard = self.arbiter.acquire(&self.model).await?;

        let res = self
            .client
            .exec_chat(
                &self.model,
                ChatRequest::new(messages).with_tools(tools),
                Some(&self.options),
            )
            .await?;

        Ok(res)
    }
}
