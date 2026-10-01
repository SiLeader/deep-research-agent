mod searxng;

use crate::DeepResearchTool;
use crate::tools::web_search::searxng::SearxngClient;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use genai::chat::ToolName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct WebSearchTool {
    searxng_client: SearxngClient,
}

impl WebSearchTool {
    pub fn new_for_searxng(origin: &str) -> anyhow::Result<Self> {
        let searxng_client = SearxngClient::new(origin)?;
        Ok(Self { searxng_client })
    }
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct WebSearchArgs {
    #[schemars(description = "The search query to perform.")]
    query: String,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct WebSearchOutput {
    #[schemars(description = "The list of search results.")]
    pages: Vec<WebSearchPage>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct WebSearchPage {
    #[schemars(description = "The URL of the page.")]
    url: String,
    #[schemars(description = "The title of the page.")]
    title: String,
    #[schemars(description = "The relevance score of the page.")]
    score: f32,
    #[schemars(description = "The published date of the page.")]
    published_date: DateTime<Utc>,
}

#[async_trait]
impl DeepResearchTool for WebSearchTool {
    type Args = WebSearchArgs;
    type Output = WebSearchOutput;

    fn name(&self) -> ToolName {
        ToolName::WebSearch
    }

    fn description(&self) -> Option<&str> {
        Some("A tool for performing web searches.")
    }

    async fn call(&self, args: Self::Args) -> anyhow::Result<Self::Output> {
        let res = self.searxng_client.search(&args.query).await?;
        Ok(res.into())
    }
}
