use chrono::{DateTime, Utc};
use reqwest::Url;
use serde::Deserialize;

#[derive(Clone)]
pub(super) struct SearxngClient {
    search_url: Url,
    client: reqwest::Client,
}

#[derive(Deserialize, Debug)]
pub(super) struct SearxngSearchResponse {
    pub results: Vec<SearxngSearchResult>,
}

#[derive(Deserialize, Debug)]
pub(super) struct SearxngSearchResult {
    pub url: String,
    pub title: String,
    pub score: f32,
    pub published_date: DateTime<Utc>,
}

impl SearxngClient {
    pub fn new(origin: &str) -> anyhow::Result<Self> {
        let mut search_url = Url::parse(origin)?;
        search_url.query_pairs_mut().append_pair("format", "json");

        let client = reqwest::Client::new();
        Ok(Self { search_url, client })
    }
}

impl SearxngClient {
    pub async fn search(&self, query: &str) -> anyhow::Result<SearxngSearchResponse> {
        let mut search_url = self.search_url.clone();
        search_url.query_pairs_mut().append_pair("q", query);

        let res = self.client.get(search_url).send().await?;
        if !res.status().is_success() {
            anyhow::bail!("Web search request failed with status: {}", res.status());
        }

        let json: SearxngSearchResponse = res.json().await?;
        Ok(json)
    }
}

impl From<SearxngSearchResult> for super::WebSearchPage {
    fn from(result: SearxngSearchResult) -> Self {
        Self {
            url: result.url,
            title: result.title,
            score: result.score,
            published_date: result.published_date,
        }
    }
}

impl From<SearxngSearchResponse> for super::WebSearchOutput {
    fn from(response: SearxngSearchResponse) -> Self {
        Self {
            pages: response.results.into_iter().map(Into::into).collect(),
        }
    }
}
