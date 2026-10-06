use crate::ReActAgent;
use crate::event::AgentEvent;
use crate::stream::AgentStream;
use async_stream::stream;
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use std::future::Future;

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
                let finished = matches!(event, AgentEvent::Finish(_));
                yield event;
                if finished {
                    break;
                }
            }
        })
    }

    pub async fn run_with_event<O: DeserializeOwned, F, Fut>(
        &self,
        prompt: String,
        event_callback: F,
    ) -> anyhow::Result<O>
    where
        F: Fn(AgentEvent) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        let mut stream = self.run_stream(prompt);
        let mut last_event = None;
        while let Some(event) = stream.next().await {
            last_event = Some(event.clone());
            event_callback(event).await;
        }
        let Some(AgentEvent::Finish(output)) = last_event else {
            anyhow::bail!("Researcher agent did not produce any events");
        };
        let resource_out: O = serde_json::from_value(output.fn_arguments)?;
        Ok(resource_out)
    }
}
