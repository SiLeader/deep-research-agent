use crate::tools::{WebRequestLimits, http::bounded_body};
use chrono::{DateTime, Utc};
use reqwest::Url;
use serde::Deserialize;

#[derive(Clone)]
pub(super) struct SearxngClient {
    search_url: Url,
    client: reqwest::Client,
    max_body_bytes: usize,
}

#[derive(Deserialize, Debug)]
pub(super) struct SearxngSearchResponse {
    pub results: Vec<SearxngSearchResult>,
}

#[derive(Deserialize, Debug)]
pub(super) struct SearxngSearchResult {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub content: Option<String>,
    pub score: f32,
    #[serde(default, rename = "publishedDate", alias = "published_date")]
    pub published_date: Option<DateTime<Utc>>,
}

impl SearxngClient {
    pub fn new(origin: &str, limits: WebRequestLimits) -> anyhow::Result<Self> {
        let mut search_url = Url::parse(origin)?;
        search_url.query_pairs_mut().append_pair("format", "json");

        let client = limits.client_builder()?.build()?;
        Ok(Self {
            search_url,
            client,
            max_body_bytes: limits.max_body_bytes,
        })
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

        let json: SearxngSearchResponse =
            serde_json::from_slice(&bounded_body(res, self.max_body_bytes).await?)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_optional_searxng_publication_dates() {
        for date in [
            json!({}),
            json!({"publishedDate": null}),
            json!({"publishedDate": "2026-10-06T00:00:00Z"}),
            json!({"published_date": "2026-10-06T00:00:00Z"}),
        ] {
            let mut result =
                json!({"url": "https://example.com", "title": "Example", "score": 1.0});
            result
                .as_object_mut()
                .unwrap()
                .extend(date.as_object().unwrap().clone());
            let response: SearxngSearchResponse =
                serde_json::from_value(json!({"results": [result]})).unwrap();
            let output: super::super::WebSearchOutput = response.into();
            assert_eq!(output.pages.len(), 1);
            assert_eq!(
                output.pages[0].published_date.is_some(),
                date.as_object().unwrap().values().any(|v| v.is_string())
            );
        }
    }
}
