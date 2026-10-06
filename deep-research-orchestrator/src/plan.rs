use crate::DeepResearchOrchestrator;
use deep_research_react_agent::stream::AgentStream;
use deep_research_tools::tools::marker::MarkerTool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub struct DeepResearchPlan {
    pub(crate) research_plans: Vec<ResearchStepPlan>,
    pub(crate) report_plan: ReportPlan,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub(crate) struct ResearchStepPlan {
    pub(crate) goal: String,
}

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub(crate) struct ReportPlan {
    goal: String,
}

impl DeepResearchOrchestrator {
    pub(crate) fn prepare_plan_agent(&mut self) {
        self.planner_agent
            .add_stop_tool(MarkerTool::<DeepResearchPlan>::new(
            "submit".into(),
            Some(
                "The final output of the planning, containing the research plans and report plan."
                    .to_string(),
            ),
            Some(true),
            None,
        ));
    }

    pub async fn plan(&self, question: String) -> anyhow::Result<DeepResearchPlan> {
        self.planner_agent.get_output(question).await
    }

    pub async fn replan(
        &self,
        question: String,
        prev_plan: DeepResearchPlan,
    ) -> anyhow::Result<DeepResearchPlan> {
        let prompt = format!(
            "The previous plan was: {:?}. Please provide a new plan for the question: {}",
            prev_plan, question
        );
        self.planner_agent.get_output(prompt).await
    }
}
