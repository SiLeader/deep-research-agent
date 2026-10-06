use crate::DeepResearchOrchestrator;
use crate::plan::{DeepResearchPlan, ReportPlan};
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
                Some("The final output of the synthesis, containing the final report.".to_string()),
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
    format!(
        "Given the report plan: {:?} and the research step outputs: {:?}, please synthesize a final report.",
        plan, outputs
    )
}
