mod cohere;

pub struct DeepResearchReranker {
    cohere_client: cohere::CohereRerankerClient,
}

impl DeepResearchReranker {
    pub fn new(base_url: String) -> Self {
        let cohere_client = cohere::CohereRerankerClient::new(base_url);
        Self { cohere_client }
    }

    pub async fn rerank(
        &self,
        model: String,
        query: String,
        documents: Vec<String>,
        top_n: usize,
    ) -> anyhow::Result<Vec<(String, f32)>> {
        let results = self
            .rerank_indices(model, query, documents.clone(), top_n)
            .await?;
        Ok(results
            .into_iter()
            .map(|(index, score)| (documents[index].clone(), score))
            .collect())
    }

    /// Return original document indices, preserving identity for duplicate text.
    pub async fn rerank_indices(
        &self,
        model: String,
        query: String,
        documents: Vec<String>,
        top_n: usize,
    ) -> anyhow::Result<Vec<(usize, f32)>> {
        if documents.is_empty() || top_n == 0 {
            return Ok(Vec::new());
        }
        let count = documents.len();
        let request = cohere::rerank::CohereRerankRequest {
            model,
            query,
            documents,
            top_n,
        };
        let response = self.cohere_client.rerank(request).await?;
        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();
        for result in response.results {
            anyhow::ensure!(result.index < count, "reranker index out of range");
            anyhow::ensure!(seen.insert(result.index), "duplicate reranker index");
            anyhow::ensure!(
                result.relevance_score.is_finite(),
                "non-finite reranker score"
            );
            results.push((result.index, result.relevance_score));
        }
        results.sort_by(|a, b| b.1.total_cmp(&a.1));
        results.truncate(top_n);
        Ok(results)
    }
}
