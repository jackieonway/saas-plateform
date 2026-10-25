//! Real admission-policy API contracts on disposable, explicitly authorized databases.
//! Only IDENTITY_PHASE02_TEST_DATABASE_URL is read; no .env or preview database fallback.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::{middleware, Router};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::state::AppState;
use crate::util::uuid7::new_uuid;

const SECRET: &str = "owned-phase02-policy-fixture-only";
const PASSWORD: &str = "Owned-policy-password-123!";
const REASON: &str = "Owned Phase 2 admission contract change";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let url = std::env::var("IDENTITY_PHASE02_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE02_TEST_DATABASE_URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid isolated admission database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "admission fixture administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated admission administration connection failed"))?;
        let name = format!("identity_phase02_policy_{}", new_uuid().simple());
        ensure!(
            name.len() <= 63,
            "owned database name exceeds PostgreSQL limit"
        );
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = match PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                return Err(anyhow!("owned admission fixture connection failed"));
            }
        };
        let database = Self { pool, admin, name };
        if let Err(error) = sqlx::migrate!("./migrations").run(&database.pool).await {
            let _ = database.finish(Ok(())).await;
            return Err(error.into());
        }
        Ok(database)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("identity_phase02_policy_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_'),
            "refusing cleanup outside the owned admission fixture"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let cleanup = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        let closed = closed
            || tokio::time::timeout(Duration::from_secs(5), self.pool.close())
                .await
                .is_ok();
        self.admin.close().await;
        eprintln!(
            "identity_phase02_policy_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name,
            result.is_ok(),
            cleanup.is_ok()
        );
        result?;
        cleanup?;
        ensure!(closed, "owned admission fixture pool did not close");
        Ok(())
    }
}

struct Reply {
    status: StatusCode,
    body: Value,
    request_id: Uuid,
}

async fn call(app: &Router, token: &str, method: Method, path: &str, body: Value) -> Result<Reply> {
    let request_id = new_uuid();
    let request = Request::builder()
        .method(method.clone())
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", request_id.to_string())
        .extension(ConnectInfo("192.0.2.42:30042".parse::<SocketAddr>()?))
        .body(Body::from(serde_json::to_vec(&body)?))?;
    let response = tokio::time::timeout(Duration::from_secs(25), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("admission API timeout: {method} {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    let body =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
    Ok(Reply {
        status,
        body,
        request_id,
    })
}

fn success(reply: &Reply) -> Result<()> {
    // Do not print success bodies: client creation returns a one-time secret.
    ensure!(
        reply.status.is_success(),
        "admission API returned {}",
        reply.status
    );
    Ok(())
}

fn object_id(reply: &Reply) -> Result<Uuid> {
    success(reply)?;
    reply.body["id"]
        .as_str()
        .ok_or_else(|| anyhow!("admission DTO lacks id"))?
        .parse()
        .map_err(Into::into)
}

fn policy_version(body: &Value) -> Result<i64> {
    body["accessPolicyVersion"]
        .as_i64()
        .ok_or_else(|| anyhow!("admission DTO lacks accessPolicyVersion"))
}

fn assert_policy(body: &Value, id: Uuid, allowed: Value, mode: Option<&str>) -> Result<()> {
    ensure!(body["id"] == json!(id), "admission DTO has wrong id");
    ensure!(
        body["allowedMemberTypes"] == allowed,
        "admission DTO has incorrect normalized allowedMemberTypes"
    );
    ensure!(
        policy_version(body)? >= 1,
        "admission DTO has invalid policy version"
    );
    ensure!(
        body["status"] == "ENABLED",
        "admission DTO lacks enabled status"
    );
    if let Some(mode) = mode {
        ensure!(
            body["subjectMode"] == mode,
            "client DTO has incorrect subjectMode"
        );
    }
    Ok(())
}

fn application_body(code: &str) -> Value {
    json!({"code":code,"name":format!("Owned admission {code}"),"entryType":"HTML","runtimeMode":"TRUSTED"})
}

fn client_body(name: &str) -> Value {
    json!({"clientName":name,"clientType":"CONFIDENTIAL","grantTypes":["authorization_code"],
        "redirectUris":["https://policy.example.test/callback"],"scopes":["profile"],"requirePkce":true})
}

fn update_body(allowed: Value, version: i64) -> Value {
    json!({"allowedMemberTypes":allowed,"accessPolicyVersion":version,"reason":REASON})
}

struct Harness {
    pool: PgPool,
    app: Router,
    admin: String,
    actor: Uuid,
}

impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let tenant = new_uuid();
        let org = new_uuid();
        let actor = new_uuid();
        let membership = new_uuid();
        let management = new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned policy platform','owned-policy-platform',true)")
            .bind(tenant).execute(pool).await?;
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned policy root')",
        )
        .bind(org)
        .bind(tenant)
        .execute(pool)
        .await?;
        let config = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/owned-policy".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let state = AppState::new(pool.clone(), Arc::new(config));
        let password_hash = state.passwords.hash(PASSWORD).await?;
        sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin) VALUES($1,'owned-policy-manager',$2,true)")
            .bind(actor).bind(password_hash).execute(pool).await?;
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE')")
            .bind(membership).bind(actor).bind(org).execute(pool).await?;
        sqlx::query("INSERT INTO applications(id,name,code,base_path,entry_type) VALUES($1,'Owned policy management','system','/micro/system','HTML')")
            .bind(management).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(management)
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(org)
        .bind(management)
        .execute(pool)
        .await?;
        for (permission, path) in [
            ("system:application", "/system/applications"),
            ("system:oauth", "/system/oauth/clients"),
        ] {
            let menu = new_uuid();
            sqlx::query("INSERT INTO menus(id,application_id,title,permission,type,path) VALUES($1,$2,$3,$3,'MENU',$4)")
                .bind(menu).bind(management).bind(permission).bind(path).execute(pool).await?;
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(tenant)
                .bind(menu)
                .execute(pool)
                .await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org)
                .bind(menu)
                .execute(pool)
                .await?;
        }
        let app = crate::routes::api_router()
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::observability::observe,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::normalize_client_ip,
            ))
            .with_state(state);
        // Exercise the real login/context flow, so new identity bindings are issued
        // by the server rather than silently filling a legacy fixture session.
        let login = call(
            &app,
            "",
            Method::POST,
            "/api/auth/login",
            json!({"username":"owned-policy-manager","password":PASSWORD}),
        )
        .await?;
        success(&login)?;
        let ticket = login.body["ticket"]
            .as_str()
            .ok_or_else(|| anyhow!("login lacks ticket"))?;
        let context = call(
            &app,
            ticket,
            Method::POST,
            "/api/auth/context",
            json!({"organizationId":org,"membershipId":membership,"userId":actor}),
        )
        .await?;
        success(&context)?;
        // Selection identifies the exact relation; its type is server proof,
        // not an accepted request field in the console context contract.
        ensure!(
            context.body["membershipId"] == json!(membership)
                && context.body["memberType"] == "EMPLOYEE"
                && context.body["user"]["id"] == json!(actor)
                && context.body["organization"]["id"] == json!(org),
            "context proof differs from the selected admission administrator"
        );
        let admin = context.body["accessToken"]
            .as_str()
            .ok_or_else(|| anyhow!("context lacks accessToken"))?
            .to_string();
        Ok(Self {
            pool: pool.clone(),
            app,
            admin,
            actor,
        })
    }

    async fn call(&self, method: Method, path: &str, body: Value) -> Result<Reply> {
        call(&self.app, &self.admin, method, path, body).await
    }

    async fn create_application(&self, code: &str) -> Result<(Uuid, Value)> {
        let reply = self
            .call(Method::POST, "/api/applications", application_body(code))
            .await?;
        let id = object_id(&reply)?;
        assert_policy(&reply.body, id, json!(["EMPLOYEE"]), None)?;
        Ok((id, reply.body))
    }

    async fn create_client(&self, name: &str) -> Result<(Uuid, Value)> {
        let reply = self
            .call(Method::POST, "/api/oauth/clients", client_body(name))
            .await?;
        let id = object_id(&reply)?;
        assert_policy(&reply.body, id, json!(["EMPLOYEE"]), Some("USER"))?;
        Ok((id, reply.body))
    }

    async fn audit_count(&self, request_id: Uuid) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE request_id=$1")
                .bind(request_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    async fn assert_audit(&self, reply: &Reply, id: Uuid, before_version: i64) -> Result<()> {
        let rows: Vec<(Option<Uuid>, String)> =
            sqlx::query_as("SELECT user_id,detail FROM audit_logs WHERE request_id=$1")
                .bind(reply.request_id)
                .fetch_all(&self.pool)
                .await?;
        ensure!(
            rows.len() == 1,
            "policy mutation did not commit exactly one audit"
        );
        ensure!(
            rows[0].0 == Some(self.actor),
            "policy audit lacks actual actor"
        );
        let detail: Value = serde_json::from_str(&rows[0].1)?;
        ensure!(
            detail["objectId"] == json!(id),
            "policy audit lacks object id"
        );
        ensure!(
            detail["before"]["accessPolicyVersion"] == before_version,
            "policy audit lacks previous version"
        );
        ensure!(
            detail["after"]["accessPolicyVersion"] == before_version + 1,
            "policy audit lacks committed version"
        );
        ensure!(
            detail["after"]["reason"] == REASON,
            "policy audit lost the submitted change reason"
        );
        for secret in [SECRET, PASSWORD, self.admin.as_str()] {
            ensure!(
                !rows[0].1.contains(secret),
                "policy audit contains fixture credential material"
            );
        }
        Ok(())
    }
}

async fn policy_contracts(h: &Harness) -> Result<()> {
    let (app_id, application) = h.create_application("policy-contract").await?;
    let (client_id, client) = h.create_client("Owned policy contract").await?;
    for (root, id, original, mode) in [
        ("/api/applications", app_id, application, None),
        ("/api/oauth/clients", client_id, client, Some("USER")),
    ] {
        let object_path = format!("{root}/{id}");
        let policy_path = format!("{object_path}/access-policy");
        let version = policy_version(&original)?;
        let initial = h.call(Method::GET, &policy_path, Value::Null).await?;
        success(&initial)?;
        assert_policy(&initial.body, id, json!(["EMPLOYEE"]), mode)?;
        let list = h.call(Method::GET, root, Value::Null).await?;
        success(&list)?;
        let item = list
            .body
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == json!(id)))
            .ok_or_else(|| anyhow!("normal list lacks created policy object"))?;
        assert_policy(item, id, json!(["EMPLOYEE"]), mode)?;
        let get = h.call(Method::GET, &object_path, Value::Null).await?;
        success(&get)?;
        assert_policy(&get.body, id, json!(["EMPLOYEE"]), mode)?;

        let invalid = [
            json!({"accessPolicyVersion":version,"reason":REASON}),
            update_body(Value::Null, version),
            update_body(json!([]), version),
            update_body(json!(["UNKNOWN"]), version),
            update_body(json!(["employee"]), version),
            json!({"allowedMemberTypes":["CUSTOMER"],"reason":REASON}),
            json!({"allowedMemberTypes":["CUSTOMER"],"accessPolicyVersion":version}),
            json!({"allowedMemberTypes":["CUSTOMER"],"accessPolicyVersion":version,"reason":"  "}),
            json!({"allowedMemberTypes":["CUSTOMER"],"accessPolicyVersion":version,"reason":"x".repeat(501)}),
            json!({"allowedMemberTypes":["CUSTOMER"],"accessPolicyVersion":version,"reason":"invalid\nreason"}),
            json!({"allowedMemberTypes":["CUSTOMER"],"accessPolicyVersion":version,"reason":"invalid\u{0000}reason"}),
        ];
        for body in invalid {
            let reply = h.call(Method::PUT, &policy_path, body).await?;
            ensure!(
                reply.status == StatusCode::BAD_REQUEST,
                "invalid admission policy returned {}",
                reply.status
            );
            ensure!(
                h.audit_count(reply.request_id).await? == 0,
                "rejected policy emitted a successful audit"
            );
            let unchanged = h.call(Method::GET, &policy_path, Value::Null).await?;
            success(&unchanged)?;
            ensure!(
                policy_version(&unchanged.body)? == version,
                "invalid admission write changed policy version"
            );
            assert_policy(&unchanged.body, id, json!(["EMPLOYEE"]), mode)?;
        }
        if mode.is_some() {
            let mut null_mode = update_body(json!(["EMPLOYEE"]), version);
            null_mode["subjectMode"] = Value::Null;
            let rejected = h.call(Method::PUT, &policy_path, null_mode).await?;
            ensure!(
                rejected.status == StatusCode::BAD_REQUEST,
                "explicit null subjectMode was accepted by policy update"
            );
            ensure!(
                h.audit_count(rejected.request_id).await? == 0,
                "null subjectMode update committed an audit"
            );
        }
        let changed = h
            .call(
                Method::PUT,
                &policy_path,
                update_body(json!(["CUSTOMER", "EMPLOYEE", "CUSTOMER"]), version),
            )
            .await?;
        success(&changed)?;
        assert_policy(&changed.body, id, json!(["EMPLOYEE", "CUSTOMER"]), mode)?;
        ensure!(
            policy_version(&changed.body)? == version + 1,
            "real policy change did not increment once"
        );
        h.assert_audit(&changed, id, version).await?;
        let stale = h
            .call(
                Method::PUT,
                &policy_path,
                update_body(json!(["CUSTOMER"]), version),
            )
            .await?;
        ensure!(
            stale.status == StatusCode::CONFLICT,
            "stale policy version did not return 409"
        );
        ensure!(
            h.audit_count(stale.request_id).await? == 0,
            "stale update committed an audit"
        );
        let noop = h
            .call(
                Method::PUT,
                &policy_path,
                update_body(json!(["CUSTOMER", "EMPLOYEE", "EMPLOYEE"]), version + 1),
            )
            .await?;
        success(&noop)?;
        ensure!(
            policy_version(&noop.body)? == version + 1,
            "normalized no-op incremented policy version"
        );

        let mut ordinary = if mode.is_some() {
            json!({"clientName":"Owned ordinary client policy update"})
        } else {
            application_body("policy-contract")
        };
        let mut null_policy = ordinary.clone();
        null_policy["allowedMemberTypes"] = Value::Null;
        null_policy["accessPolicyVersion"] = json!(version + 1);
        null_policy["reason"] = json!(REASON);
        let rejected = h.call(Method::PUT, &object_path, null_policy).await?;
        ensure!(
            rejected.status == StatusCode::BAD_REQUEST,
            "ordinary update accepted explicit null allowedMemberTypes"
        );
        if mode.is_some() {
            let mut null_mode = ordinary.clone();
            null_mode["subjectMode"] = Value::Null;
            null_mode["accessPolicyVersion"] = json!(version + 1);
            null_mode["reason"] = json!(REASON);
            let rejected = h.call(Method::PUT, &object_path, null_mode).await?;
            ensure!(
                rejected.status == StatusCode::BAD_REQUEST,
                "ordinary client update accepted explicit null subjectMode"
            );
        }
        ordinary["allowedMemberTypes"] = json!(["CUSTOMER"]);
        let missing_version = h.call(Method::PUT, &object_path, ordinary.clone()).await?;
        ensure!(
            missing_version.status == StatusCode::BAD_REQUEST,
            "ordinary update bypassed required policy version"
        );
        ordinary["accessPolicyVersion"] = json!(version + 1);
        let missing_reason = h.call(Method::PUT, &object_path, ordinary.clone()).await?;
        ensure!(
            missing_reason.status == StatusCode::BAD_REQUEST,
            "ordinary update bypassed required change reason"
        );
        ordinary["reason"] = json!(REASON);
        let updated = h.call(Method::PUT, &object_path, ordinary).await?;
        success(&updated)?;
        assert_policy(&updated.body, id, json!(["CUSTOMER"]), mode)?;
        ensure!(
            policy_version(&updated.body)? == version + 2,
            "ordinary policy update did not increment once"
        );
        h.assert_audit(&updated, id, version + 1).await?;
    }

    for (path, mut body) in [
        ("/api/applications", application_body("policy-null")),
        (
            "/api/oauth/clients",
            client_body("Owned null member policy"),
        ),
    ] {
        body["allowedMemberTypes"] = Value::Null;
        let rejected = h.call(Method::POST, path, body).await?;
        ensure!(
            rejected.status == StatusCode::BAD_REQUEST,
            "ordinary creation accepted explicit null allowedMemberTypes"
        );
    }
    let mut null_mode = client_body("Owned null subject policy");
    null_mode["subjectMode"] = Value::Null;
    let rejected = h
        .call(Method::POST, "/api/oauth/clients", null_mode)
        .await?;
    ensure!(
        rejected.status == StatusCode::BAD_REQUEST,
        "client creation accepted explicit null subjectMode"
    );

    let clients = h
        .call(Method::GET, "/api/oauth/clients", Value::Null)
        .await?;
    success(&clients)?;
    let reserved = clients
        .body
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["clientId"] == "platform-console-legacy")
        })
        .ok_or_else(|| anyhow!("migration did not register the reserved console client"))?;
    let reserved_id: Uuid = reserved["id"]
        .as_str()
        .ok_or_else(|| anyhow!("reserved console client lacks id"))?
        .parse()?;
    let reserved_version = policy_version(reserved)?;
    let reserved_path = format!("/api/oauth/clients/{reserved_id}");
    let removed = h.call(Method::DELETE, &reserved_path, Value::Null).await?;
    ensure!(
        removed.status == StatusCode::BAD_REQUEST,
        "reserved console client deletion did not return 400"
    );
    ensure!(
        h.audit_count(removed.request_id).await? == 0,
        "rejected reserved console deletion committed an audit"
    );
    let changed = h
        .call(
            Method::PUT,
            &format!("{reserved_path}/access-policy"),
            update_body(json!(["CUSTOMER"]), reserved_version),
        )
        .await?;
    ensure!(
        changed.status == StatusCode::BAD_REQUEST,
        "reserved console client accepted a customer admission policy"
    );
    ensure!(
        h.audit_count(changed.request_id).await? == 0,
        "rejected reserved console policy committed an audit"
    );
    let retained = h.call(Method::GET, &reserved_path, Value::Null).await?;
    success(&retained)?;
    assert_policy(
        &retained.body,
        reserved_id,
        json!(["EMPLOYEE"]),
        Some("USER"),
    )?;
    ensure!(
        policy_version(&retained.body)? == reserved_version,
        "rejected reserved console mutations changed its policy version"
    );
    for key in [
        "clientId",
        "clientType",
        "grantTypes",
        "scopes",
        "redirectUris",
        "requirePkce",
        "applicationId",
    ] {
        ensure!(
            retained.body[key] == reserved[key],
            "reserved console registration changed its {key} after rejected writes"
        );
    }

    let mut both = application_body("policy-create");
    both["allowedMemberTypes"] = json!(["CUSTOMER", "EMPLOYEE", "CUSTOMER"]);
    let created = h.call(Method::POST, "/api/applications", both).await?;
    assert_policy(
        &created.body,
        object_id(&created)?,
        json!(["EMPLOYEE", "CUSTOMER"]),
        None,
    )?;
    let mut both = client_body("Owned explicit client policy");
    both["allowedMemberTypes"] = json!(["CUSTOMER", "EMPLOYEE", "CUSTOMER"]);
    let created = h.call(Method::POST, "/api/oauth/clients", both).await?;
    assert_policy(
        &created.body,
        object_id(&created)?,
        json!(["EMPLOYEE", "CUSTOMER"]),
        Some("USER"),
    )?;

    for (kind, grants, mode, allowed) in [
        (
            "PUBLIC",
            json!(["client_credentials"]),
            "MACHINE",
            json!([]),
        ),
        (
            "CONFIDENTIAL",
            json!(["authorization_code"]),
            "MACHINE",
            json!([]),
        ),
        (
            "CONFIDENTIAL",
            json!(["client_credentials"]),
            "MACHINE",
            json!(["EMPLOYEE"]),
        ),
        (
            "CONFIDENTIAL",
            json!(["authorization_code"]),
            "USER",
            json!([]),
        ),
        (
            "CONFIDENTIAL",
            json!(["authorization_code", "client_credentials"]),
            "LEGACY_MIXED",
            json!(["EMPLOYEE"]),
        ),
    ] {
        let mut body = client_body("Owned invalid subject contract");
        body["clientType"] = json!(kind);
        body["grantTypes"] = grants;
        body["subjectMode"] = json!(mode);
        body["allowedMemberTypes"] = allowed;
        let reply = h.call(Method::POST, "/api/oauth/clients", body).await?;
        ensure!(
            reply.status == StatusCode::BAD_REQUEST,
            "invalid new client subject contract returned {}",
            reply.status
        );
    }
    let mut machine = client_body("Owned machine policy");
    machine["grantTypes"] = json!(["client_credentials"]);
    machine["subjectMode"] = json!("MACHINE");
    machine["allowedMemberTypes"] = json!([]);
    let machine = h.call(Method::POST, "/api/oauth/clients", machine).await?;
    let machine_id = object_id(&machine)?;
    assert_policy(&machine.body, machine_id, json!([]), Some("MACHINE"))?;
    let machine_path = format!("/api/oauth/clients/{machine_id}/access-policy");
    let unchanged = h
        .call(
            Method::PUT,
            &machine_path,
            update_body(json!([]), policy_version(&machine.body)?),
        )
        .await?;
    success(&unchanged)?;
    ensure!(
        policy_version(&unchanged.body)? == policy_version(&machine.body)?,
        "machine no-op changed policy version"
    );
    let nonempty = h
        .call(
            Method::PUT,
            &machine_path,
            update_body(json!(["CUSTOMER"]), policy_version(&machine.body)?),
        )
        .await?;
    ensure!(
        nonempty.status == StatusCode::BAD_REQUEST,
        "machine policy accepted human identities"
    );
    let current = h
        .call(
            Method::GET,
            &format!("/api/oauth/clients/{client_id}/access-policy"),
            Value::Null,
        )
        .await?;
    success(&current)?;
    let mut legacy = update_body(json!(["EMPLOYEE"]), policy_version(&current.body)?);
    legacy["subjectMode"] = json!("LEGACY_MIXED");
    let legacy = h
        .call(
            Method::PUT,
            &format!("/api/oauth/clients/{client_id}/access-policy"),
            legacy,
        )
        .await?;
    ensure!(
        legacy.status == StatusCode::BAD_REQUEST,
        "USER client was permitted to enter LEGACY_MIXED"
    );
    Ok(())
}

async fn atomicity_and_concurrency(h: &Harness) -> Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE phase02_policy_audit_fault(mode text NOT NULL); \
         INSERT INTO phase02_policy_audit_fault VALUES('off'); \
         CREATE FUNCTION phase02_policy_reject_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
         DECLARE fault text; BEGIN SELECT mode INTO fault FROM phase02_policy_audit_fault; \
           IF fault='raise' THEN RAISE EXCEPTION 'owned policy audit fault' USING ERRCODE='P0001'; END IF; \
           IF fault='skip' THEN RETURN NULL; END IF; RETURN NEW; END $$; \
         CREATE TRIGGER phase02_policy_audit_fault BEFORE INSERT ON audit_logs \
         FOR EACH ROW EXECUTE FUNCTION phase02_policy_reject_audit();",
    ).execute(&h.pool).await?;
    let (application, app_body) = h.create_application("policy-atomic").await?;
    let (client, client_body) = h.create_client("Owned atomic policy client").await?;
    for (table, root, id, original, mode) in [
        (
            "applications",
            "/api/applications",
            application,
            app_body,
            None,
        ),
        (
            "oauth_clients",
            "/api/oauth/clients",
            client,
            client_body,
            Some("USER"),
        ),
    ] {
        let path = format!("{root}/{id}/access-policy");
        let version = policy_version(&original)?;
        for fault in ["raise", "skip"] {
            sqlx::query("UPDATE phase02_policy_audit_fault SET mode=$1")
                .bind(fault)
                .execute(&h.pool)
                .await?;
            let failed = h
                .call(
                    Method::PUT,
                    &path,
                    update_body(json!(["CUSTOMER"]), version),
                )
                .await?;
            ensure!(
                failed.status == StatusCode::INTERNAL_SERVER_ERROR,
                "audit {fault} fault was not propagated"
            );
            ensure!(
                h.audit_count(failed.request_id).await? == 0,
                "failed audit transaction committed an audit"
            );
            let after = h.call(Method::GET, &path, Value::Null).await?;
            success(&after)?;
            assert_policy(&after.body, id, json!(["EMPLOYEE"]), mode)?;
            ensure!(
                policy_version(&after.body)? == version,
                "audit {fault} fault failed to roll back policy version"
            );
        }
        sqlx::query("UPDATE phase02_policy_audit_fault SET mode='off'")
            .execute(&h.pool)
            .await?;

        // Make both real HTTP writes wait on the same owned row before either
        // can commit; this exercises stale-version checks after lock waits.
        let mut blocker = h.pool.begin().await?;
        sqlx::query(&format!("SELECT id FROM {table} WHERE id=$1 FOR UPDATE"))
            .bind(id)
            .execute(&mut *blocker)
            .await?;
        let first_app = h.app.clone();
        let first_token = h.admin.clone();
        let first_path = path.clone();
        let first = tokio::spawn(async move {
            call(
                &first_app,
                &first_token,
                Method::PUT,
                &first_path,
                update_body(json!(["CUSTOMER"]), version),
            )
            .await
        });
        let second_app = h.app.clone();
        let second_token = h.admin.clone();
        let second_path = path.clone();
        let second = tokio::spawn(async move {
            call(
                &second_app,
                &second_token,
                Method::PUT,
                &second_path,
                update_body(json!(["EMPLOYEE", "CUSTOMER"]), version),
            )
            .await
        });
        let mut both_waiting = false;
        for _ in 0..100 {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() \
                 AND pid<>pg_backend_pid() AND state='active' AND wait_event_type='Lock'",
            )
            .fetch_one(&h.pool)
            .await?;
            if waiting >= 2 {
                both_waiting = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        blocker.commit().await?;
        let first = first.await??;
        let second = second.await??;
        ensure!(
            both_waiting,
            "concurrent API writes did not both reach the owned row lock"
        );
        let (winner, loser) = if first.status.is_success() {
            (&first, &second)
        } else {
            (&second, &first)
        };
        success(winner)?;
        ensure!(
            loser.status == StatusCode::CONFLICT,
            "concurrent stale policy writer did not receive 409"
        );
        h.assert_audit(winner, id, version).await?;
        ensure!(
            h.audit_count(loser.request_id).await? == 0,
            "concurrent losing transaction committed an audit"
        );
        let after = h.call(Method::GET, &path, Value::Null).await?;
        success(&after)?;
        ensure!(
            policy_version(&after.body)? == version + 1,
            "concurrent policy updates incremented version more than once"
        );
        ensure!(
            after.body["allowedMemberTypes"] == winner.body["allowedMemberTypes"],
            "committed policy differs from winning API response"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_policy_* databases via explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn admission_policy_api_contracts_and_versions() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let harness = Harness::create(&database.pool).await?;
        policy_contracts(&harness).await
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "real concurrent policy writes and audit fault injection in an owned identity_phase02_policy_* database"]
async fn admission_policy_audit_atomicity_and_concurrent_conflicts() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let harness = Harness::create(&database.pool).await?;
        atomicity_and_concurrency(&harness).await
    }
    .await;
    database.finish(result).await
}
