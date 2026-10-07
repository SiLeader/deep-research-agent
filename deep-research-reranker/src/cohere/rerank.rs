use crate::cohere::CohereRerankerClient;
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CohereRerankRequest {
    pub model: String,
    pub query: String,
    pub documents: Vec<String>,
    pub top_n: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CohereRerankResponse {
    pub results: Vec<RerankResult>,
    #[serde(default, rename = "id")]
    pub _id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RerankResult {
    pub index: usize,
    pub relevance_score: f32,
}

impl CohereRerankerClient {
    pub async fn rerank(
        &self,
        request: CohereRerankRequest,
    ) -> anyhow::Result<CohereRerankResponse> {
        let _permit = self.semaphore.acquire().await?;
        let mut builder = self.client.post(self.url.clone()).json(&request);
        if let Some(name) = &self.api_key_env {
            let key = std::env::var(name).with_context(|| {
                format!("Missing reranker API key environment variable: {name}")
            })?;
            anyhow::ensure!(
                !key.trim().is_empty(),
                "reranker API key environment variable is empty: {name}"
            );
            builder = builder.bearer_auth(key);
        }
        let response = builder.send().await?.error_for_status()?;

        let rerank_response: CohereRerankResponse = response.json().await?;
        Ok(rerank_response)
    }
}
