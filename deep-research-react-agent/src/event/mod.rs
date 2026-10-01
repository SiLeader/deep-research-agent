use genai::chat::{ToolCall, ToolResponse};

#[derive(Debug, Clone)]
pub enum AgentEvent {
    ToolCall(ToolCallEvent),
    ToolResponse(ToolResponseEvent),
    Message(MessageEvent),
    Finish(ToolCall),
    Error(ErrorEvent),
}

#[derive(Debug, Clone)]
pub struct ToolCallEvent {
    pub message: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone)]
pub struct ToolResponseEvent {
    pub tool_responses: Vec<ToolResponse>,
}

#[derive(Debug, Clone)]
pub struct MessageEvent {
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ErrorEvent {
    pub error: String,
}

impl AgentEvent {
    pub(crate) fn unwrap_tool_calls(self) -> Option<Vec<ToolCall>> {
        match self {
            AgentEvent::ToolCall(tc) => Some(tc.tool_calls),
            _ => None,
        }
    }
}
