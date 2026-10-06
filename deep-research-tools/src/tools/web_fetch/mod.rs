use crate::DeepResearchTool;
use async_trait::async_trait;
use genai::chat::ToolName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct WebFetchTool {
    client: reqwest::Client,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct WebFetchArgs {
    #[schemars(
        description = "The absolute HTTP or HTTPS URL to retrieve, such as a source URL returned by web search."
    )]
    pub url: String,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct WebFetchOutput {
    #[schemars(
        description = "The HTTP status code of the response. Check it before using the response body as source evidence."
    )]
    pub status_code: u16,
    #[schemars(
        description = "The HTTP response body decoded as text. It may contain raw HTML or an error page; it is not extracted article text or rendered browser content."
    )]
    pub content: String,
}

#[async_trait]
impl DeepResearchTool for WebFetchTool {
    type Args = WebFetchArgs;
    type Output = WebFetchOutput;

    fn name(&self) -> ToolName {
        ToolName::Custom("fetch".to_string())
    }

    fn description(&self) -> Option<&str> {
        Some(
            "Purpose: Retrieve a known URL with an HTTP GET request.\n\
             Input: Provide url as an absolute HTTP or HTTPS URL.\n\
             Output: Returns status_code and the response body as text, including raw HTML when supplied by the server. Non-success HTTP responses are also returned.\n\
             When to use: To inspect a source page or verify evidence after discovering its URL. Check status_code before interpreting content. JavaScript is not executed.",
        )
    }

    async fn call(&self, args: Self::Args) -> anyhow::Result<Self::Output> {
        let res = self.client.get(&args.url).send().await?;
        let status_code = res.status().as_u16();
        let content = res.text().await?;
        Ok(Self::Output {
            status_code,
            content,
        })
    }
}
