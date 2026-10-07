pub(crate) mod rerank;

pub(crate) struct CohereRerankerClient {
    client: reqwest::Client,
    base_url: String,
}

impl CohereRerankerClient {
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::new();
        Self { client, base_url }
    }
}
