mod handlers;

use actix_web::web::Data;
use actix_web::{App, HttpServer};
use deep_research_orchestrator::DeepResearchOrchestrator;
use tracing_actix_web::TracingLogger;

pub struct DeepResearchServer {
    orchestrator: Data<DeepResearchOrchestrator>,
}

impl DeepResearchServer {
    pub fn new(orchestrator: DeepResearchOrchestrator) -> Self {
        Self {
            orchestrator: Data::new(orchestrator),
        }
    }

    pub async fn run(self, addr: &str) -> anyhow::Result<()> {
        HttpServer::new(move || {
            App::new()
                .wrap(TracingLogger::default())
                .app_data(self.orchestrator.clone())
                .configure(handlers::configure)
        })
        .bind(addr)?
        .run()
        .await?;
        Ok(())
    }
}
