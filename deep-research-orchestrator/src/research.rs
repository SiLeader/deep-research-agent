use crate::DeepResearchOrchestrator;
use crate::plan::{ResearchStepPlan, SubmitPlanOutput};
use crate::stream::{ResearchEvent, ResearchEventStream, ResearchPhase};
use deep_research_react_agent::event::AgentEvent;
use deep_research_react_agent::stream::AgentStream;
use deep_research_tools::tools::marker::MarkerTool;
use futures::StreamExt;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct ResearchStepOutput {
    pub(crate) research_step_result: String,
    pub(crate) references: Vec<ResearchReference>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub struct ResearchReference {
    pub(crate) source: String,
    pub(crate) content: String,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct GapJudgeOutput {
    approved: bool,
}

impl DeepResearchOrchestrator {
    pub(crate) fn prepare_research_agent(&mut self) {
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

    pub fn research_stream(
        &self,
        plan: SubmitPlanOutput,
        max_loop_count: usize,
    ) -> ResearchEventStream {
        let (tx, rx) = tokio::sync::mpsc::channel(100);

        let stream = ResearchEventStream::new(rx);
        for step_plan in plan.research_plans {
            let fut = self.research_step(step_plan, max_loop_count, |event| tx.send(event));
            tokio::spawn(fut);
        }
        stream
    }

    pub(crate) async fn research_step_with_event<F>(
        &self,
        plan: ResearchStepPlan,
        max_loop_count: usize,
        event_callback: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(ResearchEvent),
    {
        let mut prev_gap: Option<GapJudgeOutput> = None;
        for _ in 0..max_loop_count {
            let mut last_event = None;
            let research_out: ResearchStepOutput = self
                .researcher_agent
                .run_with_event(plan.to_prompt(prev_gap), |event| {
                    last_event = Some(event.clone());
                    event_callback(ResearchEvent::from_research_event(
                        self.researcher_agent.model().to_string(),
                        event,
                    ));
                })
                .await?;

            let gap_out: GapJudgeOutput = self
                .gap_judger_agent
                .run_with_event(research_out.to_gap_judger_prompt(), |event| {
                    event_callback(ResearchEvent::from_gap_judging_event(
                        self.gap_judger_agent.model().to_string(),
                        event,
                    ));
                })
                .await?;

            if gap_out.approved {
                event_callback(ResearchEvent::from_completed_event(
                    self.researcher_agent.model().to_string(),
                    research_out,
                ));
                return Ok(());
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
