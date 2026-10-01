pub mod semaphore;

use async_trait::async_trait;

#[async_trait]
pub trait AgentConcurrencyArbiter {
    async fn acquire(&self) -> anyhow::Result<ArbiterTabletGuard>;
}

pub struct ArbiterTabletGuard {
    tablet: Box<dyn ArbiterTablet>,
}

impl Drop for ArbiterTabletGuard {
    fn drop(&mut self) {
        self.tablet.release();
    }
}

pub trait ArbiterTablet: Send + Sync {
    fn release(&mut self);
}
