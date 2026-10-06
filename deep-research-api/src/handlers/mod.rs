mod plan_create;
mod research_create;
mod version_get;

pub(crate) fn configure(config: &mut actix_web::web::ServiceConfig) {
    config
        .service(version_get::version_get)
        .service(plan_create::plan_create)
        .service(research_create::research_create);
}
