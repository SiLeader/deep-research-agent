mod auth;
mod handlers;

use actix_web::web::Data;
use actix_web::{App, HttpServer};
use deep_research_orchestrator::DeepResearchOrchestrator;
use tracing_actix_web::TracingLogger;

pub struct DeepResearchServer {
    orchestrator: Data<DeepResearchOrchestrator>,
    api_key: Data<auth::ApiKey>,
}

impl DeepResearchServer {
    pub fn new(orchestrator: DeepResearchOrchestrator) -> Self {
        Self {
            orchestrator: Data::new(orchestrator),
            api_key: Data::new(auth::ApiKey(None)),
        }
    }

    /// Require `Authorization: Bearer <api_key>` on `/api/` routes.
    pub fn with_api_key(mut self, api_key: String) -> anyhow::Result<Self> {
        anyhow::ensure!(!api_key.trim().is_empty(), "API key must not be empty");
        self.api_key = Data::new(auth::ApiKey(Some(api_key)));
        Ok(self)
    }

    pub async fn run(self, addr: &str) -> anyhow::Result<()> {
        HttpServer::new(move || {
            App::new()
                .wrap(TracingLogger::default())
                .app_data(self.orchestrator.clone())
                .app_data(self.api_key.clone())
                .configure(handlers::configure)
        })
        .bind(addr)?
        .run()
        .await?;
        Ok(())
    }
}
