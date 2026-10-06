use crate::DeepResearchOrchestrator;
use crate::plan::SubmitPlanOutput;
use crate::research::ResearchStepOutput;
use deep_research_react_agent::event::AgentEvent;
use deep_research_react_agent::stream::AgentStream;
use futures::Stream;
use std::pin::Pin;
use tracing::error;

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
        todo!()
    }
}

pub struct ResearchEventStream {
    this: DeepResearchOrchestrator,
    tx: tokio::sync::mpsc::Sender<ResearchEvent>,
    rx: tokio::sync::mpsc::Receiver<ResearchEvent>,
    jobs: Vec<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

struct Parent {
    id: String,
    model: String,
    stream: AgentStream,
}

impl ResearchEventStream {
    pub(crate) fn new(this: DeepResearchOrchestrator) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        Self {
            this,
            tx,
            rx,
            jobs: Vec::new(),
        }
    }

    pub(crate) fn spawn(&mut self, plan: SubmitPlanOutput, max_loop_count: usize) {
        for step_plan in plan.research_plans {
            let tx = self.tx.clone();
            let this = self.this.clone();
            let handle = tokio::spawn(this.research_step_with_event(
                step_plan,
                max_loop_count,
                async move |event| {
                    if let Err(e) = tx.send(event).await {
                        error!("Failed to send research event: {}", e);
                    }
                },
            ));
            self.jobs.push(handle);
        }
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
