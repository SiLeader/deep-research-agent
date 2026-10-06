use crate::ReActAgent;
use crate::event::AgentEvent;
use genai::chat::ToolCall;
use serde::de::DeserializeOwned;

impl ReActAgent {
    pub async fn run(&self, message: String) -> anyhow::Result<ToolCall> {
        let mut messages = self.create_initial_messages(message);

        for _ in 0..self.max_llm_calls {
            let event = self.run_llm_single(&mut messages).await;

            if let AgentEvent::Finish(tool_call) = &event {
                return Ok(tool_call.clone());
            }
            if let AgentEvent::Error(error) = &event {
                anyhow::bail!("{}", error.error);
            }
            let Some(tool_calls) = event.unwrap_tool_calls() else {
                anyhow::bail!("Finished without finish marker tools invocation");
            };

            if let AgentEvent::Finish(tool_call) =
                self.run_tools_single(&mut messages, tool_calls).await
            {
                return Ok(tool_call);
            }
        }
        anyhow::bail!("Agent exceeded max_llm_calls ({})", self.max_llm_calls)
    }

    pub async fn get_output<T>(&self, message: String) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        Ok(serde_json::from_value(
            self.run(message).await?.fn_arguments,
        )?)
    }
}
