use crate::plan::ResearchStepPlan;
use crate::stream::ResearchEvent;
use deep_research_react_agent::ReActAgent;
use deep_research_tools::tools::marker::MarkerTool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::future::Future;

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub struct ResearchStepOutput {
    pub(crate) research_step_result: String,
    pub(crate) references: Vec<ResearchReference>,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub struct ResearchReference {
    pub(crate) source: String,
    pub(crate) content: String,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
struct GapJudgeOutput {
    approved: bool,
}

#[derive(Clone)]
pub(crate) struct Researcher {
    researcher_agent: ReActAgent,
    gap_judger_agent: ReActAgent,
}

impl Researcher {
    pub(crate) fn model(&self) -> &str {
        self.researcher_agent.model()
    }

    pub(crate) fn new(researcher_agent: ReActAgent, gap_judger_agent: ReActAgent) -> Self {
        Self {
            researcher_agent,
            gap_judger_agent,
        }
    }

    pub(crate) fn prepare_agent(&mut self) {
        self.researcher_agent
            .add_stop_tool(MarkerTool::<ResearchStepOutput>::new(
                "submit".into(),
                Some(
                    "The final output of the research, indicating that the research is complete."
                        .to_string(),
                ),
                Some(true),
                None,
            ));
        self.gap_judger_agent.add_stop_tool(MarkerTool::<GapJudgeOutput>::new(
            "submit".into(),
            Some(
                "The final output of the gap judging, indicating that the gap analysis is complete."
                    .to_string(),
            ),
            Some(true),
            None,
        ));
    }

    pub(crate) async fn research_step_with_event<F, Fut>(
        &self,
        plan: ResearchStepPlan,
        max_loop_count: usize,
        event_callback: F,
    ) -> anyhow::Result<ResearchStepOutput>
    where
        F: Fn(ResearchEvent) -> Fut + Send + Sync,
        Fut: Future<Output = ()> + Send,
    {
        let mut prev_gap: Option<GapJudgeOutput> = None;
        for _ in 0..max_loop_count {
            let research_out: ResearchStepOutput = self
                .researcher_agent
                .run_with_event(plan.to_prompt(prev_gap), |event| {
                    event_callback(ResearchEvent::from_research_event(
                        self.researcher_agent.model().to_string(),
                        event,
                    ))
                })
                .await?;

            let gap_out: GapJudgeOutput = self
                .gap_judger_agent
                .run_with_event(research_out.to_gap_judger_prompt(), |event| {
                    event_callback(ResearchEvent::from_gap_judging_event(
                        self.gap_judger_agent.model().to_string(),
                        event,
                    ))
                })
                .await?;

            if gap_out.approved {
                event_callback(ResearchEvent::from_research_step_completed_event(
                    self.researcher_agent.model().to_string(),
                    research_out.clone(),
                ))
                .await;
                return Ok(research_out);
            }
            prev_gap = Some(gap_out);
        }
        anyhow::bail!("Max loop count reached without approval")
    }
}

impl ResearchStepOutput {
    fn to_gap_judger_prompt(&self) -> String {
        format!(
            "You are a research agent. Your goal is to conduct research on the following topic: {}. Please provide your findings in a clear and concise manner.",
            self.research_step_result
        )
    }
}

impl ResearchStepPlan {
    fn to_prompt(&self, gap: Option<GapJudgeOutput>) -> String {
        match gap {
            Some(gap) => format!(
                "You are a research agent. Your goal is to conduct research on the following topic: {}. The previous gap analysis result was: {}. Please provide your findings in a clear and concise manner.",
                self.goal, gap.approved
            ),
            None => format!(
                "You are a research agent. Your goal is to conduct research on the following topic: {}. Please provide your findings in a clear and concise manner.",
                self.goal
            ),
        }
    }
}
