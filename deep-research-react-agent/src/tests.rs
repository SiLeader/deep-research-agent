use super::*;
use deep_research_arbiter::semaphore::SemaphoreConcurrencyArbiter;
use genai::{
    Client, ModelIden, ServiceTarget,
    adapter::AdapterKind,
    chat::ChatOptions,
    resolver::{AuthData, Endpoint},
};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn agent(finish_last: bool) -> (ReActAgent, tokio::task::JoinHandle<usize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut count = 0;
        for index in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            count += 1;
            let name = if finish_last && index == 1 {
                "submit"
            } else {
                "unknown_tool"
            };
            let body = json!({"id": "test", "object": "chat.completion", "created": 0, "model": "gpt-test",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": null,
                "tool_calls": [{"id": format!("call-{index}"), "type": "function", "function": {"name": name, "arguments": "{}"}}]},
                "finish_reason": "tool_calls"}]}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        count
    });
    let client = Client::builder()
        .with_service_target_resolver_fn(move |_: ServiceTarget| {
            Ok(ServiceTarget {
                model: ModelIden::new(AdapterKind::OpenAI, "gpt-test"),
                auth: AuthData::from_single("test"),
                endpoint: Endpoint::from_owned(format!("http://{addr}/")),
            })
        })
        .build();
    let arbiter = Arc::new(SemaphoreConcurrencyArbiter::new(HashMap::from([(
        "gpt-test".into(),
        1,
    )])));
    let mut agent = ReActAgent::new(
        OneshotRunner::new("gpt-test".into(), client, arbiter, ChatOptions::default()),
        DeepResearchTools::default(),
        String::new(),
        HashSet::new(),
    )
    .unwrap()
    .with_max_llm_calls(2)
    .unwrap();
    agent.add_stop_tool(
        deep_research_tools::tools::marker::MarkerTool::<Value>::new(
            "submit".into(),
            None,
            Some(true),
            None,
        ),
    );
    (agent, task)
}

#[tokio::test]
async fn limits_both_execution_paths_and_allows_submit_on_last_call() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for streaming in [false, true] {
            for finish_last in [false, true] {
                let (agent, task) = agent(finish_last).await;
                let result = if streaming {
                    agent
                        .run_with_event::<Value, _, _>("test".into(), |_| async {})
                        .await
                } else {
                    agent.get_output::<Value>("test".into()).await
                };
                if finish_last {
                    assert_eq!(result.unwrap(), json!({}));
                } else {
                    assert!(
                        result
                            .unwrap_err()
                            .to_string()
                            .contains("exceeded max_llm_calls (2)")
                    );
                }
                assert_eq!(task.await.unwrap(), 2);
                assert!(agent.with_max_llm_calls(0).is_err());
            }
        }
    })
    .await
    .unwrap();
}

async fn scripted_agent(calls: Vec<Value>) -> (ReActAgent, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (index, call) in calls.into_iter().enumerate() {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let request = loop {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                            .unwrap();
                    }
                }
            };
            requests.push(request);
            let message = if let Some(text) = call.get("text") {
                json!({"role": "assistant", "content": text})
            } else {
                let calls = call
                    .get("calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_else(|| vec![call]);
                let tools: Vec<_> = calls
                    .into_iter()
                    .enumerate()
                    .map(|(position, call)| {
                        let id = if position == 0 {
                            format!("call-{index}")
                        } else {
                            format!("call-{index}-{position}")
                        };
                        json!({"id": id, "type": "function", "function": {
                            "name": call["name"], "arguments": call["arguments"].to_string()
                        }})
                    })
                    .collect();
                json!({"role": "assistant", "content": null, "tool_calls": tools})
            };
            let finish_reason = if message.get("tool_calls").is_some() {
                "tool_calls"
            } else {
                "stop"
            };
            let body = json!({"id": "test", "object": "chat.completion", "created": 0, "model": "gpt-test",
                "choices": [{"index": 0, "message": message,
                "finish_reason": finish_reason}]}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let client = Client::builder()
        .with_service_target_resolver_fn(move |_: ServiceTarget| {
            Ok(ServiceTarget {
                model: ModelIden::new(AdapterKind::OpenAI, "gpt-test"),
                auth: AuthData::from_single("test"),
                endpoint: Endpoint::from_owned(format!("http://{addr}/")),
            })
        })
        .build();
    let arbiter = Arc::new(SemaphoreConcurrencyArbiter::new(HashMap::from([(
        "gpt-test".into(),
        1,
    )])));
    let mut agent = ReActAgent::new(
        OneshotRunner::new("gpt-test".into(), client, arbiter, ChatOptions::default()),
        DeepResearchTools::default(),
        String::new(),
        HashSet::new(),
    )
    .unwrap();
    agent.add_stop_tool(
        deep_research_tools::tools::marker::MarkerTool::<Value>::new(
            "submit".into(),
            None,
            Some(true),
            None,
        ),
    );
    (agent, task)
}

#[tokio::test]
async fn rejected_submissions_are_returned_to_the_model_for_correction() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (agent, task) = scripted_agent(vec![
            json!({"name": "submit", "arguments": {"value": -1}}),
            json!({"name": "submit", "arguments": {"value": 1}}),
        ])
        .await;
        let output: Value = agent
            .get_output_validated("test".into(), |value: &Value| {
                anyhow::ensure!(value["value"].as_i64() > Some(0), "value must be positive");
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(output, json!({"value": 1}));
        let requests = task.await.unwrap();
        let feedback = requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "tool")
            .unwrap();
        assert_eq!(feedback["tool_call_id"], "call-0");
        assert!(
            feedback["content"]
                .as_str()
                .unwrap()
                .contains("value must be positive")
        );
    })
    .await
    .unwrap();
}

#[test]
fn tool_context_drops_oldest_responses_first_then_truncates_latest() {
    use genai::chat::{ChatMessage, MessageContent, ToolResponse};
    let response = |id: &str, content: String| {
        ChatMessage::tool(MessageContent::from_tool_responses(vec![ToolResponse {
            call_id: id.into(),
            fn_name: Some("tool".into()),
            content,
        }]))
    };
    let mut messages = vec![
        ChatMessage::user("question"),
        response("old", "a".repeat(400)),
        response("mid", "b".repeat(400)),
        response("new", "c".repeat(400)),
    ];
    let contents = |messages: &[ChatMessage]| -> Vec<String> {
        messages[1..]
            .iter()
            .map(|message| message.content.tool_responses()[0].content.clone())
            .collect()
    };
    crate::component::compact_tool_responses(&mut messages, 2000);
    assert!(
        contents(&messages)
            .iter()
            .all(|content| content.len() == 400)
    );
    crate::component::compact_tool_responses(&mut messages, 900);
    let compacted = contents(&messages);
    assert!(compacted[0].starts_with("[Earlier tool output omitted"));
    assert_eq!(compacted[1], "b".repeat(400));
    assert_eq!(compacted[2], "c".repeat(400));
    crate::component::compact_tool_responses(&mut messages, 300);
    let compacted = contents(&messages);
    assert!(compacted[0].is_empty());
    assert!(compacted[1].is_empty());
    assert!(compacted[2].starts_with(&"c".repeat(260)));
    assert!(compacted[2].ends_with("[Truncated to fit the context limit.]"));
    assert_eq!(
        compacted
            .iter()
            .map(|text| text.chars().count())
            .sum::<usize>(),
        300
    );
}

#[tokio::test]
async fn text_only_responses_are_corrected_within_the_call_limit() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for finish in [false, true] {
            let (agent, task) = scripted_agent(vec![
                json!({"text": "Here is my answer"}),
                if finish {
                    json!({"name": "submit", "arguments": {"answer": "done"}})
                } else {
                    json!({"text": "Still not submitting"})
                },
            ])
            .await;
            let result = agent
                .with_max_llm_calls(2)
                .unwrap()
                .get_output::<Value>("test".into())
                .await;
            if finish {
                assert_eq!(result.unwrap(), json!({"answer": "done"}));
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("exceeded max_llm_calls (2)")
                );
            }
            let requests = task.await.unwrap();
            let messages = requests[1]["messages"].as_array().unwrap();
            assert_eq!(messages[2]["role"], "assistant");
            assert_eq!(messages[2]["content"], "Here is my answer");
            assert!(messages[3]["content"].as_str().unwrap().contains("submit"));
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn agents_without_stop_tools_still_finish_on_text() {
    let (mut agent, task) = scripted_agent(vec![json!({"text": "done"})]).await;
    agent.stop_tool_names.clear();
    let mut stream = agent.run_stream("test".into());
    use futures_util::StreamExt;
    assert!(matches!(
        stream.next().await,
        Some(event::AgentEvent::Message(_))
    ));
    assert!(stream.next().await.is_none());
    assert_eq!(task.await.unwrap().len(), 1);
}

#[tokio::test]
async fn mixed_tool_batches_execute_work_before_accepting_a_submission() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (mut agent, task) = scripted_agent(vec![
            json!({"calls": [
                {"name": "submit", "arguments": {"answer": "premature"}},
                {"name": "work", "arguments": {}}
            ]}),
            json!({"name": "submit", "arguments": {"answer": "reviewed"}}),
        ])
        .await;
        agent.tools.add(
            deep_research_tools::tools::marker::MarkerTool::<Value>::new(
                "work".into(),
                None,
                Some(true),
                None,
            ),
        );
        let result: Value = agent.get_output("test".into()).await.unwrap();
        assert_eq!(result, json!({"answer": "reviewed"}));
        let requests = task.await.unwrap();
        let responses: Vec<_> = requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect();
        assert_eq!(responses.len(), 2);
        assert!(
            responses[0]["content"]
                .as_str()
                .unwrap()
                .contains("separate turn")
        );
        assert_eq!(responses[1]["tool_call_id"], "call-0-1");
        assert_eq!(responses[1]["content"], "null");
    })
    .await
    .unwrap();
}

#[test]
fn compaction_enforces_exact_character_budgets_without_losing_call_identity() {
    use genai::chat::{ChatMessage, MessageContent, ToolResponse};
    for limit in [0, 1, 10, 40, 100, 300, 700] {
        let responses = |contents: &[&str]| {
            ChatMessage::tool(MessageContent::from_tool_responses(
                contents
                    .iter()
                    .enumerate()
                    .map(|(i, content)| ToolResponse {
                        call_id: format!("call-{i}"),
                        fn_name: Some("work".into()),
                        content: (*content).into(),
                    })
                    .collect(),
            ))
        };
        let mut messages = vec![
            ChatMessage::user("unchanged"),
            responses(&["short", "small"]),
            responses(&["old", &"前".repeat(400)]),
            responses(&[&"新".repeat(400), &"末".repeat(100)]),
        ];
        crate::component::compact_tool_responses(&mut messages, limit);
        let count: usize = messages
            .iter()
            .flat_map(|message| message.content.tool_responses())
            .map(|response| response.content.chars().count())
            .sum();
        assert!(count <= limit, "{count} > {limit}");
        for message in &messages[1..] {
            for (index, response) in message.content.tool_responses().iter().enumerate() {
                assert_eq!(response.call_id, format!("call-{index}"));
                assert_eq!(response.fn_name.as_deref(), Some("work"));
            }
        }
    }
}
