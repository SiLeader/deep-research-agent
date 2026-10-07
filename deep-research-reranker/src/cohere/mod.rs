pub(crate) mod rerank;

#[derive(Clone)]
pub(crate) struct CohereRerankerClient {
    client: reqwest::Client,
    url: reqwest::Url,
    api_key_env: Option<String>,
    semaphore: std::sync::Arc<tokio::sync::Semaphore>,
}

impl CohereRerankerClient {
    pub fn new(base_url: String) -> Self {
        Self::new_with_options(base_url, None, 60, 1).expect("Invalid reranker configuration")
    }

    pub fn new_with_options(
        base_url: String,
        api_key_env: Option<String>,
        timeout_secs: u64,
        max_concurrency: usize,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            timeout_secs > 0,
            "reranker request_timeout_secs must be positive"
        );
        anyhow::ensure!(
            (1..=tokio::sync::Semaphore::MAX_PERMITS).contains(&max_concurrency),
            "invalid reranker max_concurrency"
        );
        anyhow::ensure!(
            api_key_env
                .as_ref()
                .is_none_or(|name| !name.trim().is_empty()),
            "reranker api_key_env must not be empty"
        );
        let mut url = reqwest::Url::parse(&base_url)
            .map_err(|_| anyhow::anyhow!("invalid reranker endpoint URL"))?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
            "reranker endpoint must be HTTP(S)"
        );
        anyhow::ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "reranker endpoint must not contain credentials, query, or fragment"
        );
        let path = format!("{}/rerank", url.path().trim_end_matches('/'));
        url.set_path(&path);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()?;
        Ok(Self {
            client,
            url,
            api_key_env,
            semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(max_concurrency)),
        })
    }
}
