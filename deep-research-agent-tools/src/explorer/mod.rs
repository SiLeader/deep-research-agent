use async_trait::async_trait;
use deep_research_react_agent::ReActAgent;
use deep_research_tools::DeepResearchTool;
use genai::chat::ToolName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct ExplorerTool {
    agent: ReActAgent,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub struct ExplorerArgs {
    query: String,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub struct ExplorerOutput {}

#[async_trait]
impl DeepResearchTool for ExplorerTool {
    type Args = ExplorerArgs;
    type Output = ExplorerOutput;

    fn name(&self) -> ToolName {
        "explorer".into()
    }

    fn description(&self) -> Option<&str> {
        Some("A tool that allows the agent to explore and gather information.")
    }

    async fn call(&self, args: Self::Args) -> anyhow::Result<Self::Output> {
        let res = self.agent.run(args.query).await?;
        // Implement the logic to use the ReActAgent for exploration here.
        // For now, we will just return an empty output.
        Ok(ExplorerOutput {})
    }
}
