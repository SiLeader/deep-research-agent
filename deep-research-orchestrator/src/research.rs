use crate::DeepResearchOrchestrator;
use crate::plan::{ResearchStepPlan, SubmitPlanOutput};
use crate::stream::ResearchEventStream;
use deep_research_react_agent::stream::AgentStream;
use deep_research_tools::tools::marker::MarkerTool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct ResearchStepOutput {
    pub(crate) research_step_result: String,
    pub(crate) references: Vec<ResearchReference>,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct ResearchReference {
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
        let mut results = Vec::new();
        for step_plan in plan.research_plans {
            let step_result = self.research_step(step_plan, max_loop_count).await?;
            results.push(step_result);
        }
        Ok(results)
    }

    pub(crate) async fn research_step(
        &self,
        plan: ResearchStepPlan,
        max_loop_count: usize,
    ) -> anyhow::Result<ResearchStepOutput> {
        let mut prev_gap: Option<GapJudgeOutput> = None;
        for _ in 0..max_loop_count {
            let resource_out = self
                .researcher_agent
                .get_output::<ResearchStepOutput>(plan.to_prompt(prev_gap))
                .await?;
            let gap_out = self
                .gap_judger_agent
                .get_output::<GapJudgeOutput>(resource_out.to_gap_judger_prompt())
                .await?;

            if gap_out.approved {
                return Ok(resource_out);
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
