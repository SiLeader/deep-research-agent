use crate::DeepResearchTool;
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
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new(WebRequestLimits::default()).expect("Failed to build fetch HTTP client")
    }
}

impl WebFetchTool {
    pub fn new(limits: WebRequestLimits) -> anyhow::Result<Self> {
        let client = limits
            .client_builder()?
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(PublicDns))
            .build()?;
        Ok(Self { client, limits })
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
            let charset = content_type
                .as_ref()
                .and_then(|mime| mime.get_param("charset"))
                .map(|charset| charset.as_str())
                .unwrap_or("utf-8");
            let encoding =
                encoding_rs::Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
            let body = bounded_body(res, self.limits.max_body_bytes).await?;
            return Ok(WebFetchOutput {
                status_code,
                content: encoding.decode(&body).0.into_owned(),
            });
        }
        unreachable!("redirect limit is checked before continuing")
    }
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
    #[schemars(
        description = "The HTTP response body decoded as text. It may contain raw HTML or an error page; it is not extracted article text or rendered browser content."
    )]
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
        Some(
            "Purpose: Retrieve a known URL with an HTTP GET request.\n\
             Input: Provide url as an absolute HTTP or HTTPS URL.\n\
             Output: Returns status_code and the response body as text, including raw HTML when supplied by the server. Non-success HTTP responses are also returned.\n\
             When to use: To inspect a source page or verify evidence after discovering its URL. Check status_code before interpreting content. JavaScript is not executed.",
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
    fn fixture_tool(addr: std::net::SocketAddr, limits: WebRequestLimits) -> WebFetchTool {
        let client = limits
            .client_builder()
            .unwrap()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve("public.example", addr)
            .build()
            .unwrap();
        WebFetchTool { client, limits }
    }

    #[tokio::test]
    async fn rejects_literal_and_dns_loopback_destinations() {
        let tool = WebFetchTool::default();
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
        let tool = fixture_tool(addr, WebRequestLimits::default());
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
            );
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
                assert_eq!(result.content, "hello");
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
        );
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
        let tool = fixture_tool(addr, WebRequestLimits::default());
        let result = tool
            .call(WebFetchArgs {
                url: format!("http://public.example:{}/", addr.port()),
            })
            .await
            .unwrap();
        assert_eq!(result.content, "hi");
        assert_eq!(result.status_code, 200);
        task.await.unwrap();
        final_task.await.unwrap();
    }
}
