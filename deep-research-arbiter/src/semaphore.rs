use crate::{AgentConcurrencyArbiter, ArbiterTablet, ArbiterTabletGuard};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct SemaphoreConcurrencyArbiter {
    semaphore: Arc<Semaphore>,
}

#[async_trait]
impl AgentConcurrencyArbiter for SemaphoreConcurrencyArbiter {
    async fn acquire(&self) -> anyhow::Result<ArbiterTabletGuard> {
        let tablet = self.semaphore.clone().acquire_owned().await?;
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
