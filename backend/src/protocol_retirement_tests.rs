//! Route behavior at the platform boundary after the single-authority cutover.
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use std::{collections::HashMap, sync::Arc};
use tower::ServiceExt;

#[tokio::test]
async fn retired_protocol_and_registry_routes_stay_absent_for_every_deployment_mode() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/unused-retirement-check")
        .unwrap();
    let config = crate::config::Config::from_values(&HashMap::new()).unwrap();
    let state = crate::state::AppState::new(pool, Arc::new(config));
    for remote in [false, true] {
        let router = crate::routes::api_router_for_deployment(remote).with_state(state.clone());
        for path in [
            "/oauth/authorize",
            "/oauth/token",
            "/oauth/introspect",
            "/oauth/revoke",
            "/oauth/device/authorization",
            "/oauth/.well-known/openid-configuration",
            "/oidc/authorize",
            "/oidc/token",
            "/api/sso/login",
            "/api/sso/challenge",
            "/api/oauth/clients",
            "/api/oauth/tokens",
            "/api/oauth/device-codes",
            "/api/oauth/session-token",
            "/api/oidc/clients",
            "/api/oidc/keys/publish",
            "/api/customer/context",
            "/api/customer/profile",
            "/api/customer/applications",
        ] {
            for method in [Method::GET, Method::POST] {
                let response = router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri(path)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::NOT_FOUND,
                    "retired path {path}; remote={remote}"
                );
            }
        }
    }
}
