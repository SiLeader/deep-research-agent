mod handlers;

use actix_web::{App, HttpServer};

pub struct DeepResearchServer {}

impl DeepResearchServer {
    pub fn new() -> Self {
        DeepResearchServer {}
    }

    pub async fn run(self, addr: &str) -> anyhow::Result<()> {
        HttpServer::new(|| App::new()).bind(addr)?.run().await?;
        Ok(())
    }
}
