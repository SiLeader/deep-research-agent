mod searxng;

use crate::DeepResearchTool;
use crate::fetched::FetchedDb;
use crate::tools::WebRequestLimits;
use crate::tools::web_search::searxng::SearxngClient;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use genai::chat::ToolName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct WebSearchTool {
    searxng_client: SearxngClient,
    db: Arc<FetchedDb>,
}

impl WebSearchTool {
    pub fn new_for_searxng(origin: &str, db: Arc<FetchedDb>) -> anyhow::Result<Self> {
        Self::new_for_searxng_with_limits(origin, WebRequestLimits::default(), db)
    }

    pub fn new_for_searxng_with_limits(
        origin: &str,
        limits: WebRequestLimits,
        db: Arc<FetchedDb>,
    ) -> anyhow::Result<Self> {
        let searxng_client = SearxngClient::new(origin, limits)?;
        Ok(Self { searxng_client, db })
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
    published_date: Option<DateTime<Utc>>,
}

#[async_trait]
impl DeepResearchTool for WebSearchTool {
    type Args = WebSearchArgs;
    type Output = WebSearchOutput;

    fn name(&self) -> ToolName {
        ToolName::Custom("search_sources".to_string())
    }

    fn description(&self) -> Option<&str> {
        Some(
            "Purpose: Discover web sources using the configured SearXNG search service.\n\
             Input: Provide query as a focused search query.\n\
             Output: Returns pages with URLs, titles, relevance scores, and publication dates; page bodies are not included. Available snippets are saved for search_fetched.\n\
             When to use: To find candidate sources for a research question. Use fetch on relevant URLs to inspect the source content and assess evidence.",
        )
    }

    async fn call(&self, args: Self::Args) -> anyhow::Result<Self::Output> {
        let res = self.searxng_client.search(&args.query).await?;
        for page in &res.results {
            if let Some(content) = &page.content {
                self.db.add_text(&page.url, content).await?;
            }
        }
        Ok(res.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn search_is_a_custom_tool_with_a_nonreserved_string_name() {
        let tool = WebSearchTool::new_for_searxng(
            "http://localhost/search",
            Arc::new(FetchedDb::new(1024, None, None).await.unwrap()),
        )
        .unwrap();
        assert!(matches!(tool.name(), ToolName::Custom(_)));
        assert_eq!(serde_json::to_value(tool.name()).unwrap(), "search_sources");
        assert_ne!(tool.name().to_string(), "web_search");
    }

    #[tokio::test]
    async fn providers_receive_a_custom_function_schema() {
        use crate::tools::http::tests::server;
        use genai::{
            Client, ModelIden, ServiceTarget,
            adapter::AdapterKind,
            chat::{ChatMessage, ChatRequest},
            resolver::{AuthData, Endpoint},
        };
        for adapter in [AdapterKind::OpenAI, AdapterKind::Anthropic] {
            let (addr, task) = server("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into()).await;
            let client = Client::builder()
                .with_service_target_resolver_fn(move |_: ServiceTarget| {
                    Ok(ServiceTarget {
                        model: ModelIden::new(adapter, "test-model"),
                        auth: AuthData::from_single("test"),
                        endpoint: Endpoint::from_owned(format!("http://{addr}/")),
                    })
                })
                .build();
            let mut tools = crate::DeepResearchTools::default();
            tools.add(
                WebSearchTool::new_for_searxng(
                    "http://localhost/search",
                    Arc::new(FetchedDb::new(1024, None, None).await.unwrap()),
                )
                .unwrap(),
            );
            tools.add(
                crate::tools::search_fetched::SearchFetchedTool::new(
                    Arc::new(FetchedDb::new(1024, None, None).await.unwrap()),
                    Default::default(),
                )
                .unwrap(),
            );
            let _ = client
                .exec_chat(
                    "test-model",
                    ChatRequest::new(vec![ChatMessage::user("test")])
                        .with_tools(tools.tools().unwrap()),
                    None,
                )
                .await;
            let request = task.await.unwrap();
            let start = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let body: serde_json::Value = serde_json::from_slice(&request[start..]).unwrap();
            let tool = body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| {
                    tool["function"]["name"] == "search_sources" || tool["name"] == "search_sources"
                })
                .unwrap();
            let (name, schema) = if adapter == AdapterKind::OpenAI {
                (&tool["function"]["name"], &tool["function"]["parameters"])
            } else {
                assert!(tool.get("type").is_none());
                (&tool["name"], &tool["input_schema"])
            };
            assert_eq!(name, "search_sources");
            assert_eq!(schema["properties"]["query"]["type"], "string");
            let fetched = body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| {
                    tool["function"]["name"] == "search_fetched" || tool["name"] == "search_fetched"
                })
                .unwrap();
            let schema = if adapter == AdapterKind::OpenAI {
                assert_eq!(fetched["function"]["strict"], true);
                &fetched["function"]["parameters"]
            } else {
                &fetched["input_schema"]
            };
            assert_eq!(schema["required"], serde_json::json!(["query", "top_k"]));
            assert_eq!(
                schema["properties"]["top_k"]["type"],
                serde_json::json!(["integer", "null"])
            );
            assert_eq!(schema["additionalProperties"], false);
        }
    }

    #[tokio::test]
    async fn dispatches_search_and_accepts_realistic_results() {
        use crate::tools::http::tests::server;
        let body = serde_json::json!({"results": [
            {"url": "https://example.com", "title": "Example", "score": 1.0, "content": "apple orchard"},
            {"url": "https://example.org", "title": "Dated", "score": 0.5, "publishedDate": "2026-10-06T00:00:00Z"}
        ]}).to_string();
        let (addr, task) = server(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())).await;
        let db = Arc::new(FetchedDb::new(1024, None, None).await.unwrap());
        let mut registry = crate::DeepResearchTools::default();
        registry.add(
            WebSearchTool::new_for_searxng(&format!("http://{addr}/search"), db.clone()).unwrap(),
        );
        let output = registry
            .call("search_sources", serde_json::json!({"query": "test query"}))
            .await
            .unwrap()
            .unwrap();
        assert!(output["pages"][0]["published_date"].is_null());
        assert_eq!(output["pages"][1]["published_date"], "2026-10-06T00:00:00Z");
        assert_eq!(
            db.search("apple", 5).await.unwrap()[0].url,
            "https://example.com"
        );
        let request = String::from_utf8(task.await.unwrap()).unwrap();
        assert!(request.starts_with("GET /search?format=json&q=test+query "));
    }
}
