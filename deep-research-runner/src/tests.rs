use super::*;
use deep_research_arbiter::semaphore::SemaphoreConcurrencyArbiter;
use genai::{
    ModelIden, ServiceTarget,
    adapter::AdapterKind,
    resolver::{AuthData, Endpoint},
};
use std::{collections::HashMap, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn read_request(socket: &mut tokio::net::TcpStream) {
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
                return;
            }
        }
    }
}

#[tokio::test]
async fn stalled_request_times_out_and_releases_model_budget() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stalled, _) = listener.accept().await.unwrap();
            read_request(&mut stalled).await;
            // Keep the first connection open without sending a response.
            let (mut next, _) = listener.accept().await.unwrap();
            read_request(&mut next).await;
            let body = serde_json::json!({
                "id": "test", "object": "chat.completion", "created": 0, "model": "gpt-test",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "recovered"}, "finish_reason": "stop"}]
            }).to_string();
            next.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            drop(stalled);
        });
        let client = Client::builder().with_service_target_resolver_fn(move |_: ServiceTarget| {
            Ok(ServiceTarget {
                model: ModelIden::new(AdapterKind::OpenAI, "gpt-test"),
                auth: AuthData::from_single("test"),
                endpoint: Endpoint::from_owned(format!("http://{addr}/")),
            })
        }).build();
        let arbiter = Arc::new(SemaphoreConcurrencyArbiter::new(HashMap::from([("model".into(), 1)])));
        let runner = OneshotRunner::new("model".into(), client, arbiter.clone(), ChatOptions::default());
        assert_eq!(runner.request_timeout, Duration::from_secs(120));
        assert!(runner.clone().with_request_timeout(0).is_err());
        let runner = runner.with_request_timeout(1).unwrap();
        let error = runner.run(vec![ChatMessage::user("test")], vec![]).await.unwrap_err();
        assert!(error.to_string().contains("LLM request timed out for model: model"));
        let permit = tokio::time::timeout(Duration::from_millis(100), arbiter.acquire("model"))
            .await.unwrap().unwrap();
        drop(permit);
        let response = runner.run(vec![ChatMessage::user("retry")], vec![]).await.unwrap();
        assert_eq!(response.first_text(), Some("recovered"));
        server.await.unwrap();
    }).await.unwrap();
}

fn fixture_client(addr: std::net::SocketAddr) -> Client {
    Client::builder()
        .with_service_target_resolver_fn(move |_: ServiceTarget| {
            Ok(ServiceTarget {
                model: ModelIden::new(AdapterKind::OpenAI, "gpt-test"),
                auth: AuthData::from_single("test"),
                endpoint: Endpoint::from_owned(format!("http://{addr}/")),
            })
        })
        .build()
}

fn ok_response() -> String {
    let body = serde_json::json!({
        "id": "test", "object": "chat.completion", "created": 0, "model": "gpt-test",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "recovered"}, "finish_reason": "stop"}]
    }).to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn transient_failures_are_retried_and_others_are_not() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for (status, retried) in [(503, true), (429, true), (400, false)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_request(&mut socket).await;
                let body = "{\"error\": {\"message\": \"busy\"}}";
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nRetry-After: 0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                drop(socket);
                if retried {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    read_request(&mut socket).await;
                    socket.write_all(ok_response().as_bytes()).await.unwrap();
                }
            });
            let arbiter = Arc::new(SemaphoreConcurrencyArbiter::new(HashMap::from([("model".into(), 1)])));
            let runner = OneshotRunner::new("model".into(), fixture_client(addr), arbiter, ChatOptions::default())
                .with_max_retries(1);
            let result = runner.run(vec![ChatMessage::user("test")], vec![]).await;
            assert_eq!(result.is_ok(), retried, "{status}");
            server.await.unwrap();
        }
    }).await.unwrap();
}

#[tokio::test]
async fn call_budget_is_shared_and_enforced_before_requests() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        socket.write_all(ok_response().as_bytes()).await.unwrap();
    });
    let arbiter = Arc::new(SemaphoreConcurrencyArbiter::new(HashMap::from([(
        "model".into(),
        1,
    )])));
    let runner = OneshotRunner::new(
        "model".into(),
        fixture_client(addr),
        arbiter,
        ChatOptions::default(),
    );
    let budget = CallBudget::new(1);
    with_call_budget(budget.clone(), async {
        runner
            .run(vec![ChatMessage::user("a")], vec![])
            .await
            .unwrap();
        let error = runner
            .run(vec![ChatMessage::user("b")], vec![])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("budget exhausted"));
    })
    .await;
    server.await.unwrap();
}
