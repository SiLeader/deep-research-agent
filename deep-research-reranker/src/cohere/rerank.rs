use crate::cohere::CohereRerankerClient;
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
    pub id: String,
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
        let url = format!("{}/rerank", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await?
            .error_for_status()?;

        let rerank_response: CohereRerankResponse = response.json().await?;
        Ok(rerank_response)
    }
}
