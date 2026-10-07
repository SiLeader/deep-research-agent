use async_trait::async_trait;
use deep_research_react_agent::ReActAgent;
use deep_research_runner::OneshotRunner;
use deep_research_tools::tools::marker::MarkerTool;
use deep_research_tools::{DeepResearchTool, DeepResearchTools};
use genai::chat::ToolName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::{future::Future, pin::Pin, sync::Arc};

type AgentFactory =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = anyhow::Result<ReActAgent>> + Send>> + Send + Sync>;

#[derive(Clone)]
pub struct ExplorerTool {
    agent: ReActAgent,
    agent_factory: Option<AgentFactory>,
    max_llm_calls: usize,
    max_tool_context_chars: Option<usize>,
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
    pub fn with_max_llm_calls(mut self, max_llm_calls: usize) -> anyhow::Result<Self> {
        self.agent = self.agent.with_max_llm_calls(max_llm_calls)?;
        self.max_llm_calls = max_llm_calls;
        Ok(self)
    }

    pub fn with_max_tool_context_chars(mut self, max_chars: usize) -> anyhow::Result<Self> {
        self.agent = self.agent.with_max_tool_context_chars(max_chars)?;
        self.max_tool_context_chars = Some(max_chars);
        Ok(self)
    }

    /// Create fresh tools for each invocation, including concurrent calls and clones.
    pub fn new_with_tools_factory<F, Fut>(
        runner: OneshotRunner,
        factory: F,
        system_prompt: String,
    ) -> anyhow::Result<Self>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<DeepResearchTools>> + Send + 'static,
    {
        let mut tool = Self::new_with_system_prompt(
            runner.clone(),
            DeepResearchTools::default(),
            system_prompt.clone(),
        )?;
        tool.agent_factory = Some(Arc::new(move || {
            let tools = factory();
            let runner = runner.clone();
            let prompt = system_prompt.clone();
            Box::pin(async move {
                Ok(Self::new_with_system_prompt(runner, tools.await?, prompt)?.agent)
            })
        }));
        Ok(tool)
    }

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
        Ok(Self {
            agent,
            agent_factory: None,
            max_llm_calls: 30,
            max_tool_context_chars: None,
        })
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
        let agent = match &self.agent_factory {
            Some(factory) => {
                let agent = factory().await?.with_max_llm_calls(self.max_llm_calls)?;
                match self.max_tool_context_chars {
                    Some(max_chars) => agent.with_max_tool_context_chars(max_chars)?,
                    None => agent,
                }
            }
            None => self.agent.clone(),
        };
        agent.get_output(args.query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deep_research_arbiter::{AgentConcurrencyArbiter, ArbiterTabletGuard};
    use deep_research_tools::{
        fetched::FetchedDb,
        tools::search_fetched::{FetchedConfig, SearchFetchedTool},
    };
    use genai::{Client, chat::ChatOptions};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;

    struct FailingArbiter;
    #[async_trait]
    impl AgentConcurrencyArbiter for FailingArbiter {
        async fn acquire(&self, _: &str) -> anyhow::Result<ArbiterTabletGuard> {
            anyhow::bail!("fixture stops before calling the LLM")
        }
    }

    #[tokio::test]
    async fn cloned_and_concurrent_invocations_create_isolated_databases() {
        let databases = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let tool = ExplorerTool::new_with_tools_factory(
            OneshotRunner::new(
                "test".into(),
                Client::default(),
                Arc::new(FailingArbiter),
                ChatOptions::default(),
            ),
            {
                let databases = databases.clone();
                let calls = calls.clone();
                move || {
                    let databases = databases.clone();
                    let id = calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        let db = Arc::new(FetchedDb::new(1024, None, None).await?);
                        db.add_text(&format!("https://source{id}.test"), "apple evidence")
                            .await?;
                        databases.lock().await.push(db.clone());
                        let mut tools = DeepResearchTools::default();
                        tools.add(SearchFetchedTool::new(db, FetchedConfig::default())?);
                        Ok(tools)
                    }
                }
            },
            "test".into(),
        )
        .unwrap()
        .with_max_llm_calls(1)
        .unwrap();
        let clone = tool.clone();
        let (a, b) = tokio::join!(
            tool.call(ExplorerArgs { query: "a".into() }),
            clone.call(ExplorerArgs { query: "b".into() })
        );
        assert!(a.is_err());
        assert!(b.is_err());
        assert!(tool.call(ExplorerArgs { query: "c".into() }).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let databases = databases.lock().await;
        let mut urls = HashSet::new();
        for db in databases.iter() {
            let results = db.search("apple", 20).await.unwrap();
            assert_eq!(results.len(), 1);
            urls.insert(results[0].url.clone());
        }
        assert_eq!(urls.len(), 3);
        assert_eq!(tool.max_llm_calls, 1);
    }
}
