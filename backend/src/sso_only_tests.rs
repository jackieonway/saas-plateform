//! Public console entry points cannot enable a password-mode fallback.
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn console_configuration_is_always_sso_and_password_routes_are_absent() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://fixture@127.0.0.1:1/sso_route_fixture")
        .unwrap();
    let config = crate::config::Config::from_values(&Default::default()).unwrap();
    let state = crate::state::AppState::new(pool, Arc::new(config));
    let app = crate::routes::api_router_for_deployment(true).with_state(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024).await.unwrap()).unwrap();
    assert_eq!(value, serde_json::json!({"mode":"BFF"}));
    for path in ["/api/auth/login", "/api/auth/context", "/api/auth/logout"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "retired console route: {path}"
        );
    }
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn migration_retires_password_clients_and_unbound_console_sessions() -> anyhow::Result<()> {
    let db = crate::entity_creation_tests::Database::create().await?;
    let result = async {
        let retired_relations: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_class relation JOIN pg_namespace namespace ON namespace.oid=relation.relnamespace WHERE namespace.nspname='public' AND relation.relkind IN ('r','p','v','m','f') AND relation.relname ~ '^(oauth_|oidc_)'")
            .fetch_one(&db.pool).await?;
        assert_eq!(retired_relations, 0);
        let password_clients: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_clients WHERE 'password'=ANY(grant_types)")
            .fetch_one(&db.pool).await?;
        assert_eq!(password_clients, 0);
        let unbound: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_sessions WHERE bff_session_id IS NULL AND revoked_at IS NULL")
            .fetch_one(&db.pool).await?;
        assert_eq!(unbound, 0);
        Ok(())
    }.await;
    db.finish(result).await
}
