use super::authority::{Client, SourceFuture, SourcePort, SourceProof};
use super::*;
use anyhow::{anyhow, ensure, Result as TestResult};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{Duration, Utc};
use saas_platform_backend::context_selection::{Intent, Source, Version};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tower::ServiceExt;
use uuid::Uuid;

struct Authority {
    proof: SourceProof,
    active: Arc<AtomicBool>,
}
impl SourcePort for Authority {
    fn verify<'a>(
        &'a self,
        source: &'a str,
        client: Id,
        _access: Option<&'a str>,
    ) -> SourceFuture<'a> {
        Box::pin(async move {
            if !self.active.load(Ordering::SeqCst)
                || source != "a".repeat(64)
                || client != self.proof.client.id
            {
                return Err(AppError::unauthorized("fixture original source revoked"));
            }
            Ok(self.proof.clone())
        })
    }
}

fn id() -> Id {
    Id::new(Uuid::now_v7()).unwrap()
}
fn version(v: i64) -> Version {
    Version::new(v).unwrap()
}

async fn post(
    app: &Router,
    path: &str,
    authorization: &str,
    body: Value,
) -> TestResult<(StatusCode, Value)> {
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .header("authorization", authorization)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body)?))?,
        )
        .await?;
    ensure!(
        response
            .headers()
            .get("cache-control")
            .is_some_and(|v| v == "no-store"),
        "selection responses cannot be cached"
    );
    let status = response.status();
    let body = to_bytes(response.into_body(), 65536).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

async fn cases(pool: PgPool) -> TestResult<()> {
    let tenant = id();
    let org = id();
    let user = id();
    let employee = id();
    let customer = id();
    let client = id();
    let service = id();
    let application = id();
    sqlx::query(
        "INSERT INTO tenants(id,name,code) VALUES($1,'Selection tenant','selection-tenant')",
    )
    .bind(tenant.uuid())
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO organizations(id,tenant_id,name,code,organization_type) VALUES($1,$2,'Selection org','selection-org','COMPANY')").bind(org.uuid()).bind(tenant.uuid()).execute(&pool).await?;
    sqlx::query("INSERT INTO users(id,username,password_hash,status,session_version) VALUES($1,'selection-user','unused','ENABLED',1)").bind(user.uuid()).execute(&pool).await?;
    for (member, kind) in [(employee, "EMPLOYEE"), (customer, "CUSTOMER")] {
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,identity_version) VALUES($1,$2,$3,$4,1)").bind(member.uuid()).bind(user.uuid()).bind(org.uuid()).bind(kind).execute(&pool).await?;
    }
    sqlx::query("INSERT INTO applications(id,name,code,base_path,status,allowed_member_types) VALUES($1,'Selection app','selection-app','/selection','ENABLED',ARRAY['EMPLOYEE'])").bind(application.uuid()).execute(&pool).await?;
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
        .bind(tenant.uuid())
        .bind(application.uuid())
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
    )
    .bind(org.uuid())
    .bind(application.uuid())
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO context_selection_services(id,secret_hash,purpose,allowed_oidc_clients,status) VALUES($1,$2,'CONTEXT_SELECTION_AUTHORITY',ARRAY[$3]::uuid[],'ENABLED')")
        .bind(service.uuid()).bind(crate::util::hash::sha256_hex(&"b".repeat(64))).bind(client.uuid()).execute(&pool).await?;
    sqlx::query("INSERT INTO context_selection_registrations(client_id,client_policy_version,member_type,application_id,application_policy_version) VALUES($1,2,'EMPLOYEE',$2,1)").bind(client.uuid()).bind(application.uuid()).execute(&pool).await?;
    let active = Arc::new(AtomicBool::new(true));
    let proof = SourceProof {
        contract_version: ContractVersion::V1,
        transaction_id: id(),
        intent: Intent::Login {},
        source: Source {
            issuer: "https://auth.saas.test".to_owned(),
            user_id: user,
            account_version: version(1),
            global_session_id: id(),
            interaction_id: id(),
            auth_time: Utc::now() - Duration::minutes(1),
        },
        client: Client {
            id: client,
            policy_version: version(2),
            purpose: "PLATFORM_CONSOLE".to_owned(),
            allowed_member_types: vec!["EMPLOYEE".to_owned()],
            redirect_uri: "https://platform.saas.test/auth/callback".to_owned(),
        },
        source_expires_at: Utc::now() + Duration::minutes(5),
        interaction_expires_at: Utc::now() + Duration::minutes(5),
    };
    let mut config = crate::config::Config::from_values(&Default::default())?;
    config.oidc_issuer = Some("https://auth.saas.test".to_owned());
    let mut state = AppState::new(pool.clone(), Arc::new(config));
    state.context_selection_authority = Arc::new(Authority {
        proof,
        active: active.clone(),
    });
    let app = routes().with_state(state);
    let authorization = format!(
        "Basic {}",
        STANDARD.encode(format!("{}:{}", service.uuid(), "b".repeat(64)))
    );
    let request = json!({"contractVersion":"context-selection/1","sourceProof":"a".repeat(64),"expectedClientId":client});
    let (status, choices) = post(
        &app,
        "/internal/platform/context-selection/choices",
        &authorization,
        request.clone(),
    )
    .await?;
    ensure!(
        status == StatusCode::OK,
        "choices must succeed: {status} {choices}"
    );
    ensure!(
        choices["members"].as_array().is_some_and(|v| v.len() == 1)
            && choices["members"][0]["membershipId"] == employee.uuid().to_string(),
        "employee RP must expose only the exact employee member"
    );
    let mut confirm = request.clone();
    confirm["membershipId"] = json!(customer);
    ensure!(
        post(
            &app,
            "/internal/platform/context-selection/confirm",
            &authorization,
            confirm.clone()
        )
        .await?
        .0 == StatusCode::FORBIDDEN,
        "employee RP cannot confirm a customer"
    );
    confirm["membershipId"] = json!(employee);
    sqlx::query(
        "DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2",
    )
    .bind(org.uuid())
    .bind(application.uuid())
    .execute(&pool)
    .await?;
    ensure!(
        post(
            &app,
            "/internal/platform/context-selection/confirm",
            &authorization,
            confirm.clone()
        )
        .await?
        .0 == StatusCode::FORBIDDEN,
        "application access is checked again at confirmation"
    );
    sqlx::query(
        "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
    )
    .bind(org.uuid())
    .bind(application.uuid())
    .execute(&pool)
    .await?;
    let (status, selected) = post(
        &app,
        "/internal/platform/context-selection/confirm",
        &authorization,
        confirm.clone(),
    )
    .await?;
    ensure!(
        status == StatusCode::OK
            && selected["credential"]
                .as_str()
                .is_some_and(|v| v.len() == 64),
        "confirmation creates a backend credential: {status}"
    );
    ensure!(
        post(
            &app,
            "/internal/platform/context-selection/confirm",
            &authorization,
            confirm
        )
        .await?
        .0 == StatusCode::CONFLICT,
        "same transaction cannot issue independent credentials"
    );
    active.store(false, Ordering::SeqCst);
    ensure!(
        post(
            &app,
            "/internal/platform/context-selection/choices",
            &authorization,
            request.clone()
        )
        .await?
        .0 == StatusCode::UNAUTHORIZED,
        "retired original source fails closed"
    );
    active.store(true, Ordering::SeqCst);
    sqlx::query(
        "UPDATE context_selection_services SET purpose='CONTEXT_SELECTION_CONSUMER' WHERE id=$1",
    )
    .bind(service.uuid())
    .execute(&pool)
    .await?;
    ensure!(
        post(
            &app,
            "/internal/platform/context-selection/choices",
            &authorization,
            request.clone()
        )
        .await?
        .0 == StatusCode::FORBIDDEN,
        "consumer credentials cannot act as identity authority"
    );
    let mut wrong = request;
    wrong["expectedClientId"] = json!(id());
    ensure!(
        post(
            &app,
            "/internal/platform/context-selection/choices",
            &authorization,
            wrong
        )
        .await?
        .0 == StatusCode::FORBIDDEN,
        "service cannot query unregistered RP"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires task-owned CONTEXT_SELECTION_TEST_ADMIN_URL selecting postgres"]
async fn real_platform_selection_http_is_member_and_application_scoped() -> TestResult<()> {
    let options = PgConnectOptions::from_str(&std::env::var("CONTEXT_SELECTION_TEST_ADMIN_URL")?)
        .map_err(|_| anyhow!("invalid owned test administration URL"))?;
    ensure!(
        options.get_database() == Some("postgres"),
        "explicit postgres administration database required"
    );
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await?;
    let name = format!("context_http_{}", Uuid::now_v7().simple());
    sqlx::query(&format!("CREATE DATABASE \"{name}\""))
        .execute(&admin)
        .await?;
    let task_name = name.clone();
    let result = tokio::spawn(async move {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect_with(options.database(&task_name))
            .await?;
        let result = async {
            crate::database::run_migrations(&pool).await?;
            cases(pool.clone()).await
        }
        .await;
        pool.close().await;
        result
    })
    .await;
    sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
        .execute(&admin)
        .await?;
    admin.close().await;
    result??;
    Ok(())
}
