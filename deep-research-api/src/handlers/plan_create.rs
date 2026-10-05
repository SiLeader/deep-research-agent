use actix_web::post;
use actix_web::web::{Data, Json};
use deep_research_orchestrator::DeepResearchOrchestrator;
use deep_research_orchestrator::plan::SubmitPlanOutput;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub(super) struct PlanCreateRequest {
    prompt: String,
    previous_plan: Option<SubmitPlanOutput>,
}

#[post("/v1/deep/research/plan")]
pub(super) async fn plan_create(
    orchestrator: Data<DeepResearchOrchestrator>,
    json: Json<PlanCreateRequest>,
) -> actix_web::Result<actix_web::HttpResponse> {
    let json = json.into_inner();
    let plan = if let Some(prev_plan) = json.previous_plan {
        orchestrator.replan(json.prompt, prev_plan).await
    } else {
        orchestrator.plan(json.prompt).await
    };
    let plan = match plan {
        Ok(p) => p,
        Err(_) => return Ok(actix_web::HttpResponse::InternalServerError().finish()),
    };
    Ok(Json(plan).into())
}
