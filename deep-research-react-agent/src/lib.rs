mod component;
pub mod event;
mod run_future;
mod run_stream;
pub mod stream;
#[cfg(test)]
mod tests;

use deep_research_runner::OneshotRunner;
use deep_research_tools::{DeepResearchTool, DeepResearchTools};
use genai::chat::ToolName;
use std::collections::HashSet;

#[derive(Clone)]
pub struct ReActAgent {
    oneshot: OneshotRunner,
    tools: DeepResearchTools,
    system_prompt: String,
    stop_tool_names: HashSet<String>,
    max_llm_calls: usize,
}

impl ReActAgent {
    pub fn new(
        oneshot: OneshotRunner,
        tools: DeepResearchTools,
        system_prompt: String,
        stop_tool_names: HashSet<ToolName>,
    ) -> anyhow::Result<Self> {
        tools.tools()?;
        Ok(Self {
            oneshot,
            system_prompt,
            stop_tool_names: stop_tool_names.into_iter().map(|t| t.to_string()).collect(),
            tools,
            max_llm_calls: 30,
        })
    }

    pub fn model(&self) -> &str {
        self.oneshot.model()
    }

    pub fn with_max_llm_calls(mut self, max_llm_calls: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(max_llm_calls > 0, "max_llm_calls must be positive");
        self.max_llm_calls = max_llm_calls;
        Ok(self)
    }

    pub fn add_stop_tool(&mut self, tool: impl DeepResearchTool + 'static) {
        self.stop_tool_names.insert(tool.name().to_string());
        self.tools.add(tool);
    }
}
