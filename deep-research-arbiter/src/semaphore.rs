use crate::{AgentConcurrencyArbiter, ArbiterTablet, ArbiterTabletGuard};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct SemaphoreConcurrencyArbiter {
    semaphore: HashMap<String, Arc<Semaphore>>,
}

impl SemaphoreConcurrencyArbiter {
    pub fn new(model_limits: HashMap<String, usize>) -> Self {
        let semaphore = model_limits
            .into_iter()
            .map(|(model, limit)| (model, Arc::new(Semaphore::new(limit))))
            .collect();
        Self { semaphore }
    }
}

#[async_trait]
impl AgentConcurrencyArbiter for SemaphoreConcurrencyArbiter {
    async fn acquire(&self, model: &str) -> anyhow::Result<ArbiterTabletGuard> {
        let semaphore = self
            .semaphore
            .get(model)
            .ok_or_else(|| anyhow::anyhow!("Semaphore not found for model: {}", model))?;
        let tablet = semaphore.clone().acquire_owned().await?;
        Ok(ArbiterTabletGuard {
            tablet: Box::new(SemaphoreTablet {
                permit: Some(tablet),
            }),
        })
    }
}

pub struct SemaphoreTablet {
    permit: Option<OwnedSemaphorePermit>,
}

impl ArbiterTablet for SemaphoreTablet {
    fn release(&mut self) {
        if let Some(permit) = self.permit.take() {
            drop(permit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    #[tokio::test]
    async fn unknown_model_returns_error() {
        let arbiter = SemaphoreConcurrencyArbiter::new(HashMap::new());
        let error = arbiter.acquire("missing").await.err().unwrap();
        assert_eq!(error.to_string(), "Semaphore not found for model: missing");
    }

    #[tokio::test]
    async fn enforces_limit_and_releases_permit_when_guard_is_dropped() {
        let arbiter = SemaphoreConcurrencyArbiter::new(HashMap::from([("model".into(), 2)]));
        let first = arbiter.acquire("model").await.unwrap();
        let second = arbiter.acquire("model").await.unwrap();
        let mut waiting = pin!(arbiter.acquire("model"));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(waiting.as_mut().poll(&mut cx).is_pending());
        drop(first);
        let Poll::Ready(Ok(third)) = waiting.as_mut().poll(&mut cx) else {
            panic!("dropping a guard must unblock the waiting acquisition");
        };
        drop(second);
        drop(third);
        assert_eq!(arbiter.semaphore["model"].available_permits(), 2);
    }

    #[tokio::test]
    async fn model_limits_are_independent() {
        let arbiter = SemaphoreConcurrencyArbiter::new(HashMap::from([
            ("first".into(), 1),
            ("second".into(), 1),
        ]));
        let _first = arbiter.acquire("first").await.unwrap();
        let mut waiting = pin!(arbiter.acquire("first"));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(waiting.as_mut().poll(&mut cx).is_pending());
        let mut independent = pin!(arbiter.acquire("second"));
        assert!(matches!(
            independent.as_mut().poll(&mut cx),
            Poll::Ready(Ok(_))
        ));
    }

    #[tokio::test]
    async fn cancelling_waiter_does_not_leak_permits() {
        let arbiter = SemaphoreConcurrencyArbiter::new(HashMap::from([("model".into(), 1)]));
        let guard = arbiter.acquire("model").await.unwrap();
        let mut waiting = Box::pin(arbiter.acquire("model"));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(waiting.as_mut().poll(&mut cx).is_pending());
        drop(waiting);
        drop(guard);
        let mut next = pin!(arbiter.acquire("model"));
        assert!(matches!(next.as_mut().poll(&mut cx), Poll::Ready(Ok(_))));
        assert_eq!(arbiter.semaphore["model"].available_permits(), 1);
    }
}
