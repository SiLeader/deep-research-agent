use actix_web::post;
use actix_web::web::{Data, Json};
use deep_research_orchestrator::plan::{DeepResearchPlan, validate_prompt};
use deep_research_orchestrator::{Busy, DeepResearchOrchestrator};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct PlanCreateRequest {
    prompt: String,
    previous_plan: Option<DeepResearchPlan>,
}

#[post("/v1/deep/research/plan")]
pub(super) async fn plan_create(
    orchestrator: Data<DeepResearchOrchestrator>,
    json: Json<PlanCreateRequest>,
) -> actix_web::Result<actix_web::HttpResponse> {
    let json = json.into_inner();
    validate_prompt(&json.prompt).map_err(actix_web::error::ErrorBadRequest)?;
    let plan = if let Some(prev_plan) = json.previous_plan {
        prev_plan
            .validate()
            .map_err(actix_web::error::ErrorBadRequest)?;
        orchestrator.replan(json.prompt, prev_plan).await
    } else {
        orchestrator.plan(json.prompt).await
    };
    match plan {
        Ok(plan) => Ok(actix_web::HttpResponse::Ok().json(plan)),
        Err(error) if error.is::<Busy>() => Ok(super::busy_response()),
        Err(error) => {
            tracing::error!("Planning failed: {error:#}");
            Ok(actix_web::HttpResponse::InternalServerError()
                .json(serde_json::json!({ "error": "plan generation failed" })))
        }
    }
}
