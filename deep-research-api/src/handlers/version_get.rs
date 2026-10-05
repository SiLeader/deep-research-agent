use actix_web::get;

#[get("/version")]
pub(super) async fn version_get() -> actix_web::Result<actix_web::HttpResponse> {
    let version = env!("CARGO_PKG_VERSION");
    Ok(actix_web::HttpResponse::Ok()
        .json(serde_json::json!({ "version": version, "apis": ["v1"] })))
}
