use crate::research::ResearchStepOutput;
use crate::synthesizer::FinalReport;
use deep_research_react_agent::event::AgentEvent;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchEvent {
    pub(crate) model: String,
    #[serde(flatten)]
    pub(crate) phase: ResearchPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", content = "data")]
pub enum ResearchPhase {
    Researching(AgentEvent),
    GapJudging(AgentEvent),
    ResearchStepCompleted(ResearchStepOutput),
    Synthesizing(AgentEvent),
    Synthesized(FinalReport),
    Failed { error: String },
}

impl ResearchEvent {
    pub(crate) fn failed(model: String, error: String) -> Self {
        Self {
            model,
            phase: ResearchPhase::Failed { error },
        }
    }

    pub(crate) fn from_research_event(model: String, event: AgentEvent) -> Self {
        Self {
            model,
            phase: ResearchPhase::Researching(event),
        }
    }

    pub(crate) fn from_gap_judging_event(model: String, event: AgentEvent) -> Self {
        Self {
            model,
            phase: ResearchPhase::GapJudging(event),
        }
    }

    pub(crate) fn from_research_step_completed_event(
        model: String,
        output: ResearchStepOutput,
    ) -> Self {
        Self {
            model,
            phase: ResearchPhase::ResearchStepCompleted(output),
        }
    }

    pub(crate) fn from_synthesizing_event(model: String, event: AgentEvent) -> Self {
        Self {
            model,
            phase: ResearchPhase::Synthesizing(event),
        }
    }

    pub(crate) fn from_synthesized_event(model: String, report: FinalReport) -> Self {
        Self {
            model,
            phase: ResearchPhase::Synthesized(report),
        }
    }
}

pub struct ResearchEventStream {
    rx: tokio::sync::mpsc::Receiver<ResearchEvent>,
    task: tokio::task::JoinHandle<()>,
}

impl ResearchEventStream {
    pub(crate) fn new(
        rx: tokio::sync::mpsc::Receiver<ResearchEvent>,
        task: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self { rx, task }
    }
}

impl Stream for ResearchEventStream {
    type Item = ResearchEvent;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

impl Drop for ResearchEventStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::time::Duration;

    #[tokio::test]
    async fn closed_channel_drains_events_and_ends() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let task = tokio::spawn(async move {
            tx.send(ResearchEvent::failed("model".into(), "failure".into()))
                .await
                .unwrap();
        });
        let mut stream = ResearchEventStream::new(rx, task);
        assert!(stream.next().await.is_some());
        assert!(
            tokio::time::timeout(Duration::from_secs(1), stream.next())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn dropping_stream_cancels_task_and_children() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _tx = tx;
            let mut children = tokio::task::JoinSet::new();
            children.spawn(async move {
                let _dropped = dropped_tx;
                started_tx.send(()).unwrap();
                futures::future::pending::<()>().await;
            });
            children.join_next().await;
        });
        let stream = ResearchEventStream::new(rx, task);
        started_rx.await.unwrap();
        drop(stream);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), dropped_rx)
                .await
                .unwrap()
                .is_err()
        );
    }
}
