use crate::ReActAgent;
use crate::stream::AgentStream;
use async_stream::stream;
use genai::chat::ChatMessage;

impl ReActAgent {
    pub fn run_stream(&self, message: String) -> AgentStream {
        self.clone().run_stream_moved(message)
    }

    fn run_stream_moved(self, message: String) -> AgentStream {
        AgentStream::new(stream! {
            let mut messages = self.create_initial_messages(message);

            loop {
                let event = self.run_llm_single(&mut messages).await;
                yield event.clone();

                let Some(tool_calls) = event.unwrap_tool_calls() else {
                    break;
                };

                let event = self.run_tools_single(&mut messages, tool_calls).await;
                yield event;
            }
        })
    }
}
