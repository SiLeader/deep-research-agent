use crate::event::{AgentEvent, ToolCallEvent};
use crate::{OutputValidator, ReActAgent, event};
use genai::chat::{ChatMessage, ChatRole, ContentPart, MessageContent, ToolCall, ToolResponse};

const ELIDED_TOOL_RESPONSE: &str =
    "[Earlier tool output omitted to fit the context limit. Call the tool again if you need it.]";

impl ReActAgent {
    pub(crate) fn create_initial_messages(&self, message: String) -> Vec<ChatMessage> {
        vec![
            ChatMessage::system(self.system_prompt.clone()),
            ChatMessage::user(message),
        ]
    }

    pub(crate) async fn run_llm_single(&self, messages: &mut Vec<ChatMessage>) -> AgentEvent {
        let tools = match self.tools.tools() {
            Ok(tools) => tools,
            Err(e) => {
                return AgentEvent::Error(event::ErrorEvent {
                    error: e.to_string(),
                });
            }
        };
        if let Some(limit) = self.max_tool_context_chars {
            compact_tool_responses(messages, limit);
        }
        match self.oneshot.run(messages.clone(), tools).await {
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
                error: format!("{e:#}"),
            }),
        }
    }

    pub(crate) async fn run_tools_single(
        &self,
        messages: &mut Vec<ChatMessage>,
        tool_calls: Vec<ToolCall>,
        validator: Option<&OutputValidator>,
    ) -> AgentEvent {
        // Only the first stop-tool call is evaluated. A rejected submission is
        // answered like any other tool call so the model can resubmit.
        let submission = tool_calls
            .iter()
            .position(|tool_call| self.stop_tool_names.contains(&tool_call.fn_name));
        let mut rejection = None;
        if let Some(index) = submission {
            match validator.map(|validate| validate(&tool_calls[index].fn_arguments)) {
                Some(Err(error)) => rejection = Some(error),
                _ => return AgentEvent::Finish(tool_calls[index].clone()),
            }
        }

        let tool_jobs = tool_calls.into_iter().enumerate().map(|(index, tool_call)| {
            let rejection = rejection.as_ref();
            async move {
                let content = if self.stop_tool_names.contains(&tool_call.fn_name) {
                    if Some(index) == submission {
                        format!(
                            "Submission rejected: {:#}\nCorrect the arguments and call `{}` again.",
                            rejection.expect("a submission reaching tool execution was rejected"),
                            tool_call.fn_name
                        )
                    } else {
                        "Ignored: only the first submission in a turn is evaluated.".to_string()
                    }
                } else {
                    match self
                        .tools
                        .call(&tool_call.fn_name, tool_call.fn_arguments)
                        .await
                    {
                        Ok(Some(value)) => value.to_string(),
                        Ok(None) => "Tool not found".to_string(),
                        Err(e) => format!("Tool call failed: {e:#}"),
                    }
                };
                ToolResponse {
                    call_id: tool_call.call_id,
                    fn_name: Some(tool_call.fn_name),
                    content,
                }
            }
        });

        let tool_responses = futures::future::join_all(tool_jobs).await;
        messages.push(ChatMessage::tool(MessageContent::from_tool_responses(
            tool_responses.clone(),
        )));

        AgentEvent::ToolResponse(event::ToolResponseEvent { tool_responses })
    }
}

/// Keep the total tool-response text within `limit` characters.
pub(crate) fn compact_tool_responses(messages: &mut [ChatMessage], limit: usize) {
    let size = |content: &str| content.chars().count();
    let mut total: usize = messages
        .iter()
        .flat_map(|message| message.content.tool_responses())
        .map(|response| size(&response.content))
        .sum();
    if total <= limit {
        return;
    }
    let latest = messages
        .iter()
        .rposition(|message| message.role == ChatRole::Tool);
    let placeholder = size(ELIDED_TOOL_RESPONSE);
    for (index, message) in messages.iter_mut().enumerate() {
        if Some(index) == latest {
            continue;
        }
        for part in message.content.iter_mut() {
            if let ContentPart::ToolResponse(response) = part {
                let current = size(&response.content);
                if current > placeholder {
                    total = total - current + placeholder;
                    response.content = ELIDED_TOOL_RESPONSE.to_string();
                    if total <= limit {
                        return;
                    }
                }
            }
        }
    }
    // The latest responses alone exceed the limit: share it among them.
    let Some(latest) = latest else { return };
    let message = &mut messages[latest];
    let count = message.content.tool_responses().len().max(1);
    let share = limit / count;
    for part in message.content.iter_mut() {
        if let ContentPart::ToolResponse(response) = part
            && size(&response.content) > share
        {
            let truncated: String = response.content.chars().take(share).collect();
            response.content = format!("{truncated}\n[Truncated to fit the context limit.]");
        }
    }
}
