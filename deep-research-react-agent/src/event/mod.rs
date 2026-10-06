use genai::chat::{ToolCall, ToolResponse};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    ToolCall(ToolCallEvent),
    ToolResponse(ToolResponseEvent),
    Message(MessageEvent),
    Finish(ToolCall),
    Error(ErrorEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEvent {
    pub message: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponseEvent {
    pub tool_responses: Vec<ToolResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEvent {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
