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
    #[schemars(
        description = "A focused research question or information-gathering objective. Include the scope and relevant constraints needed for the explorer to investigate it."
    )]
    query: String,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub struct ExplorerOutput {
    #[schemars(
        description = "An answer to the requested research question based on gathered evidence, including uncertainty or limitations where relevant."
    )]
    answer: String,
    #[schemars(description = "Sources consulted and the evidence supporting the answer.")]
    references: Vec<ExplorerReference>,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
struct ExplorerReference {
    #[schemars(
        description = "The source URL or another identifiable source locator for the supporting evidence."
    )]
    source: String,
    #[schemars(
        description = "A relevant excerpt or faithful summary of the source evidence supporting the answer."
    )]
    content: String,
}

impl ExplorerTool {
    pub fn new(runner: OneshotRunner, search_tools: DeepResearchTools) -> anyhow::Result<Self> {
        Self::new_with_system_prompt(
            runner,
            search_tools,
            "You are an explorer agent that gathers information and provides answers with references.".to_string(),
        )
    }

    pub fn new_with_system_prompt(
        runner: OneshotRunner,
        search_tools: DeepResearchTools,
        system_prompt: String,
    ) -> anyhow::Result<Self> {
        let mut agent = ReActAgent::new(runner, search_tools, system_prompt, HashSet::new())?;
        agent.add_stop_tool(MarkerTool::<ExplorerOutput>::new(
            "submit".into(),
            Some(
                "Purpose: Submit the explorer's answer and supporting references and end exploration.\n\
                 Input: Provide answer with evidence-based findings and references with source locators and relevant evidence from sources actually consulted.\n\
                 When to use: Once the investigation is complete. Include uncertainty and limitations where the available evidence is insufficient."
                    .to_string(),
            ),
            Some(true),
            None,
        ));
        Ok(Self { agent })
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
        Some(
            "Purpose: Delegate a focused investigation to an explorer agent that uses the configured search tools.\n\
             Input: Provide query as a research question or objective with relevant scope and constraints.\n\
             Output: Returns an answer and references containing source locators and supporting evidence.\n\
             When to use: When answering the assigned research goal requires gathering and assessing external information.",
        )
    }

    async fn call(&self, args: Self::Args) -> anyhow::Result<Self::Output> {
        let res = self.agent.run(args.query).await?;
        let output = serde_json::from_value(res.fn_arguments)?;
        Ok(output)
    }
}
