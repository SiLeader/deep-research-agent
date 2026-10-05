use deep_research_react_agent::event::AgentEvent;
use deep_research_react_agent::stream::AgentStream;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use tokio::sync::RwLock;

pub struct ResearchEvent {
    model: String,
    event: AgentEvent,
}

pub struct ResearchEventStream {
    stream: RwLock<HashMap<String, Parent>>,
}

struct Parent {
    id: String,
    model: String,
    stream: AgentStream,
}

impl ResearchEventStream {
    pub(crate) fn new() -> Self {
        Self {
            stream: RwLock::new(HashMap::new()),
        }
    }

    pub(crate) fn add_stream(&mut self, id: String, model: String, stream: AgentStream) {
        let mut lock = self.stream.blocking_write();
        lock.insert(id.clone(), Parent { id, model, stream });
    }
}

impl Stream for ResearchEventStream {
    type Item = ResearchEvent;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if let Some(parent) = &mut *self.stream.blocking_read() {
            for (_id, parent) in parent.iter_mut() {
                if let std::task::Poll::Ready(Some(event)) =
                    Pin::new(&mut parent.stream).poll_next(cx)
                {
                    return std::task::Poll::Ready(Some(ResearchEvent {
                        model: parent.model.clone(),
                        event,
                    }));
                }
            }
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(None)
        }
    }
}
