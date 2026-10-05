use async_trait::async_trait;
use deep_research_react_agent::ReActAgent;
use deep_research_runner::OneshotRunner;
use deep_research_tools::tools::marker::MarkerTool;
use deep_research_tools::{DeepResearchTool, DeepResearchTools};
use genai::chat::ToolName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone)]
pub struct ExplorerTool {
    agent: ReActAgent,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub struct ExplorerArgs {
    query: String,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub struct ExplorerOutput {
    answer: String,
    references: Vec<ExplorerReference>,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
struct ExplorerReference {
    source: String,
    content: String,
}

impl ExplorerTool {
    pub fn new(runner: OneshotRunner, search_tools: DeepResearchTools) -> Self {
        let mut agent = ReActAgent::new(
            runner,
            search_tools,
            "You are an explorer agent that gathers information and provides answers with references.".to_string(),
            HashSet::new(),
        );
        agent.add_stop_tool(MarkerTool::<ExplorerOutput>::new(
            "submit".into(),
            Some(
                "The final output of the explorer agent, containing the answer and references."
                    .to_string(),
            ),
            Some(true),
            None,
        ));
        Self { agent }
    }
}

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
        let output = serde_json::from_value(res.fn_arguments)?;
        Ok(output)
    }
}
