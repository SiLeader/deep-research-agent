use crate::DeepResearchOrchestrator;
use crate::plan::ResearchStepPlan;
use crate::stream::ResearchEventStream;
use deep_research_react_agent::stream::AgentStream;
use deep_research_tools::tools::marker::MarkerTool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct ResearchStepOutput {
    pub(crate) research_step_result: String,
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

    pub async fn research(&self, plan: ResearchStepPlan) -> anyhow::Result<ResearchStepOutput> {
        let resource_out = self
            .researcher_agent
            .get_output::<ResearchStepOutput>(plan.to_prompt())
            .await?;
        let gap_out = self
            .gap_judger_agent
            .get_output::<GapJudgeOutput>(resource_out.to_gap_judger_prompt())
            .await?;
        if gap_out.approved {
            return Ok(resource_out);
        }

        todo!()
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
    fn to_prompt(&self) -> String {
        format!(
            "You are a research agent. Your goal is to conduct research on the following topic: {}. Please provide your findings in a clear and concise manner.",
            self.goal
        )
    }
}
