use crate::event::AgentEvent;
use crate::stream::AgentStream;
use crate::{OutputValidator, ReActAgent};
use async_stream::stream;
use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::sync::Arc;

impl ReActAgent {
    pub fn run_stream(&self, message: String) -> AgentStream {
        self.clone().run_stream_moved(message, None)
    }

    /// Like [`Self::run_stream`], but a submission rejected by `validator` is
    /// returned to the model instead of finishing the run.
    pub fn run_stream_validated(&self, message: String, validator: OutputValidator) -> AgentStream {
        self.clone().run_stream_moved(message, Some(validator))
    }

    fn run_stream_moved(self, message: String, validator: Option<OutputValidator>) -> AgentStream {
        AgentStream::new(stream! {
            let mut messages = self.create_initial_messages(message);

            for _ in 0..self.max_llm_calls {
                let event = self.run_llm_single(&mut messages).await;
                yield event.clone();

                let Some(tool_calls) = event.unwrap_tool_calls() else {
                    return;
                };

                let event = self
                    .run_tools_single(&mut messages, tool_calls, validator.as_ref())
                    .await;
                let finished = matches!(event, AgentEvent::Finish(_));
                yield event;
                if finished {
                    return;
                }
            }
            yield AgentEvent::Error(crate::event::ErrorEvent {
                error: format!("Agent exceeded max_llm_calls ({})", self.max_llm_calls),
            });
        })
    }

    pub async fn run_with_event<O, F, Fut>(
        &self,
        prompt: String,
        event_callback: F,
    ) -> anyhow::Result<O>
    where
        O: DeserializeOwned + 'static,
        F: Fn(AgentEvent) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        self.run_with_event_validated(prompt, |_: &O| Ok(()), event_callback)
            .await
    }

    /// Run until a submission deserializes as `O` and passes `validate`.
    /// Rejected submissions are returned to the model with the error.
    pub async fn run_with_event_validated<O, V, F, Fut>(
        &self,
        prompt: String,
        validate: V,
        event_callback: F,
    ) -> anyhow::Result<O>
    where
        O: DeserializeOwned + 'static,
        V: Fn(&O) -> anyhow::Result<()> + Send + Sync + 'static,
        F: Fn(AgentEvent) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        let validator: OutputValidator = Arc::new(move |arguments: &serde_json::Value| {
            validate(&serde_json::from_value::<O>(arguments.clone())?)
        });
        let mut stream = self.run_stream_validated(prompt, validator);
        let mut last_event = None;
        while let Some(event) = stream.next().await {
            last_event = Some(event.clone());
            event_callback(event).await;
        }
        let Some(AgentEvent::Finish(output)) = last_event else {
            if let Some(AgentEvent::Error(error)) = last_event {
                anyhow::bail!("{}", error.error);
            }
            anyhow::bail!("Agent finished without a submit tool call");
        };
        Ok(serde_json::from_value(output.fn_arguments)?)
    }
}
