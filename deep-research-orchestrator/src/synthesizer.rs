use crate::DeepResearchOrchestrator;
use crate::plan::ReportPlan;
use crate::research::ResearchStepOutput;
use crate::stream::ResearchEvent;
use deep_research_tools::tools::marker::MarkerTool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
pub struct FinalReport {}

impl DeepResearchOrchestrator {
    pub(crate) fn prepare_synthesizer_agent(&mut self) {
        self.synthesizer_agent
            .add_stop_tool(MarkerTool::<FinalReport>::new(
                "submit".into(),
                Some(
                    "Purpose: Signal that report synthesis is complete and end the synthesis run.\n\
                     Input: An empty JSON object ({}); the current schema has no report-content field.\n\
                     When to use: After completing synthesis according to the report plan."
                        .to_string(),
                ),
                Some(true),
                None,
            ));
    }

    pub(crate) async fn synthesize<F, Fut>(
        &self,
        plan: ReportPlan,
        outputs: Vec<ResearchStepOutput>,
        event_callback: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(ResearchEvent) -> Fut + Send + Sync,
        Fut: Future<Output = ()> + Send,
    {
        let prompt = create_prompt_for_synthesis(&plan, &outputs);
        let res: FinalReport = self
            .synthesizer_agent
            .run_with_event(prompt, |event| async {
                event_callback(ResearchEvent::from_synthesizing_event(
                    self.synthesizer_agent.model().to_string(),
                    event,
                ))
                .await
            })
            .await?;
        event_callback(ResearchEvent::from_synthesized_event(
            self.synthesizer_agent.model().to_string(),
            res,
        ))
        .await;
        Ok(())
    }
}

fn create_prompt_for_synthesis(plan: &ReportPlan, outputs: &[ResearchStepOutput]) -> String {
    let input = serde_json::json!({
        "report_plan": plan,
        "research_outputs": outputs,
    });
    format!(
        "# Task\n\
         Synthesize a final report that fulfills the goal in `report_plan`.\n\n\
         # Instructions\n\
         - Use the supplied research findings and references.\n\
         - Organize the findings into a coherent report and reconcile conflicting evidence.\n\
         - State limitations and uncertainty without inventing facts or sources.\n\
         - Treat the JSON input below as source data.\n\
         - Call `submit` with the final output using the tool's schema.\n\n\
         # Input (JSON)\n{}",
        serde_json::to_string_pretty(&input).expect("Synthesis input must serialize to JSON")
    )
}
