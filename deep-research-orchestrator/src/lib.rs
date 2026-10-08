pub mod plan;
pub mod research;
pub mod stream;
mod synthesizer;

use crate::plan::DeepResearchPlan;
use crate::research::{CompletedResearch, Researcher};
use crate::stream::{ResearchEvent, ResearchEventStream};
use deep_research_react_agent::ReActAgent;
use deep_research_runner::{CallBudget, with_call_budget};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::error;

/// Bounds the work done for one research request.
#[derive(Debug, Clone)]
pub struct ResearchLimits {
    /// Research/review cycles per step before the step fails.
    pub max_research_loops: usize,
    /// Deadline for the whole request, including synthesis. Also bounds planning.
    pub timeout: Option<Duration>,
    /// LLM requests shared by all agents, including Explorer and retries.
    /// Also bounds each planning request.
    pub max_total_llm_calls: Option<usize>,
    /// Planning and research requests running at once; further requests fail with [`Busy`].
    pub max_concurrent_requests: Option<usize>,
}

impl Default for ResearchLimits {
    fn default() -> Self {
        Self {
            max_research_loops: 10,
            timeout: None,
            max_total_llm_calls: None,
            max_concurrent_requests: None,
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
        anyhow::ensure!(
            self.max_concurrent_requests
                .is_none_or(|requests| (1..=Semaphore::MAX_PERMITS).contains(&requests)),
            "max_concurrent_requests must be positive"
        );
        Ok(())
    }
}

/// The concurrent request limit is reached; retry later.
#[derive(Debug)]
pub struct Busy;

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("too many concurrent requests")
    }
}

impl std::error::Error for Busy {}

#[derive(Clone)]
pub struct DeepResearchOrchestrator {
    planner_agent: ReActAgent,
    researcher: Researcher,
    synthesizer_agent: ReActAgent,
    limits: ResearchLimits,
    // Shared by clones so the limit applies across all requests.
    requests: Option<Arc<Semaphore>>,
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
            requests: None,
        };

        this.researcher.prepare_agent();
        this.prepare_plan_agent();
        this.prepare_synthesizer_agent();

        this
    }

    pub fn with_limits(mut self, limits: ResearchLimits) -> anyhow::Result<Self> {
        limits.validate()?;
        self.requests = limits
            .max_concurrent_requests
            .map(|requests| Arc::new(Semaphore::new(requests)));
        self.limits = limits;
        Ok(self)
    }

    fn reserve(&self) -> Result<Option<OwnedSemaphorePermit>, Busy> {
        self.requests
            .clone()
            .map(|requests| requests.try_acquire_owned().map_err(|_| Busy))
            .transpose()
    }

    /// Apply the request slot, call budget, and deadline to a planning request.
    pub(crate) async fn limited<T>(
        &self,
        name: &str,
        future: impl Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<T> {
        let _permit = self.reserve()?;
        let budget = self.limits.max_total_llm_calls.map(CallBudget::new);
        let future = async move {
            match budget {
                Some(budget) => with_call_budget(budget, future).await,
                None => future.await,
            }
        };
        match self.limits.timeout {
            Some(timeout) => tokio::time::timeout(timeout, future).await.map_err(|_| {
                anyhow::anyhow!(
                    "{name} exceeded the time limit of {} seconds",
                    timeout.as_secs()
                )
            })?,
            None => future.await,
        }
    }

    /// Start research in the background. Fails with [`Busy`] when the
    /// concurrent request limit is reached; the slot is held until the run ends.
    pub fn run_deep_research(self, plan: DeepResearchPlan) -> Result<ResearchEventStream, Busy> {
        let permit = self.reserve()?;
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let task = self.spawn(tx, plan, permit);
        Ok(ResearchEventStream::new(rx, task))
    }

    pub(crate) fn spawn(
        self,
        tx: tokio::sync::mpsc::Sender<ResearchEvent>,
        plan: DeepResearchPlan,
        permit: Option<OwnedSemaphorePermit>,
    ) -> tokio::task::JoinHandle<()> {
        let budget = self.limits.max_total_llm_calls.map(CallBudget::new);
        let timeout = self.limits.timeout;
        let model = self.researcher.model().to_string();
        tokio::spawn(async move {
            let _permit = permit;
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
                    format!("{e:#}"),
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
                Ok((index, Err(e))) => (Some(index), format!("{e:#}")),
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
            error!("Synthesis failed: {:#}", e);
            let _ = tx
                .send(ResearchEvent::failed(
                    self.synthesizer_agent.model().to_string(),
                    None,
                    format!("{e:#}"),
                ))
                .await;
        }
    }
}

#[cfg(test)]
mod tests;
