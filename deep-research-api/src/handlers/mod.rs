mod plan_create;
mod research_create;
mod version_get;

pub(crate) fn configure(config: &mut actix_web::web::ServiceConfig) {
    config
        .service(version_get::version_get)
        .service(plan_create::plan_create)
        .service(research_create::research_create);
}

/// The orchestrator's concurrent request limit is reached.
pub(crate) fn busy_response() -> actix_web::HttpResponse {
    actix_web::HttpResponse::ServiceUnavailable()
        .insert_header(("Retry-After", "30"))
        .json(serde_json::json!({ "error": "too many concurrent requests" }))
}
