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
    pub url: String,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct WebFetchOutput {
    pub status_code: u16,
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
        Some("A tool for fetching web pages.")
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
