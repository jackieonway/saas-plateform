//! Platform session refresh and membership-edit regressions in owned databases.
//! Run `cargo test platform_session_refresh_tests -- --ignored --nocapture`.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::Utc;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::routes;
use crate::state::AppState;
use crate::util::{hash, jwt};

const SECRET: &str = "platform-refresh-owned-database-tests";

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Result<(StatusCode, Value)> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let bytes = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            serde_json::to_vec(&body)?
        }
        None => Vec::new(),
    };
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        app.clone().oneshot(builder.body(Body::from(bytes))?),
    )
    .await
    .map_err(|_| anyhow!("platform session API timed out"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

async fn refresh(app: &Router, token: &str, extra: &str) -> Result<(StatusCode, Value)> {
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        app.clone().oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/oauth/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "grant_type=refresh_token&refresh_token={token}{extra}"
                )))?,
        ),
    )
    .await
    .map_err(|_| anyhow!("platform refresh API timed out"))??;
    let status = response.status();
    if status == StatusCode::OK {
        ensure!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .is_some_and(|v| v == "no-store"),
            "platform refresh credentials must not be cached"
        );
    }
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| anyhow!("missing credential response field {key}"))
}

struct Harness {
    app: Router,
    user: Uuid,
    tenant: Uuid,
    root: Uuid,
    child: Uuid,
    other: Uuid,
    application: Uuid,
}

impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let tenant = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform) VALUES('刷新测试平台','admin-tenant',true) RETURNING id"
        ).fetch_one(pool).await?;
        let root = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,name) VALUES($1,'根组织') RETURNING id",
        )
        .bind(tenant)
        .fetch_one(pool)
        .await?;
        let child = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,parent_id,name) VALUES($1,$2,'下级组织') RETURNING id"
        ).bind(tenant).bind(root).fetch_one(pool).await?;
        let other = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,parent_id,name) VALUES($1,$2,'其他组织') RETURNING id"
        ).bind(tenant).bind(root).fetch_one(pool).await?;
        let user = sqlx::query_scalar(
            "INSERT INTO users(username,password_hash,is_super_admin) VALUES('refresh-test-admin','unused',true) RETURNING id"
        ).fetch_one(pool).await?;
        for organization in [root, child, other] {
            sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
                .bind(user)
                .bind(organization)
                .execute(pool)
                .await?;
        }
        let application = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('系统管理','system') RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        sqlx::query("INSERT INTO menus(application_id,title,permission,type) VALUES($1,'用户管理','system:user','MENU')")
            .bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO menus(application_id,title,permission,type,path) VALUES($1,'会话管理','system:oauth','MENU','/system/oauth/tokens')").bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)").bind(tenant).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(root).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) SELECT $1,id FROM menus WHERE application_id=$2").bind(tenant).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) SELECT $1,id FROM menus WHERE application_id=$2").bind(root).bind(application).execute(pool).await?;
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/refresh-tests".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
            ("SESSION_TTL_HOURS".into(), "1".into()),
            ("REFRESH_TOKEN_TTL_SECONDS".into(), "86400".into()),
        ]))?;
        let app = routes::api_router()
            .nest("/oauth", routes::oauth2::oauth_router())
            .with_state(AppState::new(pool.clone(), Arc::new(config)));
        Ok(Self {
            app,
            user,
            tenant,
            root,
            child,
            other,
            application,
        })
    }

    async fn context(&self, org: Uuid) -> Result<Value> {
        let ticket = jwt::sign(
            &jwt::Claims {
                sub: self.user.to_string(),
                typ: "credential".into(),
                tenant: None,
                org: None,
                cid: None,
                scope: None,
                sid: None,
                ver: Some(0),
                aud: None,
                jti: None,
                iat: Utc::now().timestamp(),
                exp: Utc::now().timestamp() + 300,
            },
            SECRET,
        )?;
        let (status, value) = call(
            &self.app,
            Method::POST,
            "/api/auth/context",
            Some(&ticket),
            Some(json!({"organizationId": org})),
        )
        .await?;
        ensure!(
            status == StatusCode::OK,
            "context issuance failed: {status}"
        );
        Ok(value)
    }

    async fn profile_status(&self, token: &str) -> Result<StatusCode> {
        Ok(call(
            &self.app,
            Method::GET,
            "/api/auth/profile",
            Some(token),
            None,
        )
        .await?
        .0)
    }

    async fn replace_memberships(
        &self,
        token: &str,
        orgs: &[Uuid],
        department: Option<Uuid>,
    ) -> Result<()> {
        let items: Vec<Value> = orgs.iter().map(|org| json!({
            "organizationId": org, "departmentId": if *org == self.root { department } else { None },
            "position": "已更新职位",
        })).collect();
        let (status, _) = call(
            &self.app,
            Method::PUT,
            &format!("/api/users/{}/memberships", self.user),
            Some(token),
            Some(json!({"items":items})),
        )
        .await?;
        ensure!(
            status == StatusCode::OK,
            "membership update failed: {status}"
        );
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL database creation privileges"]
async fn expired_access_rotates_context_bound_refresh_once() -> Result<()> {
    let db = Database::create(false).await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let context = h.context(h.child).await?;
        let access = text(&context, "accessToken")?;
        let original = text(&context, "refreshToken")?;
        let initial = jwt::verify(access, SECRET)?;
        let refresh_deadline: chrono::DateTime<Utc> = sqlx::query_scalar(
            "SELECT expires_at FROM platform_session_refresh_tokens WHERE token_hash=$1",
        ).bind(hash::sha256_hex(original)).fetch_one(&db.pool).await?;
        let access_lifetime = initial.exp - initial.iat;
        let refresh_lifetime = refresh_deadline.timestamp() - initial.iat;
        // PostgreSQL and the signing host may have a small clock difference.
        // Assert the actual stored/signed deadlines and configured ceilings.
        ensure!(
            context["expiresIn"].as_i64() == Some(access_lifetime)
                && access_lifetime > 0 && access_lifetime <= 3600
                && context["refreshExpiresIn"].as_i64() == Some(refresh_lifetime)
                && refresh_lifetime > access_lifetime && refresh_lifetime <= 86400,
            "context deadlines or configured access/refresh ceilings differ"
        );
        let persisted_hash: String =
            sqlx::query_scalar("SELECT token_hash FROM platform_session_refresh_tokens")
                .fetch_one(&db.pool)
                .await?;
        ensure!(
            persisted_hash == hash::sha256_hex(original) && persisted_hash != original,
            "refresh persistence must contain only digest"
        );
        let mut expired = initial.clone();
        expired.exp = Utc::now().timestamp() - 120;
        ensure!(
            h.profile_status(&jwt::sign(&expired, SECRET)?).await? == StatusCode::UNAUTHORIZED,
            "expired access was accepted"
        );
        let (status, renewed) = refresh(&h.app, original, "").await?;
        ensure!(
            status == StatusCode::OK && renewed["token_type"] == "Bearer",
            "refresh failed"
        );
        let current = jwt::verify(text(&renewed, "access_token")?, SECRET)?;
        ensure!(
            current.sub == initial.sub
                && current.tenant == initial.tenant
                && current.org == initial.org
                && current.sid == initial.sid
                && current.ver == initial.ver
                && current.typ == "session",
            "refresh changed authenticated identity or context"
        );
        ensure!(
            current.tenant == Some(h.tenant.to_string())
                && current.org == Some(h.child.to_string()),
            "refresh returned wrong tenant/organization"
        );
        ensure!(
            text(&renewed, "refresh_token")? != original,
            "refresh token did not rotate"
        );
        ensure!(
            h.profile_status(text(&renewed, "access_token")?).await? == StatusCode::OK,
            "refreshed access cannot use protected APIs"
        );
        let (status, replay) = refresh(&h.app, original, "").await?;
        ensure!(
            status == StatusCode::BAD_REQUEST && replay["error"] == "invalid_grant",
            "refresh replay accepted"
        );
        let next = text(&renewed, "refresh_token")?;
        let (left, right) = tokio::join!(refresh(&h.app, next, ""), refresh(&h.app, next, ""));
        let statuses = [left?.0, right?.0];
        ensure!(
            statuses.iter().filter(|s| **s == StatusCode::OK).count() == 1
                && statuses
                    .iter()
                    .filter(|s| **s == StatusCode::BAD_REQUEST)
                    .count()
                    == 1,
            "concurrent refresh consumed same token twice"
        );
        ensure!(
            refresh(&h.app, access, "").await?.0 == StatusCode::UNAUTHORIZED,
            "access JWT was accepted as public refresh grant"
        );
        let separate = h.context(h.root).await?;
        let platform = text(&separate, "refreshToken")?;
        for extra in [
            "&client_id=browser",
            "&client_secret=secret",
            "&scope=profile",
        ] {
            ensure!(
                refresh(&h.app, platform, extra).await?.0 == StatusCode::BAD_REQUEST,
                "platform refresh accepted OAuth client/scope conversion"
            );
        }
        ensure!(
            h.profile_status(platform).await? == StatusCode::UNAUTHORIZED,
            "refresh token granted direct business API access"
        );
        let (status, _) = call(
            &h.app,
            Method::POST,
            "/api/auth/context",
            Some(platform),
            Some(json!({"organizationId":h.other})),
        )
        .await?;
        ensure!(
            status == StatusCode::UNAUTHORIZED,
            "refresh token used to switch context"
        );
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL database creation privileges"]
async fn membership_edits_preserve_retained_sessions_and_data_grants() -> Result<()> {
    let db = Database::create(false).await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let root = h.context(h.root).await?;
        let child = h.context(h.child).await?;
        let other = h.context(h.other).await?;
        let membership: Uuid = sqlx::query_scalar("SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2")
            .bind(h.user).bind(h.root).fetch_one(&db.pool).await?;
        sqlx::query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,'records','read','ORG')")
            .bind(membership).bind(h.application).execute(&db.pool).await?;
        let department: Uuid = sqlx::query_scalar("INSERT INTO departments(organization_id,name) VALUES($1,'新部门') RETURNING id")
            .bind(h.root).fetch_one(&db.pool).await?;
        h.replace_memberships(text(&root,"accessToken")?, &[h.root,h.child], Some(department)).await?;
        for retained in [&root, &child] {
            ensure!(h.profile_status(text(retained,"accessToken")?).await? == StatusCode::OK,
                "retained membership session was revoked by editing");
            ensure!(refresh(&h.app,text(retained,"refreshToken")?,"").await?.0 == StatusCode::OK,
                "retained membership refresh was revoked by editing");
        }
        let kept: (Uuid, Option<Uuid>, String) = sqlx::query_as("SELECT id,department_id,position FROM memberships WHERE user_id=$1 AND organization_id=$2")
            .bind(h.user).bind(h.root).fetch_one(&db.pool).await?;
        ensure!(kept == (membership,Some(department),"已更新职位".into()), "retained membership ID or edits lost");
        let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM membership_data_grants WHERE membership_id=$1")
            .bind(membership).fetch_one(&db.pool).await?;
        ensure!(grants == 1, "member data grants were deleted by retained membership update");
        ensure!(h.profile_status(text(&other,"accessToken")?).await? == StatusCode::UNAUTHORIZED,
            "removed membership session is still active");
        ensure!(refresh(&h.app,text(&other,"refreshToken")?,"").await?.0 == StatusCode::BAD_REQUEST,
            "removed membership could refresh");
        h.replace_memberships(text(&root,"accessToken")?, &[h.root,h.child,h.other], Some(department)).await?;
        ensure!(refresh(&h.app,text(&other,"refreshToken")?,"").await?.0 == StatusCode::BAD_REQUEST,
            "readding membership revived revoked refresh");
        h.replace_memberships(text(&root,"accessToken")?, &[], None).await?;
        ensure!(h.profile_status(text(&root,"accessToken")?).await? == StatusCode::UNAUTHORIZED,
            "removed current login context was not revoked");
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL database creation privileges"]
async fn revocation_disabled_ancestry_and_absolute_expiry_block_refresh() -> Result<()> {
    let db = Database::create(false).await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let logout = h.context(h.root).await?;
        ensure!(call(&h.app, Method::POST, "/api/auth/logout", Some(text(&logout,"accessToken")?), None).await?.0 == StatusCode::OK,
            "logout failed");
        ensure!(refresh(&h.app,text(&logout,"refreshToken")?,"").await?.0 == StatusCode::BAD_REQUEST,
            "logout refresh remains active");
        let disabled = h.context(h.child).await?;
        sqlx::query("UPDATE organizations SET status='DISABLED' WHERE id=$1").bind(h.root).execute(&db.pool).await?;
        ensure!(refresh(&h.app,text(&disabled,"refreshToken")?,"").await?.0 == StatusCode::BAD_REQUEST,
            "disabled ancestor can refresh");
        sqlx::query("UPDATE organizations SET status='ENABLED' WHERE id=$1").bind(h.root).execute(&db.pool).await?;
        ensure!(refresh(&h.app,text(&disabled,"refreshToken")?,"").await?.0 == StatusCode::BAD_REQUEST,
            "reenabling ancestor revived old refresh");
        let expiry = h.context(h.root).await?;
        let digest = hash::sha256_hex(text(&expiry,"refreshToken")?);
        sqlx::query("UPDATE platform_session_refresh_tokens SET issued_at=now()-interval '2 hours',expires_at=now()-interval '1 hour' WHERE token_hash=$1")
            .bind(digest).execute(&db.pool).await?;
        ensure!(refresh(&h.app,text(&expiry,"refreshToken")?,"").await?.0 == StatusCode::BAD_REQUEST,
            "expired refresh token accepted");
        let bounded = h.context(h.root).await?;
        sqlx::query("UPDATE tenants SET expires_at=now()+interval '60 seconds' WHERE id=$1")
            .bind(h.tenant).execute(&db.pool).await?;
        let (status, response) = refresh(&h.app,text(&bounded,"refreshToken")?,"").await?;
        ensure!(status == StatusCode::OK && response["expires_in"].as_i64().is_some_and(|s| s<=60)
            && response["refresh_expires_in"].as_i64().is_some_and(|s| s<=60), "refresh ignored shortened tenant validity");
        let revised = h.context(h.root).await?;
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1")
            .bind(h.user).execute(&db.pool).await?;
        ensure!(refresh(&h.app,text(&revised,"refreshToken")?,"").await?.0 == StatusCode::BAD_REQUEST,
            "revoked account version could refresh");
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL database creation privileges"]
async fn platform_device_listing_includes_current_device_and_preserves_personal_scope() -> Result<()>
{
    let db = Database::create(false).await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        sqlx::query("INSERT INTO menus(application_id,title,permission,type) VALUES($1,'设备与会话','system:oauth','MENU')")
            .bind(h.application).execute(&db.pool).await?;
        let ordinary_tenant: Uuid = sqlx::query_scalar("INSERT INTO tenants(name,code) VALUES('设备范围租户','device-tenant') RETURNING id")
            .fetch_one(&db.pool).await?;
        let ordinary_org: Uuid = sqlx::query_scalar("INSERT INTO organizations(tenant_id,name) VALUES($1,'设备范围组织') RETURNING id")
            .bind(ordinary_tenant).fetch_one(&db.pool).await?;
        let other_user: Uuid = sqlx::query_scalar("INSERT INTO users(username,password_hash) VALUES('another-device-user','unused') RETURNING id")
            .fetch_one(&db.pool).await?;
        for user in [h.user, other_user] {
            sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
                .bind(user).bind(ordinary_org).execute(&db.pool).await?;
        }
        let other_device: Uuid = sqlx::query_scalar("INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at,ip,user_agent) VALUES($1,$2,$3,now()+interval '1 hour','127.0.0.8','Other test device') RETURNING id")
            .bind(other_user).bind(ordinary_tenant).bind(ordinary_org).fetch_one(&db.pool).await?;
        let platform = h.context(h.root).await?;
        let ordinary = h.context(ordinary_org).await?;
        let current_claims = jwt::verify(text(&platform,"accessToken")?, SECRET)?;
        let current_id = current_claims.sid.ok_or_else(|| anyhow!("current session missing id"))?;
        let (status, listing) = call(&h.app, Method::GET, "/api/oauth/tokens/sessions", Some(text(&platform,"accessToken")?), None).await?;
        ensure!(status == StatusCode::OK && listing["sessions"].as_array().is_some_and(|items| items.is_empty()),
            "platform device-only login must preserve separate empty OAuth aggregate");
        let devices = listing["platformSessions"].as_array().ok_or_else(|| anyhow!("platform devices missing"))?;
        ensure!(devices.len()==3 && devices.iter().filter(|s| s["isCurrent"]==true).count()==1,
            "platform listing missing devices or current-session marker is ambiguous");
        let current = devices.iter().find(|s| s["id"]==current_id).ok_or_else(|| anyhow!("current device missing"))?;
        ensure!(current["userId"]==json!(h.user) && current["username"]=="refresh-test-admin"
            && current["tenantId"]==json!(h.tenant) && current["organizationId"]==json!(h.root)
            && current["active"]==true && current["isCurrent"]==true,
            "current login device metadata is incorrect");
        ensure!(devices.iter().any(|s| s["id"]==json!(other_device) && s["userAgent"]=="Other test device"
            && s["ip"]=="127.0.0.8" && s["isCurrent"]==false), "administrator cannot see other login device");
        for device in devices {
            for forbidden in ["token", "tokenHash", "accessToken", "refreshToken", "passwordHash"] {
                ensure!(device.get(forbidden).is_none(), "device listing leaked credential material");
            }
        }
        for path in ["/api/oauth/tokens/sessions", "/api/oauth/tokens", "/api/oauth/device-codes"] {
            ensure!(call(&h.app, Method::GET, path, Some(text(&ordinary,"accessToken")?), None).await?.0==StatusCode::FORBIDDEN,
                "ordinary tenant entered global device/OAuth management");
        }
        let (status, personal) = call(&h.app, Method::GET, "/api/auth/my-sessions", Some(text(&ordinary,"accessToken")?), None).await?;
        let personal_devices = personal["platformSessions"].as_array().ok_or_else(|| anyhow!("personal devices missing"))?;
        ensure!(status==StatusCode::OK && personal_devices.len()==2
            && personal_devices.iter().all(|s| s["userId"]==json!(h.user))
            && personal_devices.iter().filter(|s| s["isCurrent"]==true).count()==1,
            "ordinary personal device listing omitted own devices or leaked another person's devices");
        let (status, renewed) = refresh(&h.app,text(&ordinary,"refreshToken")?,"").await?;
        ensure!(status==StatusCode::OK, "ordinary device refresh failed");
        let (_, after_refresh) = call(&h.app, Method::GET, "/api/auth/my-sessions", Some(text(&renewed,"access_token")?), None).await?;
        ensure!(after_refresh["platformSessions"].as_array().is_some_and(|items| items.len()==2),
            "refresh created duplicate or removed current device records");
        Ok(())
    }.await;
    db.finish(assertions).await
}
