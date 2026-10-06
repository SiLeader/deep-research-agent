use actix_web::post;
use actix_web::web::Data;
use deep_research_orchestrator::DeepResearchOrchestrator;
use deep_research_orchestrator::plan::DeepResearchPlan;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct ResearchCreateRequest {
    plan: DeepResearchPlan,
}

#[post("/v1/deep/research")]
pub(super) async fn research_create(
    orchestrator: Data<DeepResearchOrchestrator>,
) -> actix_web::Result<actix_web::HttpResponse> {
    Ok(actix_web::HttpResponse::Ok().json(serde_json::json!({
        "message": "Research creation endpoint is under construction."
    })))
}
