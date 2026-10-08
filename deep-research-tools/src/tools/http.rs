use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebRequestLimits {
    pub connect_timeout_secs: u64,
    pub request_timeout_secs: u64,
    pub max_body_bytes: usize,
}

impl Default for WebRequestLimits {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 10,
            request_timeout_secs: 60,
            max_body_bytes: 2 * 1024 * 1024,
        }
    }
}

impl WebRequestLimits {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.connect_timeout_secs > 0,
            "connect_timeout_secs must be positive"
        );
        anyhow::ensure!(
            self.request_timeout_secs > 0,
            "request_timeout_secs must be positive"
        );
        anyhow::ensure!(self.max_body_bytes > 0, "max_body_bytes must be positive");
        Ok(())
    }

    pub(crate) fn client_builder(&self) -> anyhow::Result<reqwest::ClientBuilder> {
        self.validate()?;
        Ok(reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(self.connect_timeout_secs))
            .timeout(Duration::from_secs(self.request_timeout_secs)))
    }
}

pub(crate) async fn bounded_body(
    mut response: reqwest::Response,
    limit: usize,
) -> anyhow::Result<Vec<u8>> {
    if let Some(length) = response.content_length() {
        anyhow::ensure!(
            length <= limit as u64,
            "HTTP response body exceeds {limit} bytes"
        );
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            chunk.len() <= limit - body.len(),
            "HTTP response body exceeds {limit} bytes"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub(crate) async fn server(
        response: String,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
        delayed_server(response, Duration::ZERO).await
    }

    /// Like [`server`], but waits `delay` after reading the request.
    pub(crate) async fn delayed_server(
        response: String,
        delay: Duration,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            tokio::time::sleep(delay).await;
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });
        (addr, task)
    }

    #[tokio::test]
    async fn bounds_fixed_and_chunked_response_bodies() {
        for response in [
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nhel\r\n2\r\nlo\r\n0\r\n\r\n",
        ] {
            for limit in [4, 5] {
                let (addr, task) = server(response.to_string()).await;
                let res = reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .get(format!("http://{addr}/"))
                    .send()
                    .await
                    .unwrap();
                let body = bounded_body(res, limit).await;
                if limit == 4 {
                    assert!(body.unwrap_err().to_string().contains("exceeds"));
                } else {
                    assert_eq!(body.unwrap(), b"hello");
                }
                task.await.unwrap();
            }
        }
    }
}
