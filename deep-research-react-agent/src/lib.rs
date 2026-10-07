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
use std::sync::Arc;

/// Checks the arguments of a stop-tool call. A rejection is returned to the
/// model as the tool response so it can correct and resubmit.
pub type OutputValidator = Arc<dyn Fn(&serde_json::Value) -> anyhow::Result<()> + Send + Sync>;

#[derive(Clone)]
pub struct ReActAgent {
    oneshot: OneshotRunner,
    tools: DeepResearchTools,
    system_prompt: String,
    stop_tool_names: HashSet<String>,
    max_llm_calls: usize,
    max_tool_context_chars: Option<usize>,
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
            max_tool_context_chars: None,
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

    /// Bound the tool output kept in the conversation. Older tool responses are
    /// replaced with a placeholder first; the latest ones are truncated if needed.
    pub fn with_max_tool_context_chars(mut self, max_chars: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(max_chars > 0, "max_tool_context_chars must be positive");
        self.max_tool_context_chars = Some(max_chars);
        Ok(self)
    }

    pub fn add_stop_tool(&mut self, tool: impl DeepResearchTool + 'static) {
        self.stop_tool_names.insert(tool.name().to_string());
        self.tools.add(tool);
    }
}
