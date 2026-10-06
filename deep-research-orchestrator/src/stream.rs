use crate::research::ResearchStepOutput;
use deep_research_react_agent::event::AgentEvent;
use deep_research_react_agent::stream::AgentStream;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use tokio::sync::RwLock;

pub struct ResearchEvent {
    pub(crate) model: String,
    pub(crate) event: AgentEvent,
    pub(crate) phase: ResearchPhase,
}

pub enum ResearchPhase {
    Researching,
    GapJudging,
    Completed,
}

impl ResearchEvent {
    pub(crate) fn from_research_event(model: String, event: AgentEvent) -> Self {
        Self {
            model,
            event,
            phase: ResearchPhase::Researching,
        }
    }

    pub(crate) fn from_gap_judging_event(model: String, event: AgentEvent) -> Self {
        Self {
            model,
            event,
            phase: ResearchPhase::GapJudging,
        }
    }

    pub(crate) fn from_completed_event(model: String, output: ResearchStepOutput) -> Self {
        Self {
            model,
            event,
            phase: ResearchPhase::Completed,
        }
    }
}

pub struct ResearchEventStream {
    rx: tokio::sync::mpsc::Receiver<ResearchEvent>,
}

struct Parent {
    id: String,
    model: String,
    stream: AgentStream,
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
