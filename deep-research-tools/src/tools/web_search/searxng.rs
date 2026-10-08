use crate::tools::{WebRequestLimits, http::bounded_body};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use reqwest::Url;
use serde::{Deserialize, Deserializer};

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
    #[serde(
        default,
        rename = "publishedDate",
        alias = "published_date",
        deserialize_with = "lenient_date"
    )]
    pub published_date: Option<DateTime<Utc>>,
}

/// SearXNG serializes engine-provided datetimes with `isoformat()`, which omits
/// the offset for naive values. Treat those as UTC and drop unparsable dates
/// instead of failing the whole search.
fn lenient_date<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error> {
    let Some(serde_json::Value::String(text)) =
        Option::<serde_json::Value>::deserialize(deserializer)?
    else {
        return Ok(None);
    };
    let text = text.trim();
    Ok(DateTime::parse_from_rfc3339(text)
        .map(|date| date.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
                .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f"))
                .ok()
                .map(|date| date.and_utc())
        })
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|date| date.and_utc())
        }))
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
            snippets_saved: false,
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
            json!({"publishedDate": "2026-10-06T00:00:00"}),
            json!({"publishedDate": "2026-10-06T09:00:00+09:00"}),
            json!({"publishedDate": "2026-10-06"}),
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
            assert_eq!(
                output.pages[0].published_date.map(|date| date.to_rfc3339()),
                date.as_object()
                    .unwrap()
                    .values()
                    .any(|v| v.is_string())
                    .then(|| "2026-10-06T00:00:00+00:00".to_string())
            );
        }
        for invalid in [json!("not a date"), json!(""), json!(1728172800)] {
            let response: SearxngSearchResponse = serde_json::from_value(json!({"results": [
                {"url": "https://example.com", "title": "Example", "score": 1.0, "publishedDate": invalid}
            ]}))
            .unwrap();
            assert!(response.results[0].published_date.is_none());
        }
    }
}
