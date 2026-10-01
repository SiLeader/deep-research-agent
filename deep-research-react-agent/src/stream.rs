use crate::event::AgentEvent;
use futures::Stream;
use std::pin::Pin;

pub struct AgentStream {
    stream: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
}

impl AgentStream {
    pub(crate) fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = AgentEvent> + Send + 'static,
    {
        Self {
            stream: Box::pin(stream),
        }
    }
}

impl Stream for AgentStream {
    type Item = AgentEvent;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.get_mut().stream.as_mut().poll_next(cx)
    }
}
