pub mod plan;
pub mod research;
mod stream;
mod synthesizer;

use crate::plan::DeepResearchPlan;
use crate::research::Researcher;
use crate::stream::{ResearchEvent, ResearchEventStream};
use deep_research_react_agent::ReActAgent;
use tracing::error;

#[derive(Clone)]
pub struct DeepResearchOrchestrator {
    planner_agent: ReActAgent,
    researcher: Researcher,
    synthesizer_agent: ReActAgent,
}

impl DeepResearchOrchestrator {
    pub fn new(
        planner_agent: ReActAgent,
        researcher_agent: ReActAgent,
        gap_judger_agent: ReActAgent,
        synthesizer_agent: ReActAgent,
    ) -> Self {
        let mut this = Self {
            planner_agent,
            researcher: Researcher::new(researcher_agent, gap_judger_agent),
            synthesizer_agent,
        };

        this.researcher.prepare_agent();
        this.prepare_plan_agent();
        this.prepare_synthesizer_agent();

        this
    }

    pub fn run_deep_research(self, plan: DeepResearchPlan) -> ResearchEventStream {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let stream = ResearchEventStream::new(rx);
        self.spawn(tx, plan, 10);
        stream
    }

    pub(crate) fn spawn(
        self,
        tx: tokio::sync::mpsc::Sender<ResearchEvent>,
        plan: DeepResearchPlan,
        max_loop_count: usize,
    ) {
        tokio::spawn(async move {
            let mut research_jobs = Vec::with_capacity(plan.research_plans.len());
            for step_plan in plan.research_plans {
                let tx = tx.clone();
                let this = self.researcher.clone();
                let handle = tokio::spawn(async move {
                    this.research_step_with_event(step_plan, max_loop_count, move |event| {
                        let tx = tx.clone();
                        async move {
                            if let Err(e) = tx.send(event).await {
                                error!("Failed to send research event: {}", e);
                            }
                        }
                    })
                    .await
                });
                research_jobs.push(handle);
            }
            let results = futures::future::join_all(research_jobs).await;

            self.synthesize(
                plan.report_plan,
                results
                    .into_iter()
                    .filter_map(|r| r.ok().map(|a| a.ok()).flatten())
                    .collect(),
                move |event| {
                    let tx = tx.clone();
                    async move {
                        if let Err(e) = tx.send(event).await {
                            error!("Failed to send research event: {}", e);
                        }
                    }
                },
            )
            .await
        });
    }
}
