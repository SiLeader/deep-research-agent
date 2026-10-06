use super::*;
use deep_research_arbiter::semaphore::SemaphoreConcurrencyArbiter;
use deep_research_runner::OneshotRunner;
use deep_research_tools::DeepResearchTools;
use futures::StreamExt;
use genai::adapter::AdapterKind;
use genai::chat::ChatOptions;
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// A deterministic local OpenAI-compatible endpoint: no external LLM or credentials.
async fn read_request(socket: &mut tokio::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    let (body_start, body_length) = loop {
        let mut buffer = [0; 4096];
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0, "connection closed before request headers");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            break (end + 4, length);
        }
    };
    while bytes.len() < body_start + body_length {
        let mut buffer = [0; 4096];
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0, "connection closed before request body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    serde_json::from_slice(&bytes[body_start..body_start + body_length]).unwrap()
}

fn assert_simple_schema(value: &Value) {
    match value {
        Value::Object(object) => {
            for unsupported in ["$ref", "anyOf", "oneOf", "allOf"] {
                assert!(
                    !object.contains_key(unsupported),
                    "unexpected {unsupported}: {value}"
                );
            }
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                assert_eq!(object["additionalProperties"], false);
                let required = object["required"].as_array().unwrap();
                assert_eq!(required.len(), properties.len());
                for name in properties.keys() {
                    assert!(required.contains(&json!(name)));
                }
            }
            for nested in object.values() {
                assert_simple_schema(nested);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_simple_schema(item);
            }
        }
        _ => {}
    }
}

fn prompt_input(request: &Value) -> Value {
    let prompt = request["messages"][1]["content"].as_str().unwrap();
    serde_json::from_str(prompt.split_once("# Input (JSON)\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn structured_pipeline_passes_schemas_retry_feedback_and_final_report() {
    for invalid_report in [false, true] {
        tokio::time::timeout(Duration::from_secs(10), async {
        let plan = crate::plan::tests::plan_value();
        let mut initial = serde_json::to_value(crate::research::tests::output()).unwrap();
        initial["findings"][0]["question"] = json!("question");
        initial["findings"][0]["status"] = json!("partial");
        initial["limitations"] = json!(["Need a primary source"]);
        let mut revised = initial.clone();
        revised["findings"][0]["status"] = json!("supported");
        revised["limitations"] = json!([]);
        let rejection = json!({"approved": false, "gaps": [{"question": "question",
            "kind": "insufficient_evidence", "reason": "Need a primary source", "next_action": "Find a primary source"}]});
        let mut report = json!({"title": "Report", "summary": "Answer", "sections": [{
            "heading": "Results", "content": "Supported answer", "sources": ["https://example.com"]}], "limitations": []});
        if invalid_report {
            report["sections"][0]["sources"] = json!(["https://invented.example"]);
        }
        let responses = vec![plan.clone(), initial.clone(), rejection.clone(), revised.clone(),
            json!({"approved": true, "gaps": []}), report.clone()];
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/api/v1/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for arguments in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                let submit = request["tools"].as_array().unwrap().iter()
                    .find(|tool| tool["function"]["name"] == "submit").expect("registered submit must reach the LLM");
                assert_simple_schema(&submit["function"]["parameters"]);
                requests.push(request);
                let body = json!({"id": "test", "object": "chat.completion", "created": 0, "model": "gpt-test",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": null,
                    "tool_calls": [{"id": "submit-call", "type": "function", "function": {
                        "name": "submit", "arguments": arguments.to_string()}}]}, "finish_reason": "tool_calls"}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}).to_string();
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        let client = Client::builder().with_service_target_resolver(ServiceTargetResolver::from_resolver_fn(
            move |target: ServiceTarget| Ok(ServiceTarget {
                endpoint: Endpoint::from_owned(endpoint.clone()), auth: AuthData::from_single("test"),
                model: ModelIden::new(AdapterKind::OpenAI, target.model.model_name),
            })
        )).build();
        let agent = ReActAgent::new(OneshotRunner::new("gpt-test".into(), client,
            Arc::new(SemaphoreConcurrencyArbiter::new(HashMap::from([("gpt-test".into(), 1)]))),
            ChatOptions::default()), DeepResearchTools::default(), String::new(), Default::default()).unwrap();
        let orchestrator = DeepResearchOrchestrator::new(agent.clone(), agent.clone(), agent.clone(), agent);
        let planned = orchestrator.plan("question".into()).await.unwrap();
        assert_eq!(serde_json::to_value(&planned).unwrap(), plan);
        let events: Vec<_> = orchestrator.run_deep_research(planned).collect().await;
        let events: Vec<Value> = events.into_iter().map(|event| serde_json::to_value(event).unwrap()).collect();
        if invalid_report {
            assert_eq!(events.last().unwrap()["phase"], "Failed");
            assert!(events.last().unwrap()["data"]["error"].as_str().unwrap().contains("unknown source"));
        } else {
            assert!(events.iter().all(|event| event["phase"] != "Failed"), "{events:?}");
            assert_eq!(events.last().unwrap()["phase"], "Synthesized");
            assert_eq!(events.last().unwrap()["data"], report);
        }
        assert_eq!(events.iter().find(|event| event["phase"] == "ResearchStepCompleted").unwrap()["data"], revised);
        let requests = server.await.unwrap();
        let retry = prompt_input(&requests[3]);
        assert_eq!(retry["previous_research_output"], initial);
        assert_eq!(retry["previous_gap_analysis"], rejection);
        let synthesis = prompt_input(&requests[5]);
        assert_eq!(synthesis["research_outputs"][0]["research_plan"], plan["research_plans"][0]);
        assert_eq!(synthesis["research_outputs"][0]["research_output"], revised);
    }).await.expect("pipeline must finish without extra LLM calls");
    }
}
