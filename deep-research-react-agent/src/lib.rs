use deep_research_arbiter::AgentConcurrencyArbiter;
use deep_research_tools::DeepResearchTools;
use std::sync::Arc;

pub struct ReActAgent {
    client: genai::Client,
    arbiter: Arc<dyn AgentConcurrencyArbiter>,
}

impl ReActAgent {
    pub fn new(client: genai::Client, arbiter: Arc<dyn AgentConcurrencyArbiter>) -> Self {
        Self { client, arbiter }
    }
}

impl ReActAgent {
    pub async fn run(&self, tools: &DeepResearchTools) {}
}
