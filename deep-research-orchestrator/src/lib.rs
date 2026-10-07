pub mod plan;
pub mod research;
pub mod stream;
mod synthesizer;

use crate::plan::DeepResearchPlan;
use crate::research::{CompletedResearch, Researcher};
use crate::stream::{ResearchEvent, ResearchEventStream};
use deep_research_react_agent::ReActAgent;
use deep_research_runner::{CallBudget, with_call_budget};
use std::time::Duration;
use tracing::error;

/// Bounds the work done for one research request.
#[derive(Debug, Clone)]
pub struct ResearchLimits {
    /// Research/review cycles per step before the step fails.
    pub max_research_loops: usize,
    /// Deadline for the whole request, including synthesis.
    pub timeout: Option<Duration>,
    /// LLM requests shared by all agents, including Explorer and retries.
    pub max_total_llm_calls: Option<usize>,
}

impl Default for ResearchLimits {
    fn default() -> Self {
        Self {
            max_research_loops: 10,
            timeout: None,
            max_total_llm_calls: None,
        }
    }
}

impl ResearchLimits {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_research_loops > 0,
            "max_research_loops must be positive"
        );
        anyhow::ensure!(
            self.timeout.is_none_or(|timeout| !timeout.is_zero()),
            "research timeout must be positive"
        );
        anyhow::ensure!(
            self.max_total_llm_calls.is_none_or(|calls| calls > 0),
            "max_total_llm_calls must be positive"
        );
        Ok(())
    }
}

#[derive(Clone)]
pub struct DeepResearchOrchestrator {
    planner_agent: ReActAgent,
    researcher: Researcher,
    synthesizer_agent: ReActAgent,
    limits: ResearchLimits,
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
            limits: ResearchLimits::default(),
        };

        this.researcher.prepare_agent();
        this.prepare_plan_agent();
        this.prepare_synthesizer_agent();

        this
    }

    pub fn with_limits(mut self, limits: ResearchLimits) -> anyhow::Result<Self> {
        limits.validate()?;
        self.limits = limits;
        Ok(self)
    }

    pub fn run_deep_research(self, plan: DeepResearchPlan) -> ResearchEventStream {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let task = self.spawn(tx, plan);
        ResearchEventStream::new(rx, task)
    }

    pub(crate) fn spawn(
        self,
        tx: tokio::sync::mpsc::Sender<ResearchEvent>,
        plan: DeepResearchPlan,
    ) -> tokio::task::JoinHandle<()> {
        let budget = self.limits.max_total_llm_calls.map(CallBudget::new);
        let timeout = self.limits.timeout;
        let model = self.researcher.model().to_string();
        tokio::spawn(async move {
            let run = self.run(tx.clone(), plan, budget);
            let Some(timeout) = timeout else {
                return run.await;
            };
            // Dropping the run on timeout aborts every research job it owns.
            if tokio::time::timeout(timeout, run).await.is_err() {
                error!("Research timed out after {:?}", timeout);
                let _ = tx
                    .send(ResearchEvent::failed(
                        model,
                        None,
                        format!(
                            "Research exceeded the time limit of {} seconds",
                            timeout.as_secs()
                        ),
                    ))
                    .await;
            }
        })
    }

    async fn run(
        self,
        tx: tokio::sync::mpsc::Sender<ResearchEvent>,
        plan: DeepResearchPlan,
        budget: Option<std::sync::Arc<CallBudget>>,
    ) {
        if let Err(e) = plan.validate() {
            let _ = tx
                .send(ResearchEvent::failed(
                    self.researcher.model().to_string(),
                    None,
                    e.to_string(),
                ))
                .await;
            return;
        }
        let max_loop_count = self.limits.max_research_loops;
        let mut research_jobs = tokio::task::JoinSet::new();
        for (index, step_plan) in plan.research_plans.into_iter().enumerate() {
            let tx = tx.clone();
            let this = self.researcher.clone();
            let job = async move {
                let result = this
                    .research_step_with_event(
                        index,
                        step_plan.clone(),
                        max_loop_count,
                        move |event| {
                            let tx = tx.clone();
                            async move {
                                if let Err(e) = tx.send(event).await {
                                    error!("Failed to send research event: {}", e);
                                }
                            }
                        },
                    )
                    .await;
                (
                    index,
                    result.map(|output| CompletedResearch {
                        research_plan: step_plan,
                        research_output: output,
                    }),
                )
            };
            match budget.clone() {
                Some(budget) => research_jobs.spawn(with_call_budget(budget, job)),
                None => research_jobs.spawn(job),
            };
        }
        let mut results = Vec::new();
        while let Some(result) = research_jobs.join_next().await {
            let (step, failure) = match result {
                Ok((index, Ok(output))) => {
                    results.push((index, output));
                    continue;
                }
                Ok((index, Err(e))) => (Some(index), e.to_string()),
                Err(e) => (None, e.to_string()),
            };
            error!(step, "Research failed: {}", failure);
            let _ = tx
                .send(ResearchEvent::failed(
                    self.researcher.model().to_string(),
                    step,
                    failure,
                ))
                .await;
            return;
        }
        results.sort_by_key(|(index, _)| *index);

        let synthesis = self.synthesize(
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
        );
        let result = match budget {
            Some(budget) => with_call_budget(budget, synthesis).await,
            None => synthesis.await,
        };
        if let Err(e) = result {
            error!("Synthesis failed: {}", e);
            let _ = tx
                .send(ResearchEvent::failed(
                    self.synthesizer_agent.model().to_string(),
                    None,
                    e.to_string(),
                ))
                .await;
        }
    }
}

#[cfg(test)]
mod tests;
