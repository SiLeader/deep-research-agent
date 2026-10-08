mod plan_create;
mod research_create;
mod version_get;

pub(crate) fn configure(config: &mut actix_web::web::ServiceConfig) {
    config
        // Accommodate every valid plan, including JSON-escaped text.
        .app_data(actix_web::web::JsonConfig::default().limit(16 * 1024 * 1024))
        .service(version_get::version_get)
        .service(
            // Authenticate the scope the router matches, not the raw request
            // path, so percent-encoded paths cannot bypass the check.
            actix_web::web::scope("/api")
                .wrap(actix_web::middleware::from_fn(crate::auth::require_api_key))
                .service(plan_create::plan_create)
                .service(research_create::research_create),
        );
}

/// The orchestrator's concurrent request limit is reached.
pub(crate) fn busy_response() -> actix_web::HttpResponse {
    actix_web::HttpResponse::ServiceUnavailable()
        .insert_header(("Retry-After", "30"))
        .json(serde_json::json!({ "error": "too many concurrent requests" }))
}
