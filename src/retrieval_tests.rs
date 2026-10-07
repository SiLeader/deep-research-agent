use super::*;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

async fn fixture(responses: Vec<Value>) -> (String, JoinHandle<Vec<(String, Value)>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).into_owned();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        requests.push((
                            headers,
                            serde_json::from_slice(&bytes[end + 4..end + 4 + length])
                                .unwrap_or(Value::Null),
                        ));
                        break;
                    }
                }
            }
            let body = response.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (endpoint, task)
}

fn services(endpoint: String, provider_type: config::ProviderType) -> ModelServices {
    build_model_services(
        vec![config::ModelConfig {
            id: "embedding".into(),
            provider: "fixture".into(),
            name: "fixture-embedding".into(),
            max_concurrency: 1,
        }],
        vec![config::ProviderConfig {
            id: "fixture".into(),
            provider_type,
            api_key_env: "PATH".into(),
            endpoint: Some(endpoint),
        }],
    )
    .unwrap()
}

fn embedding() -> Value {
    json!({"object": "list", "model": "fixture-embedding", "data": [{"object": "embedding", "index": 0, "embedding": [1.0, 0.0]}], "usage": {"prompt_tokens": 1, "total_tokens": 1}})
}

#[tokio::test]
async fn configured_search_tools_support_all_four_retrieval_modes() {
    for use_embedding in [false, true] {
        for use_reranker in [false, true] {
            let (endpoint, embedding_task) = fixture(if use_embedding {
                vec![embedding(), embedding()]
            } else {
                vec![]
            })
            .await;
            let services = services(endpoint, config::ProviderType::OpenAI);
            let (rerank_endpoint, rerank_task) = fixture(if use_reranker {
                vec![json!({"results": [{"index": 0, "relevance_score": 0.95}]})]
            } else {
                vec![]
            })
            .await;
            let (search_endpoint, search_task) = fixture(vec![json!({"results": [{"url": "https://evidence.test", "title": "Evidence", "score": 1.0, "content": "apple orchard"}]})]).await;
            let embedding_config = if use_embedding {
                "[tools.fetched.embedding]\nmodel = 'embedding'\n"
            } else {
                ""
            };
            let reranker_config = if use_reranker {
                format!(
                    "[tools.fetched.reranker]\nmodel = 'fixture-reranker'\nendpoint = '{rerank_endpoint}'\napi_key_env = 'PATH'\n"
                )
            } else {
                String::new()
            };
            let config: config::Config = toml::from_str(&format!("models = []\nproviders = []\n[tools.web_search.searxng]\nendpoint = '{search_endpoint}'\n{embedding_config}{reranker_config}")).unwrap();
            let retrieval = RetrievalModels::new(&config.tools.fetched, &services).unwrap();
            let tools = build_search_tools(config.tools.clone(), retrieval.clone())
                .await
                .unwrap();
            let isolated = build_search_tools(config.tools, retrieval).await.unwrap();
            let search = tools
                .call("search_sources", json!({"query": "apple"}))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(search["pages"][0]["url"], "https://evidence.test");
            let results = tools
                .call("search_fetched", json!({"query": "apple", "top_k": null}))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(results["chunks"][0]["url"], "https://evidence.test");
            if use_reranker {
                assert!((results["chunks"][0]["score"].as_f64().unwrap() - 0.95).abs() < 1e-6);
            }
            let empty = isolated
                .call("search_fetched", json!({"query": "apple", "top_k": null}))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(empty["chunks"], json!([]));
            search_task.await.unwrap();
            let requests = embedding_task.await.unwrap();
            assert_eq!(requests.len(), if use_embedding { 2 } else { 0 });
            for (headers, body) in requests {
                assert!(headers.starts_with("POST /v1/embeddings "));
                assert_eq!(body["model"], "fixture-embedding");
                assert!(headers.to_lowercase().contains("authorization: bearer "));
            }
            let requests = rerank_task.await.unwrap();
            if use_reranker {
                assert!(requests[0].0.starts_with("POST /v1/rerank "));
                assert!(
                    requests[0]
                        .0
                        .to_lowercase()
                        .contains("authorization: bearer ")
                );
                assert_eq!(requests[0].1["model"], "fixture-reranker");
                assert_eq!(requests[0].1["documents"], json!(["apple orchard"]));
            }
        }
    }
}

#[tokio::test]
async fn embedding_calls_respect_the_shared_model_budget() {
    let (endpoint, task) = fixture(vec![embedding()]).await;
    let services = services(endpoint, config::ProviderType::OpenAI);
    let settings: config::FetchedSettings =
        toml::from_str("[embedding]\nmodel = 'embedding'").unwrap();
    let retrieval = RetrievalModels::new(&settings, &services).unwrap();
    let db = FetchedDb::new(1024, retrieval.embedder, None)
        .await
        .unwrap();
    let permit = services.arbiter.acquire("embedding").await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(30),
            db.add_text("https://test", "apple")
        )
        .await
        .is_err()
    );
    drop(permit);
    db.add_text("https://test", "apple").await.unwrap();
    assert_eq!(task.await.unwrap().len(), 1);
}

#[test]
fn retrieval_configuration_rejects_unknown_and_unsupported_models() {
    let services = services(
        "http://localhost/v1/".into(),
        config::ProviderType::Anthropic,
    );
    for (model, message) in [
        ("missing", "Unknown tools.fetched.embedding.model ID"),
        ("embedding", "OpenAI-compatible provider"),
    ] {
        let settings: config::FetchedSettings =
            toml::from_str(&format!("[embedding]\nmodel = '{model}'")).unwrap();
        let error = RetrievalModels::new(&settings, &services).err().unwrap();
        assert!(error.to_string().contains(message));
    }
    let settings: config::FetchedSettings =
        toml::from_str("[reranker]\nmodel = 'rank'\nendpoint = 'file:///tmp'").unwrap();
    assert!(RetrievalModels::new(&settings, &services).is_err());
}

#[test]
fn retrieval_settings_default_and_reject_invalid_values() {
    let defaults: config::FetchedSettings = toml::from_str("").unwrap();
    assert!(defaults.embedding.is_none());
    assert!(defaults.reranker.is_none());
    defaults.validate().unwrap();
    let enabled: config::FetchedSettings = toml::from_str("chunk_size = 128\n[embedding]\nmodel = 'embedding'\n[reranker]\nmodel = 'rank'\nendpoint = 'http://localhost/v1/'").unwrap();
    assert_eq!(enabled.search.chunk_size, 128);
    assert_eq!(enabled.embedding.unwrap().request_timeout_secs, 60);
    let reranker = enabled.reranker.unwrap();
    assert_eq!(reranker.request_timeout_secs, 60);
    assert_eq!(reranker.max_concurrency, 1);
    assert!(reranker.api_key_env.is_none());
    for text in [
        "[embedding]\nmodel = ''",
        "[embedding]\nmodel = 'embed'\nrequest_timeout_secs = 0",
        "[reranker]\nmodel = ''\nendpoint = 'http://localhost'",
        "[reranker]\nmodel = 'rank'\nendpoint = ''",
        "[reranker]\nmodel = 'rank'\nendpoint = 'http://localhost'\napi_key_env = ''",
        "[reranker]\nmodel = 'rank'\nendpoint = 'http://localhost'\nrequest_timeout_secs = 0",
        "[reranker]\nmodel = 'rank'\nendpoint = 'http://localhost'\nmax_concurrency = 0",
    ] {
        assert!(
            toml::from_str::<config::FetchedSettings>(text)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    for endpoint in [
        "invalid",
        "file:///tmp",
        "https://user:password@example.test/v1",
        "https://example.test/v1?key=secret",
        "https://example.test/#fragment",
    ] {
        assert!(DeepResearchReranker::new_with_options(endpoint.into(), None, 60, 1).is_err());
    }
}

async fn unresponsive_fixture() -> (String, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    (endpoint, task)
}

#[tokio::test]
async fn configured_embedding_and_reranking_requests_time_out() {
    let (embed_endpoint, embed_task) = unresponsive_fixture().await;
    let (rerank_endpoint, rerank_task) = unresponsive_fixture().await;
    let services = services(embed_endpoint, config::ProviderType::OpenAI);
    let settings: config::FetchedSettings = toml::from_str(&format!("[embedding]\nmodel = 'embedding'\nrequest_timeout_secs = 1\n[reranker]\nmodel = 'rank'\nendpoint = '{rerank_endpoint}'\nrequest_timeout_secs = 1")).unwrap();
    let retrieval = RetrievalModels::new(&settings, &services).unwrap();
    let embed_db = FetchedDb::new(1024, retrieval.embedder, None)
        .await
        .unwrap();
    let rank_db = FetchedDb::new(1024, None, retrieval.reranker)
        .await
        .unwrap();
    rank_db.add_text("https://test", "apple").await.unwrap();
    let results = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(
            embed_db.add_text("https://test", "apple"),
            rank_db.search("apple", 1)
        )
    })
    .await
    .unwrap();
    assert!(
        results
            .0
            .unwrap_err()
            .to_string()
            .contains("embedding request timed out")
    );
    assert!(results.1.is_err());
    assert!(embed_db.search("apple", 1).await.unwrap().is_empty());
    embed_task.abort();
    rerank_task.abort();
}

#[tokio::test]
async fn missing_reranker_credentials_are_reported_as_tool_errors() {
    let (endpoint, task) = fixture(vec![]).await;
    let key_env = "DEEP_RESEARCH_TEST_MISSING_RERANK_KEY_7E79C50C";
    assert!(std::env::var_os(key_env).is_none());
    let client =
        DeepResearchReranker::new_with_options(endpoint, Some(key_env.into()), 60, 1).unwrap();
    let error = client
        .rerank_indices("rank".into(), "apple".into(), vec!["apple".into()], 1)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Missing reranker API key environment variable")
    );
    assert!(task.await.unwrap().is_empty());
}

#[tokio::test]
async fn reranker_clones_share_the_configured_concurrency_budget() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = DeepResearchReranker::new_with_options(
        format!("http://{}", listener.local_addr().unwrap()),
        None,
        60,
        1,
    )
    .unwrap();
    let clone = client.clone();
    let first = tokio::spawn(async move {
        client
            .rerank_indices("rank".into(), "apple".into(), vec!["apple".into()], 1)
            .await
    });
    let (mut first_socket, _) = listener.accept().await.unwrap();
    let second = tokio::spawn(async move {
        clone
            .rerank_indices("rank".into(), "apple".into(), vec!["apple".into()], 1)
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    let body = json!({"results": [{"index": 0, "relevance_score": 0.9}]}).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // Consume each request before closing the fixture's connection.
    async fn respond(socket: &mut tokio::net::TcpStream, response: &str) {
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        socket.write_all(response.as_bytes()).await.unwrap();
    }
    respond(&mut first_socket, &response).await;
    first.await.unwrap().unwrap();
    let (mut second_socket, _) =
        tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
    respond(&mut second_socket, &response).await;
    second.await.unwrap().unwrap();
}
