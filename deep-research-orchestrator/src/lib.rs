pub mod plan;
pub mod research;
pub mod stream;
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
        let task = self.spawn(tx, plan, 10);
        ResearchEventStream::new(rx, task)
    }

    pub(crate) fn spawn(
        self,
        tx: tokio::sync::mpsc::Sender<ResearchEvent>,
        plan: DeepResearchPlan,
        max_loop_count: usize,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut research_jobs = tokio::task::JoinSet::new();
            for (index, step_plan) in plan.research_plans.into_iter().enumerate() {
                let tx = tx.clone();
                let this = self.researcher.clone();
                research_jobs.spawn(async move {
                    let result = this
                        .research_step_with_event(step_plan, max_loop_count, move |event| {
                            let tx = tx.clone();
                            async move {
                                if let Err(e) = tx.send(event).await {
                                    error!("Failed to send research event: {}", e);
                                }
                            }
                        })
                        .await;
                    result.map(|output| (index, output))
                });
            }
            let mut results = Vec::new();
            while let Some(result) = research_jobs.join_next().await {
                match result {
                    Ok(Ok(output)) => results.push(output),
                    result => {
                        let failure = match result {
                            Ok(Err(e)) => e.to_string(),
                            Err(e) => e.to_string(),
                            Ok(Ok(_)) => unreachable!(),
                        };
                        error!("Research failed: {}", failure);
                        let _ = tx
                            .send(ResearchEvent::failed(
                                self.researcher.model().to_string(),
                                failure,
                            ))
                            .await;
                        return;
                    }
                }
            }
            results.sort_by_key(|(index, _)| *index);

            let result = self
                .synthesize(
                    plan.report_plan,
                    results.into_iter().map(|(_, output)| output).collect(),
                    |event| {
                        let tx = tx.clone();
                        async move {
                            if let Err(e) = tx.send(event).await {
                                error!("Failed to send research event: {}", e);
                            }
                        }
                    },
                )
                .await;
            if let Err(e) = result {
                error!("Synthesis failed: {}", e);
                let _ = tx
                    .send(ResearchEvent::failed(
                        self.synthesizer_agent.model().to_string(),
                        e.to_string(),
                    ))
                    .await;
            }
        })
    }
}
