use crate::DeepResearchTool;
use crate::fetched::FetchedDb;
use crate::tools::{WebRequestLimits, http::bounded_body};
use async_trait::async_trait;
use genai::chat::ToolName;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

mod public_network;
use public_network::{PublicDns, validate_url};

#[derive(Clone)]
pub struct WebFetchTool {
    client: reqwest::Client,
    limits: WebRequestLimits,
    db: Arc<FetchedDb>,
}

impl WebFetchTool {
    pub fn new(limits: WebRequestLimits, db: Arc<FetchedDb>) -> anyhow::Result<Self> {
        let client = limits
            .client_builder()?
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(PublicDns))
            .build()?;
        Ok(Self { client, limits, db })
    }

    async fn fetch(&self, mut url: reqwest::Url) -> anyhow::Result<WebFetchOutput> {
        for redirect_count in 0..=10 {
            validate_url(&url)?;
            let res = self.client.get(url.clone()).send().await?;
            if matches!(res.status().as_u16(), 301 | 302 | 303 | 307 | 308)
                && let Some(location) = res.headers().get(reqwest::header::LOCATION)
            {
                anyhow::ensure!(redirect_count < 10, "fetch exceeded redirect limit");
                url = url.join(location.to_str()?)?;
                continue;
            }
            let status_code = res.status().as_u16();
            let content_type = res
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<mime::Mime>().ok());
            let body = bounded_body(res, self.limits.max_body_bytes).await?;
            let mut stored = false;
            if (200..300).contains(&status_code)
                && let Some(kind) = &content_type
            {
                let is_html = matches!(kind.essence_str(), "text/html" | "application/xhtml+xml");
                let charset = kind.get_param("charset").map(|charset| charset.as_str());
                let content = detect_encoding(&body, charset, is_html)
                    .decode(&body)
                    .0
                    .into_owned();
                if is_html {
                    // Converting a large page is CPU-bound; keep it off the async workers.
                    let markdown =
                        tokio::task::spawn_blocking(move || htmd::convert(&content)).await??;
                    self.db.add_text(url.as_str(), &markdown).await?;
                    stored = !markdown.trim().is_empty();
                } else if matches!(kind.essence_str(), "text/plain" | "text/markdown") {
                    self.db.add_text(url.as_str(), &content).await?;
                    stored = !content.trim().is_empty();
                }
            }
            return Ok(WebFetchOutput {
                status_code,
                url: url.to_string(),
                content_type: content_type.map(|kind| kind.to_string()),
                stored,
            });
        }
        unreachable!("redirect limit is checked before continuing")
    }
}

/// Choose the decoder by byte-order mark, then the Content-Type charset, then
/// for HTML a `<meta>` charset declaration near the start of the document.
fn detect_encoding(
    body: &[u8],
    header_charset: Option<&str>,
    is_html: bool,
) -> &'static encoding_rs::Encoding {
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(body) {
        return encoding;
    }
    header_charset
        .and_then(|charset| encoding_rs::Encoding::for_label(charset.trim().as_bytes()))
        .or_else(|| is_html.then(|| meta_charset(body)).flatten())
        .unwrap_or(encoding_rs::UTF_8)
}

fn meta_charset(body: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let head = body[..body.len().min(4096)].to_ascii_lowercase();
    let mut rest = head.as_slice();
    while let Some(start) = find(rest, b"<meta") {
        let tag = &rest[start..];
        let tag = &tag[..find(tag, b">").unwrap_or(tag.len())];
        if let Some(position) = find(tag, b"charset") {
            let value = tag[position + b"charset".len()..]
                .trim_ascii_start()
                .strip_prefix(b"=")
                .map(|value| value.trim_ascii_start());
            if let Some(value) = value {
                let value = value
                    .strip_prefix(b"\"")
                    .or(value.strip_prefix(b"'"))
                    .unwrap_or(value);
                let end = value
                    .iter()
                    .position(|byte| {
                        matches!(byte, b'"' | b'\'' | b';' | b'/' | b'>')
                            || byte.is_ascii_whitespace()
                    })
                    .unwrap_or(value.len());
                if let Some(encoding) = encoding_rs::Encoding::for_label(&value[..end]) {
                    // A meta declaration cannot describe UTF-16 content (HTML spec).
                    return Some(
                        if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
                            encoding_rs::UTF_8
                        } else {
                            encoding
                        },
                    );
                }
            }
        }
        rest = &rest[start + b"<meta".len()..];
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
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
    /// Final source URL after redirects. Use it when citing retrieved content.
    pub url: String,
    /// Parsed Content-Type, if supplied by the server.
    pub content_type: Option<String>,
    /// True when nonempty supported content was saved. Read it using search_fetched.
    pub stored: bool,
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
             Output: Returns status_code, final url, content_type, and stored. Successful HTML, plain text, and Markdown are saved for search_fetched; error pages and unsupported or missing content types are not indexed.\n\
             When to use: To inspect a source page or verify evidence after discovering its URL. Use search_fetched to read saved evidence. Check status_code and stored. JavaScript is not executed.",
        )
    }

    async fn call(&self, args: Self::Args) -> anyhow::Result<Self::Output> {
        let url = reqwest::Url::parse(&args.url)?;
        tokio::time::timeout(
            Duration::from_secs(self.limits.request_timeout_secs),
            self.fetch(url),
        )
        .await
        .map_err(|_| anyhow::anyhow!("fetch request timed out"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::http::tests::server;

    // Test-only DNS pinning lets a public URL reach the local fixture. Production
    // constructors always install PublicDns and cannot override this restriction.
    async fn fixture_tool(addr: std::net::SocketAddr, limits: WebRequestLimits) -> WebFetchTool {
        let client = limits
            .client_builder()
            .unwrap()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve("public.example", addr)
            .build()
            .unwrap();
        WebFetchTool {
            client,
            limits,
            db: Arc::new(FetchedDb::new(1024, None, None).await.unwrap()),
        }
    }

    #[tokio::test]
    async fn indexes_only_successful_supported_nonempty_content() {
        for (status, kind, body, stored) in [
            (200, Some("text/html"), "<h1>apple</h1><p>orchard</p>", true),
            (
                200,
                Some("application/xhtml+xml"),
                "<p>apple orchard</p>",
                true,
            ),
            (200, Some("text/plain"), "apple orchard", true),
            (200, Some("text/markdown"), "# apple orchard", true),
            (404, Some("text/html"), "<p>apple orchard</p>", false),
            (200, Some("application/pdf"), "apple orchard", false),
            (200, None, "apple orchard", false),
            (200, Some("text/plain"), "  ", false),
            (200, Some("text/html"), "<p> </p>", false),
        ] {
            let header = kind
                .map(|kind| format!("Content-Type: {kind}\r\n"))
                .unwrap_or_default();
            let (addr, task) = server(format!("HTTP/1.1 {status} Test\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())).await;
            let tool = fixture_tool(addr, WebRequestLimits::default()).await;
            let mut registry = crate::DeepResearchTools::default();
            registry.add(tool.clone());
            registry.add(
                crate::tools::search_fetched::SearchFetchedTool::new(
                    tool.db.clone(),
                    Default::default(),
                )
                .unwrap(),
            );
            let result = registry.call("fetch", serde_json::json!({"url": format!("http://public.example:{}/page", addr.port())})).await.unwrap().unwrap();
            assert_eq!(result["stored"], stored);
            assert!(result.get("content").is_none());
            let found = registry
                .call(
                    "search_fetched",
                    serde_json::json!({"query": "apple", "top_k": null}),
                )
                .await
                .unwrap()
                .unwrap();
            let chunks = found["chunks"].as_array().unwrap();
            assert_eq!(!chunks.is_empty(), stored);
            if stored {
                assert_eq!(chunks[0]["url"], result["url"]);
                assert!(!chunks[0]["content"].as_str().unwrap().contains("<p>"));
            }
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn storage_failure_is_reported_without_indexing_evidence() {
        use genai::{
            Client, ModelIden, ServiceTarget,
            adapter::AdapterKind,
            resolver::{AuthData, Endpoint},
        };
        let response = serde_json::json!({"object": "list", "model": "test", "data": [{"object": "embedding", "index": 0, "embedding": [0.0, 0.0]}], "usage": {"prompt_tokens": 1, "total_tokens": 1}}).to_string();
        let (embed_addr, embed_task) = server(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len())).await;
        let client = Client::builder()
            .with_service_target_resolver_fn(move |_: ServiceTarget| {
                Ok(ServiceTarget {
                    model: ModelIden::new(AdapterKind::OpenAI, "test"),
                    auth: AuthData::from_single("test"),
                    endpoint: Endpoint::from_owned(format!("http://{embed_addr}/")),
                })
            })
            .build();
        let (addr, task) = server("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\napple".into()).await;
        let mut tool = fixture_tool(addr, WebRequestLimits::default()).await;
        tool.db = Arc::new(
            FetchedDb::new(
                1024,
                Some(crate::fetched::Embedder::new("test", client)),
                None,
            )
            .await
            .unwrap(),
        );
        let error = tool
            .call(WebFetchArgs {
                url: format!("http://public.example:{}/", addr.port()),
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("zero embedding vector"));
        assert!(tool.db.search("apple", 5).await.unwrap().is_empty());
        task.await.unwrap();
        embed_task.await.unwrap();
    }

    #[test]
    fn detects_encoding_from_bom_header_and_meta_declarations() {
        let sjis = encoding_rs::SHIFT_JIS.encode("日本語").0.into_owned();
        for (html, expected) in [
            ("<meta charset=\"Shift_JIS\">", encoding_rs::SHIFT_JIS),
            (
                "<META http-equiv='Content-Type' content='text/html; charset=euc-jp'>",
                encoding_rs::EUC_JP,
            ),
            ("<meta charset=utf-16>", encoding_rs::UTF_8),
            ("<p>no declaration</p>", encoding_rs::UTF_8),
        ] {
            assert_eq!(
                detect_encoding(html.as_bytes(), None, true),
                expected,
                "{html}"
            );
        }
        let page = [b"<meta charset=shift_jis><p>".as_slice(), &sjis].concat();
        assert_eq!(
            detect_encoding(&page, None, true).decode(&page).0,
            "<meta charset=shift_jis><p>日本語"
        );
        assert_eq!(detect_encoding(&page, None, false), encoding_rs::UTF_8);
        assert_eq!(
            detect_encoding(&page, Some("euc-jp"), true),
            encoding_rs::EUC_JP
        );
        assert_eq!(
            detect_encoding(
                b"\xEF\xBB\xBF<meta charset=shift_jis>",
                Some("euc-jp"),
                true
            ),
            encoding_rs::UTF_8
        );
    }

    #[tokio::test]
    async fn rejects_literal_and_dns_loopback_destinations() {
        let tool = WebFetchTool::new(
            WebRequestLimits::default(),
            Arc::new(FetchedDb::new(1024, None, None).await.unwrap()),
        )
        .unwrap();
        for url in ["http://127.0.0.1/", "http://[::1]/", "http://localhost/"] {
            let error = tool
                .call(WebFetchArgs { url: url.into() })
                .await
                .unwrap_err();
            assert!(format!("{error:#}").contains("public IP"), "{error:#}");
        }
    }

    #[tokio::test]
    async fn blocks_private_redirects_before_connecting() {
        let (addr, task) = server("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()).await;
        let tool = fixture_tool(addr, WebRequestLimits::default()).await;
        let error = tool
            .call(WebFetchArgs {
                url: format!("http://public.example:{}/", addr.port()),
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("public IP"));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn returns_status_and_limits_body_size() {
        for limit in [4, 5] {
            let (addr, task) = server(
                "HTTP/1.1 404 Not Found\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello"
                    .into(),
            )
            .await;
            let tool = fixture_tool(
                addr,
                WebRequestLimits {
                    max_body_bytes: limit,
                    ..Default::default()
                },
            )
            .await;
            let result = tool
                .call(WebFetchArgs {
                    url: format!("http://public.example:{}/", addr.port()),
                })
                .await;
            if limit == 4 {
                assert!(result.unwrap_err().to_string().contains("exceeds"));
            } else {
                let result = result.unwrap();
                assert_eq!(result.status_code, 404);
                assert!(!result.stored);
                assert!(tool.db.search("hello", 5).await.unwrap().is_empty());
            }
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn times_out_unresponsive_pages() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let tool = fixture_tool(
            addr,
            WebRequestLimits {
                request_timeout_secs: 1,
                ..Default::default()
            },
        )
        .await;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            tool.call(WebFetchArgs {
                url: format!("http://public.example:{}/", addr.port()),
            }),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        server.abort();
    }

    #[tokio::test]
    async fn follows_public_redirects_and_preserves_charset_decoding() {
        let (final_addr, final_task) = server("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-16le\r\nContent-Length: 4\r\nConnection: close\r\n\r\nh\0i\0".into()).await;
        let (addr, task) = server(format!("HTTP/1.1 302 Found\r\nLocation: http://public.example:{}/final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", final_addr.port())).await;
        let tool = fixture_tool(addr, WebRequestLimits::default()).await;
        let result = tool
            .call(WebFetchArgs {
                url: format!("http://public.example:{}/", addr.port()),
            })
            .await
            .unwrap();
        assert!(result.stored);
        let found = tool.db.search("hi", 5).await.unwrap();
        assert_eq!(found[0].content, "hi");
        assert_eq!(found[0].url, result.url);
        assert!(result.url.ends_with("/final"));
        assert_eq!(result.status_code, 200);
        task.await.unwrap();
        final_task.await.unwrap();
    }
}
