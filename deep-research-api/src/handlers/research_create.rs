use actix_web::post;
use actix_web::web::{Bytes, Data, Json};
use deep_research_orchestrator::DeepResearchOrchestrator;
use deep_research_orchestrator::plan::DeepResearchPlan;
use deep_research_orchestrator::stream::ResearchEvent;
use futures_util::{Stream, StreamExt};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct ResearchCreateRequest {
    plan: DeepResearchPlan,
}

#[post("/api/v1/deep/research")]
pub(super) async fn research_create(
    orchestrator: Data<DeepResearchOrchestrator>,
    json: Json<ResearchCreateRequest>,
) -> actix_web::Result<actix_web::HttpResponse> {
    let plan = json.into_inner().plan;
    plan.validate().map_err(actix_web::error::ErrorBadRequest)?;
    let stream = orchestrator.get_ref().clone().run_deep_research(plan);
    Ok(research_response(stream, false))
}

fn research_response(
    stream: impl Stream<Item = ResearchEvent> + 'static,
    use_accel_buffering: bool,
) -> actix_web::HttpResponse {
    let body = async_stream::stream! {
        let mut stream = std::pin::pin!(stream);
        let period = std::time::Duration::from_secs(15);
        let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                event = stream.next() => {
                    let Some(event) = event else { break; };
                    match serde_json::to_string(&event) {
                        Ok(data) => yield Ok::<_, actix_web::Error>(Bytes::from(format!("data: {data}\n\n"))),
                        Err(error) => {
                            yield Err(actix_web::error::ErrorInternalServerError(error));
                            break;
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    yield Ok(Bytes::from_static(b": keep-alive\n\n"));
                }
            }
        }
    };
    actix_web::HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header_if(!use_accel_buffering, ("X-Accel-Buffering", "no"))
        .streaming(body)
}

trait InsertHeaderIf {
    fn insert_header_if(&mut self, condition: bool, header: (&str, &str)) -> &mut Self;
}

impl InsertHeaderIf for actix_web::HttpResponseBuilder {
    fn insert_header_if(&mut self, condition: bool, header: (&str, &str)) -> &mut Self {
        if condition {
            self.insert_header(header)
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, body::MessageBody, http::StatusCode, test};
    use async_trait::async_trait;
    use deep_research_arbiter::{AgentConcurrencyArbiter, ArbiterTabletGuard};
    use deep_research_react_agent::ReActAgent;
    use deep_research_runner::OneshotRunner;
    use deep_research_tools::DeepResearchTools;
    use genai::{Client, chat::ChatOptions};
    use serde_json::{Value, json};
    use std::{pin::pin, sync::Arc, time::Duration};

    struct FailingArbiter;

    #[async_trait]
    impl AgentConcurrencyArbiter for FailingArbiter {
        async fn acquire(&self, _: &str) -> anyhow::Result<ArbiterTabletGuard> {
            anyhow::bail!("test runner unavailable")
        }
    }

    fn orchestrator() -> DeepResearchOrchestrator {
        let agent = ReActAgent::new(
            OneshotRunner::new(
                "test-model".into(),
                Client::default(),
                Arc::new(FailingArbiter),
                ChatOptions::default(),
            ),
            DeepResearchTools::default(),
            String::new(),
            Default::default(),
        )
        .unwrap();
        DeepResearchOrchestrator::new(agent.clone(), agent.clone(), agent.clone(), agent)
    }

    fn decode_events(body: &[u8]) -> Vec<Value> {
        std::str::from_utf8(body)
            .unwrap()
            .split("\n\n")
            .filter(|frame| !frame.is_empty())
            .map(|frame| serde_json::from_str(frame.strip_prefix("data: ").unwrap()).unwrap())
            .collect()
    }

    #[actix_web::test]
    async fn streams_each_event_before_research_finishes() {
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|event| (event, rx))
        });
        let response = research_response(stream, false);
        assert_eq!(
            response.headers().get("Content-Type").unwrap(),
            "text/event-stream"
        );
        assert_eq!(response.headers().get("Cache-Control").unwrap(), "no-cache");
        assert_eq!(response.headers().get("X-Accel-Buffering").unwrap(), "no");
        let mut body = pin!(response.into_body());

        let progress = json!({
            "model": "test-model", "phase": "Researching",
            "data": { "Message": { "message": "調査中\n次の行\r\ndata: injected" } }
        });
        tx.send(serde_json::from_value(progress.clone()).unwrap())
            .await
            .unwrap();
        let chunk = tokio::time::timeout(
            Duration::from_secs(1),
            futures_util::future::poll_fn(|cx| body.as_mut().poll_next(cx)),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert_eq!(decode_events(&chunk), vec![progress]);
        assert_eq!(chunk.iter().filter(|&&b| b == b'\n').count(), 2);

        let completed = json!({ "model": "test-model", "phase": "Synthesized", "data": {"title": "Report", "summary": "Summary", "sections": [], "limitations": []} });
        tx.send(serde_json::from_value(completed.clone()).unwrap())
            .await
            .unwrap();
        drop(tx);
        let chunk = futures_util::future::poll_fn(|cx| body.as_mut().poll_next(cx))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decode_events(&chunk), vec![completed]);
        assert!(
            futures_util::future::poll_fn(|cx| body.as_mut().poll_next(cx))
                .await
                .is_none()
        );
    }

    #[actix_web::test]
    async fn buffering_header_is_omitted_when_accel_buffering_is_enabled() {
        let response = research_response(futures_util::stream::empty::<ResearchEvent>(), true);
        assert!(!response.headers().contains_key("X-Accel-Buffering"));
    }

    #[actix_web::test]
    async fn research_failures_emit_terminal_event_and_close() {
        let app = test::init_service(
            App::new()
                .app_data(Data::new(orchestrator()))
                .configure(crate::handlers::configure),
        )
        .await;
        let request = test::TestRequest::post()
            .uri("/api/v1/deep/research")
            .set_json(json!({ "plan": {
                "research_plans": [{"goal": "research goal", "scope": "scope", "questions": ["question"]}], "report_plan": { "goal": "report goal", "sections": [{"heading": "Results", "focus": "Answer"}] }
            }}))
            .to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = tokio::time::timeout(Duration::from_secs(1), test::read_body(response))
            .await
            .expect("research stream must close after failure");
        let events = decode_events(&body);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["phase"], "Researching");
        assert!(
            events[0]["data"]["Error"]["error"]
                .as_str()
                .unwrap()
                .contains("test runner unavailable")
        );
        assert_eq!(events[1]["phase"], "Failed");
        assert!(events[1]["data"]["error"].is_string());
    }

    #[actix_web::test]
    async fn idle_stream_emits_keep_alive_comments() {
        tokio::time::pause();
        let stream = futures_util::stream::pending::<ResearchEvent>();
        let mut body = pin!(research_response(stream, false).into_body());
        for _ in 0..2 {
            let chunk = futures_util::future::poll_fn(|cx| body.as_mut().poll_next(cx))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(chunk, Bytes::from_static(b": keep-alive\n\n"));
        }
    }

    #[actix_web::test]
    async fn invalid_requests_are_rejected_before_streaming() {
        let app = test::init_service(
            App::new()
                .app_data(Data::new(orchestrator()))
                .configure(crate::handlers::configure),
        )
        .await;
        for body in [
            "",
            "{",
            "{}",
            r#"{"plan":{"research_plans":[]}}"#,
            r#"{"plan":{"research_plans":[],"report_plan":{"goal":"report","sections":[{"heading":"Results","focus":"Answer"}]}}}"#,
            r#"{"plan":{"research_plans":[{"goal":"goal","scope":"scope","questions":[]}],"report_plan":{"goal":"report","sections":[{"heading":"Results","focus":"Answer"}]}}}"#,
            r#"{"plan":{"research_plans":[{"goal":42}],"report_plan":{"goal":"report"}}}"#,
        ] {
            let request = test::TestRequest::post()
                .uri("/api/v1/deep/research")
                .insert_header(("Content-Type", "application/json"))
                .set_payload(body)
                .to_request();
            let response = test::call_service(&app, request).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_ne!(
                response.headers().get("Content-Type").unwrap(),
                "text/event-stream"
            );
        }
        let response =
            test::call_service(&app, test::TestRequest::get().uri("/version").to_request()).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}
