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
}

impl ResearchEvent {
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
}

impl ResearchEventStream {
    pub(crate) fn new(rx: tokio::sync::mpsc::Receiver<ResearchEvent>) -> Self {
        Self { rx }
    }
}

impl Stream for ResearchEventStream {
    type Item = ResearchEvent;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if let std::task::Poll::Ready(Some(event)) = self.rx.poll_recv(cx) {
            return std::task::Poll::Ready(Some(event));
        }
        std::task::Poll::Pending
    }
}
