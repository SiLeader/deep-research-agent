use crate::ReActAgent;
use crate::event::AgentEvent;
use genai::chat::ToolCall;
use serde::de::DeserializeOwned;

impl ReActAgent {
    pub async fn run(&self, message: String) -> anyhow::Result<ToolCall> {
        let mut messages = self.create_initial_messages(message);

        loop {
            let event = self.run_llm_single(&mut messages).await;

            if let AgentEvent::Finish(tool_call) = &event {
                return Ok(tool_call.clone());
            }
            let Some(tool_calls) = event.unwrap_tool_calls() else {
                anyhow::bail!("Finished without finish marker tools invocation");
            };

            self.run_tools_single(&mut messages, tool_calls).await;
        }
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
