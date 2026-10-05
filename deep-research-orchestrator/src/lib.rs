pub mod plan;
pub mod research;
mod stream;

use deep_research_react_agent::ReActAgent;

pub struct DeepResearchOrchestrator {
    planner_agent: ReActAgent,
    researcher_agent: ReActAgent,
    gap_judger_agent: ReActAgent,
    synthesizer_agent: ReActAgent,
}

impl DeepResearchOrchestrator {
    pub fn new(
        planner_agent: ReActAgent,
        researcher_agent: ReActAgent,
        gap_judger_agent: ReActAgent,
        synthesizer_agent: ReActAgent,
    ) -> Self {
        Self {
            planner_agent,
            researcher_agent,
            gap_judger_agent,
            synthesizer_agent,
        }
    }
}
