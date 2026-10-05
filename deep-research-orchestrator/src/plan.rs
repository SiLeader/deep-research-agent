use crate::DeepResearchOrchestrator;
use deep_research_react_agent::stream::AgentStream;
use deep_research_tools::tools::marker::MarkerTool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct SubmitPlanOutput {
    research_plans: Vec<ResearchStepPlan>,
    report_plan: ReportPlan,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
pub(crate) struct ResearchStepPlan {
    pub(crate) goal: String,
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct ReportPlan {
    goal: String,
}

impl DeepResearchOrchestrator {
    pub(crate) fn prepare_plan_agent(&mut self) {
        self.planner_agent
            .add_stop_tool(MarkerTool::<SubmitPlanOutput>::new(
            "submit".into(),
            Some(
                "The final output of the planning, containing the research plans and report plan."
                    .to_string(),
            ),
            Some(true),
            None,
        ));
    }

    pub async fn plan(&self, question: String) -> AgentStream {
        self.planner_agent.run_stream(question).await
    }
}
