//! Removed public platform refresh credentials must never be accepted.
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn password_grants_and_public_platform_refresh_are_rejected_without_database_access() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://fixture@127.0.0.1:1/sso_negative_fixture")
        .unwrap();
    let config = crate::config::Config::from_values(&Default::default()).unwrap();
    let state = crate::state::AppState::new(pool, Arc::new(config));
    let app = crate::routes::oauth2::oauth_router().with_state(state);
    for (body, error) in [
        (
            "grant_type=password&username=admin&password=unused",
            "unsupported_grant_type",
        ),
        (
            "grant_type=refresh_token&refresh_token=psr_retired-fixture",
            "invalid_grant",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/token")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(value["error"], error);
    }
}
