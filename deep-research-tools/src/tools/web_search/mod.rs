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
    #[schemars(
        description = "A focused web search query containing relevant keywords, names, or constraints for finding sources on the research topic."
    )]
    query: String,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct WebSearchOutput {
    #[schemars(
        description = "Matching pages returned by the search service, with URLs, titles, scores, and publication dates. Page bodies are not included."
    )]
    pages: Vec<WebSearchPage>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct WebSearchPage {
    #[schemars(
        description = "The source page URL, which can be passed to fetch to inspect its contents."
    )]
    url: String,
    #[schemars(description = "The page title as reported by the search service.")]
    title: String,
    #[schemars(
        description = "The relevance score supplied by the search service; it is not a measure of source reliability."
    )]
    score: f32,
    #[schemars(
        description = "The page publication timestamp reported by the search service, expressed in UTC."
    )]
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
        Some(
            "Purpose: Discover web sources using the configured SearXNG search service.\n\
             Input: Provide query as a focused search query.\n\
             Output: Returns pages with URLs, titles, relevance scores, and publication dates; page bodies are not included.\n\
             When to use: To find candidate sources for a research question. Use fetch on relevant URLs to inspect the source content and assess evidence.",
        )
    }

    async fn call(&self, args: Self::Args) -> anyhow::Result<Self::Output> {
        let res = self.searxng_client.search(&args.query).await?;
        Ok(res.into())
    }
}
