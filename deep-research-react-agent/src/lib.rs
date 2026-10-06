mod component;
pub mod event;
mod run_future;
mod run_stream;
pub mod stream;

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
}

impl ReActAgent {
    pub fn new(
        oneshot: OneshotRunner,
        tools: DeepResearchTools,
        system_prompt: String,
        stop_tool_names: HashSet<ToolName>,
    ) -> Self {
        Self {
            oneshot,
            tools,
            system_prompt,
            stop_tool_names: stop_tool_names.into_iter().map(|t| t.to_string()).collect(),
        }
    }

    pub fn model(&self) -> &str {
        self.oneshot.model()
    }

    pub fn add_stop_tool(&mut self, tool: impl DeepResearchTool + 'static) {
        self.stop_tool_names.insert(tool.name().to_string());
        self.tools.add(tool);
    }
}
