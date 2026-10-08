use actix_web::body::{EitherBody, MessageBody};
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::web::Data;

/// Bearer token required for `/api/` routes when configured.
pub(crate) struct ApiKey(pub(crate) Option<String>);

pub(crate) async fn require_api_key(
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<EitherBody<impl MessageBody>>, actix_web::Error> {
    let expected = req.app_data::<Data<ApiKey>>().and_then(|key| key.0.clone());
    if let Some(expected) = expected
        && req.path().starts_with("/api/")
    {
        let provided = req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(bearer_token);
        if !provided
            .is_some_and(|provided| constant_time_eq(provided.as_bytes(), expected.as_bytes()))
        {
            let response = actix_web::HttpResponse::Unauthorized()
                .insert_header(("WWW-Authenticate", "Bearer"))
                .finish();
            return Ok(req.into_response(response).map_into_right_body());
        }
    }
    Ok(next.call(req).await?.map_into_left_body())
}

/// The authentication scheme name is case-insensitive (RFC 9110).
fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim_start_matches(' '))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, http::StatusCode, middleware::from_fn, test, web};

    #[actix_web::test]
    async fn api_routes_require_the_configured_bearer_token() {
        for key in [None, Some("secret".to_string())] {
            let app = test::init_service(
                App::new()
                    .wrap(from_fn(require_api_key))
                    .app_data(Data::new(ApiKey(key.clone())))
                    .route("/api/v1/test", web::get().to(actix_web::HttpResponse::Ok))
                    .route("/version", web::get().to(actix_web::HttpResponse::Ok)),
            )
            .await;
            for (path, header, authorized) in [
                ("/api/v1/test", None, key.is_none()),
                ("/api/v1/test", Some("Bearer wrong"), key.is_none()),
                ("/api/v1/test", Some("Bearer secret"), true),
                ("/api/v1/test", Some("bearer secret"), true),
                ("/api/v1/test", Some("BEARER  secret"), true),
                ("/api/v1/test", Some("Basic secret"), key.is_none()),
                ("/api/v1/test", Some("Bearersecret"), key.is_none()),
                ("/version", None, true),
            ] {
                let mut request = test::TestRequest::get().uri(path);
                if let Some(header) = header {
                    request = request.insert_header(("Authorization", header));
                }
                let response = test::call_service(&app, request.to_request()).await;
                let expected = if authorized {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                };
                assert_eq!(response.status(), expected, "{key:?} {path} {header:?}");
            }
        }
    }
}
