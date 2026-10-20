//! Hub HTTP API. Routes and shapes follow `server/openapi/hub.yaml`
//! (CONTRACT, lead). Everything except `/health` is a 501 stub for now.

use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

pub const API_PREFIX: &str = "/api/v1";

pub fn router() -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/mirrors", get(not_implemented))
        .route("/skins", get(not_implemented).post(not_implemented))
        .route("/skins/{id}", delete(not_implemented))
        .route("/me/skin", post(not_implemented))
        .route("/rooms", post(not_implemented))
        .route("/rooms/{code}/join", post(not_implemented))
        .route("/rooms/{room_id}", delete(not_implemented))
        .route("/rooms/{room_id}/ws", get(not_implemented));
    Router::new().nest(API_PREFIX, api)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn not_implemented() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(
            json!({ "code": "notImplemented", "message": "not implemented yet", "details": null }),
        ),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn health_ok() {
        let res = router()
            .oneshot(Request::get("/api/v1/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "ok");
    }

    #[tokio::test]
    async fn stubs_return_501() {
        let res = router()
            .oneshot(Request::post("/api/v1/rooms").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_IMPLEMENTED);
    }
}
