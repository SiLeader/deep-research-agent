use crate::event::{AgentEvent, ToolCallEvent};
use crate::{ReActAgent, event};
use genai::chat::{ChatMessage, MessageContent, ToolCall};

impl ReActAgent {
    pub(crate) fn create_initial_messages(&self, message: String) -> Vec<ChatMessage> {
        vec![
            ChatMessage::system(self.system_prompt.clone()),
            ChatMessage::user(message),
        ]
    }

    pub(crate) async fn run_llm_single(&self, messages: &mut Vec<ChatMessage>) -> AgentEvent {
        match self.oneshot.run(messages.clone()).await {
            Ok(response) => {
                let content = response.content.clone();
                messages.push(ChatMessage::assistant(content.clone()));

                let message = response.first_text().map(ToString::to_string);
                let tool_calls = response.into_tool_calls();
                if tool_calls.is_empty() {
                    AgentEvent::Message(event::MessageEvent {
                        message: message.clone().unwrap_or_default(),
                    })
                } else {
                    AgentEvent::ToolCall(ToolCallEvent {
                        message,
                        tool_calls,
                    })
                }
            }
            Err(e) => AgentEvent::Error(event::ErrorEvent {
                error: e.to_string(),
            }),
        }
    }

    pub(crate) async fn run_tools_single(
        &self,
        messages: &mut Vec<ChatMessage>,
        tool_calls: Vec<ToolCall>,
    ) -> AgentEvent {
        let mut tool_jobs = Vec::new();

        if let Some(tc) = tool_calls
            .iter()
            .find(|tool_call| self.stop_tool_names.contains(&tool_call.fn_name))
        {
            return AgentEvent::Finish(tc.clone());
        }

        for tool_call in tool_calls {
            tool_jobs.push(async move {
                let value = self
                    .tools
                    .call(&tool_call.fn_name, tool_call.fn_arguments)
                    .await?;
                Ok(match value {
                    None => None,
                    Some(value) => Some(genai::chat::ToolResponse {
                        call_id: tool_call.call_id,
                        fn_name: Some(tool_call.fn_name),
                        content: value.to_string(),
                    }),
                })
            });
        }

        match futures::future::join_all(tool_jobs)
            .await
            .into_iter()
            .collect::<anyhow::Result<Vec<_>>>()
        {
            Ok(results) => {
                let tool_responses = results.into_iter().flatten().collect::<Vec<_>>();
                messages.push(ChatMessage::tool(MessageContent::from_tool_responses(
                    tool_responses.clone(),
                )));

                AgentEvent::ToolResponse(event::ToolResponseEvent { tool_responses })
            }
            Err(e) => AgentEvent::Error(event::ErrorEvent {
                error: e.to_string(),
            }),
        }
    }
}
