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
