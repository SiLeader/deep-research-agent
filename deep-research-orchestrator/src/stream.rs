use deep_research_react_agent::event::AgentEvent;
use deep_research_react_agent::stream::AgentStream;
use futures::Stream;
use std::pin::Pin;
use tokio::sync::RwLock;

pub struct ResearchEvent {
    model: String,
    event: AgentEvent,
}

pub struct ResearchEventStream {
    stream: RwLock<Option<Parent>>,
}

struct Parent {
    model: String,
    stream: AgentStream,
}

impl ResearchEventStream {
    pub(crate) fn initial(model: String, stream: AgentStream) -> Self {
        Self {
            stream: RwLock::new(Some(Parent { model, stream })),
        }
    }

    pub(crate) fn switch_stream(&mut self, model: String, stream: AgentStream) {
        let mut lock = self.stream.blocking_write();
        *lock = Some(Parent { model, stream });
    }
}

impl Stream for ResearchEventStream {
    type Item = ResearchEvent;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if let Some(parent) = &mut *self.stream.blocking_read() {
            match Pin::new(&mut parent.stream).poll_next(cx) {
                std::task::Poll::Ready(Some(event)) => {
                    std::task::Poll::Ready(Some(ResearchEvent {
                        model: parent.model.clone(),
                        event,
                    }))
                }
                std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        } else {
            std::task::Poll::Ready(None)
        }
    }
}
