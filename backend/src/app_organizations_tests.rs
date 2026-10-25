//! HTTP regressions on explicitly configured, loopback-only disposable databases.
use crate::{
    config::Config,
    entity_creation_tests::Database,
    state::AppState,
    util::{hash, jwt},
};
use anyhow::{ensure, Result};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{collections::HashMap, sync::Arc};
use tower::ServiceExt;
use uuid::Uuid;

const SECRET: &str = "owned-store-organization-tests-only";

struct Harness {
    pool: PgPool,
    router: Router,
    application: Uuid,
    hq: Uuid,
    store: Uuid,
    other: Uuid,
    sibling: Uuid,
    foreign: Uuid,
    membership: Uuid,
    menu: Uuid,
    token: String,
    policy: Value,
}

fn router(pool: &PgPool, policy: &Value) -> Result<Router> {
    let config = Config::from_values(&HashMap::from([
        (
            "DATABASE_URL".into(),
            "postgres://unused/owned-store-organizations".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("DATA_GRANT_POLICY".into(), policy.to_string()),
    ]))?;
    Ok(crate::routes::api_router().with_state(AppState::new(pool.clone(), Arc::new(config))))
}

async fn org(
    pool: &PgPool,
    tenant: Uuid,
    parent: Option<Uuid>,
    name: &str,
    kind: &str,
) -> Result<Uuid> {
    Ok(sqlx::query_scalar("INSERT INTO organizations(tenant_id,parent_id,name,organization_type,code,contact_name,contact_phone,contact_email,id) VALUES($1,$2,$3,$4,$3,'Store contact','12345678','store@example.test',$5) RETURNING id")
        .bind(tenant).bind(parent).bind(name).bind(kind).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?)
}

impl Harness {
    async fn new(pool: &PgPool) -> Result<Self> {
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(code,name,id) VALUES('stores-tenant','Stores tenant',$1) RETURNING id",
        )
        .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
        .await?;
        let foreign_tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(code,name,id) VALUES('foreign-tenant','Foreign tenant',$1) RETURNING id",
        )
        .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
        .await?;
        let root = org(pool, tenant, None, "Root", "COMPANY").await?;
        let hq = org(pool, tenant, Some(root), "HQ", "BRANCH").await?;
        let store = org(pool, tenant, Some(hq), "Store-A", "STORE").await?;
        let other = org(pool, tenant, Some(hq), "Other", "COMPANY").await?;
        let sibling = org(pool, tenant, Some(root), "Sibling", "STORE").await?;
        let foreign = org(pool, foreign_tenant, None, "Foreign", "STORE").await?;
        let application:Uuid=sqlx::query_scalar("INSERT INTO applications(code,name,base_path,entry_url,id) VALUES('stores','Stores','/micro/stores','https://stores.example.test/index.html',$1) RETURNING id").bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        sqlx::query(
            "INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2),($3,$2)",
        )
        .bind(tenant)
        .bind(application)
        .bind(foreign_tenant)
        .execute(pool)
        .await?;
        for target in [hq, store, other, sibling, foreign] {
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(target).bind(application).execute(pool).await?;
        }
        let user:Uuid=sqlx::query_scalar("INSERT INTO users(username,password_hash,is_super_admin,id) VALUES('hq-reader','unusable-test-hash',true,$1) RETURNING id").bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let membership: Uuid = sqlx::query_scalar(
            "INSERT INTO memberships(user_id,organization_id,id) VALUES($1,$2,$3) RETURNING id",
        )
        .bind(user)
        .bind(hq)
        .bind(crate::util::uuid7::new_uuid())
        .fetch_one(pool)
        .await?;
        let menu:Uuid=sqlx::query_scalar("INSERT INTO menus(application_id,type,title,permission,id) VALUES($1,'BUTTON','Read','store:read',$2) RETURNING id").bind(application).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
            .bind(hq)
            .bind(menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type,id) VALUES($1,$2,'store','read','ORG_SUBTREE',$3)").bind(membership).bind(application).bind(crate::util::uuid7::new_uuid()).execute(pool).await?;
        let session = crate::util::uuid7::new_uuid();
        sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) \
         SELECT $1,$2,$3,$4,now()+interval '1 hour',u.session_version,m.id,m.member_type,m.identity_version,'urn:saas:legacy-platform','platform-console-legacy',c.access_policy_version \
         FROM users u JOIN memberships m ON m.user_id=u.id AND m.organization_id=$4 AND m.member_type='EMPLOYEE' \
         JOIN oauth_clients c ON c.client_id='platform-console-legacy' WHERE u.id=$2").bind(session).bind(user).bind(tenant).bind(hq).execute(pool).await?;
        sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_type,client_secret_hash,application_id,id) VALUES('stores-directory','Stores directory','CONFIDENTIAL',$1,$2,$3)").bind(hash::sha256_hex("directory-secret")).bind(application).bind(crate::util::uuid7::new_uuid()).execute(pool).await?;
        let now = Utc::now().timestamp();
        let jti = crate::util::uuid7::new_uuid();
        let token = jwt::sign(
            &jwt::Claims {
                sub: user.to_string(),
                typ: "access".into(),
                tenant: Some(tenant.to_string()),
                org: Some(hq.to_string()),
                cid: Some("stores-directory".into()),
                scope: Some(String::new()),
                sid: None,
                ver: None,
                aud: Some("stores".into()),
                jti: Some(jti.to_string()),
                exp: now + 300,
                iat: now,
            },
            SECRET,
        )?;
        sqlx::query("INSERT INTO oauth_tokens(token_hash,token_type,user_id,client_id,tenant_id,organization_id,scope,expires_at,token_use,audience,jti,application_id,platform_session_id,id,account_version,membership_id,member_type,identity_version,client_policy_version,application_policy_version,origin_issuer,origin_client_ref,origin_client_policy_version,application_service_client_ref) \
            SELECT $1,'access',$2,c.client_id,$3,$4,'',now()+interval '5 minutes','app','stores',$5,$6,$7,$8,s.account_version,s.membership_id,s.member_type,s.identity_version,c.access_policy_version,a.access_policy_version,s.origin_issuer,s.origin_client_ref,s.origin_client_policy_version,c.client_id \
            FROM platform_sessions s JOIN oauth_clients c ON c.client_id='stores-directory' JOIN applications a ON a.id=$6 \
            WHERE s.id=$7 AND s.user_id=$2 AND s.tenant_id=$3 AND s.organization_id=$4 AND platform_session_active(s.id)")
            .bind(hash::sha256_hex(&token)).bind(user).bind(tenant).bind(hq).bind(jti).bind(application).bind(session).bind(crate::util::uuid7::new_uuid()).execute(pool).await?;
        let policy = json!({"applications":[{"applicationId":application,"capabilities":[{"resource":"store","action":"read","scopes":["SELF","ORG_SUBTREE"]}],"organizationLookups":[{"resource":"store","action":"read","organizationType":"STORE"}]}],"managementScopes":[]});
        Ok(Self {
            pool: pool.clone(),
            router: router(pool, &policy)?,
            application,
            hq,
            store,
            other,
            sibling,
            foreign,
            membership,
            menu,
            token,
            policy,
        })
    }
    fn path(&self, id: Option<Uuid>) -> String {
        format!(
            "/api/apps/stores/organizations{}?resource=store&action=read",
            id.map(|id| format!("/{id}")).unwrap_or_default()
        )
    }
    async fn get(&self, path: &str) -> Result<(StatusCode, Value)> {
        get(&self.router, &self.token, path).await
    }
}

async fn get(app: &Router, token: &str, path: &str) -> Result<(StatusCode, Value)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())?,
        )
        .await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

async fn database() -> Result<Database> {
    let url = std::env::var("TEST_DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("explicit loopback TEST_DATABASE_URL is required"))?;
    let parsed =
        reqwest::Url::parse(&url).map_err(|_| anyhow::anyhow!("invalid isolated database URL"))?;
    ensure!(
        matches!(
            parsed.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        ),
        "organization tests require a loopback database server"
    );
    Database::create().await
}

#[tokio::test]
#[ignore = "requires explicit loopback TEST_DATABASE_URL and disposable database privileges"]
async fn store_directory_is_purpose_bound_tenant_bounded_and_fresh_after_revocation() -> Result<()>
{
    let database = database().await?;
    let result=async {
        let h=Harness::new(&database.pool).await?;
        let before=Utc::now();
        let (status,list)=h.get(&h.path(None)).await?;
        ensure!(status==StatusCode::OK && list["total"]==1,"HQ directory did not return exactly its STORE child");
        ensure!(list["records"][0]["id"]==h.store.to_string(),"wrong directory target");
        let (status,one)=h.get(&h.path(Some(h.store))).await?;
        ensure!(status==StatusCode::OK && one["organizationType"]=="STORE" && one["status"]=="ENABLED","valid STORE denied");
        ensure!(one["contextVersion"]==list["contextVersion"] && one["revision"].as_str().is_some_and(|v|v.len()==64),"proof identity missing");
        let expiry=chrono::DateTime::parse_from_rfc3339(one["expiresAt"].as_str().ok_or_else(||anyhow::anyhow!("missing expiry"))?)?;
        ensure!(expiry>before && expiry<=Utc::now()+chrono::Duration::seconds(3),"witness has an unbounded lifetime");
        ensure!(one.get("authorityExpiresAt").is_none() && one.get("updatedAt").is_none(),"internal metadata leaked");
        sqlx::query("UPDATE organizations SET contact_phone='87654321',updated_at=now() WHERE id=$1").bind(h.store).execute(&h.pool).await?;
        let (_,changed)=h.get(&h.path(Some(h.store))).await?;
        ensure!(changed["contextVersion"]==one["contextVersion"] && changed["revision"]!=one["revision"] && changed["contactPhone"]=="87654321","target metadata did not receive its own fresh revision");
        let target_expiry=Utc::now()+chrono::Duration::seconds(2);
        sqlx::query("UPDATE organizations SET expires_at=$2 WHERE id=$1").bind(h.store).bind(target_expiry).execute(&h.pool).await?;
        let (status,limited)=h.get(&h.path(Some(h.store))).await?;
        let proof_expiry=chrono::DateTime::parse_from_rfc3339(limited["expiresAt"].as_str().ok_or_else(||anyhow::anyhow!("missing bounded expiry"))?)?;
        ensure!(status==StatusCode::OK && proof_expiry<=target_expiry,"metadata proof outlived target authority");
        sqlx::query("UPDATE organizations SET expires_at=NULL WHERE id=$1").bind(h.store).execute(&h.pool).await?;
        for id in [h.other,h.sibling,h.foreign] {ensure!(h.get(&h.path(Some(id))).await?.0==StatusCode::NOT_FOUND,"out-of-scope organization exposed");}
        ensure!(h.get(&(h.path(None)+"&tenantId="+&h.foreign.to_string())).await?.0==StatusCode::BAD_REQUEST,"caller tenant override accepted");
        ensure!(h.get(&(h.path(None)+"&size=51")).await?.0==StatusCode::BAD_REQUEST,"oversized directory page accepted");
        ensure!(h.get(&(h.path(Some(h.store))+"&q=Store")).await?.0==StatusCode::BAD_REQUEST,"single metadata accepted search parameters");
        let mut denied=h.policy.clone();denied["applications"][0]["organizationLookups"]=json!([]);
        ensure!(get(&router(&h.pool,&denied)?,&h.token,&h.path(None)).await?.0==StatusCode::BAD_REQUEST,"ordinary capability opened the directory");
        sqlx::query("UPDATE membership_data_grants SET scope_type='SELF' WHERE membership_id=$1").bind(h.membership).execute(&h.pool).await?;
        ensure!(h.get(&h.path(None)).await?.1["total"]==0,"SELF expanded to STORE descendants");
        let (status,metadata)=h.get(&h.path(Some(h.store))).await?;
        ensure!(status==StatusCode::OK && metadata.get("ownerUserId").is_none(),"known SELF metadata was denied or claimed business ownership");
        sqlx::query("UPDATE membership_data_grants SET scope_type='ORG_SUBTREE' WHERE membership_id=$1").bind(h.membership).execute(&h.pool).await?;
        for mutation in ["organization_type='OTHER'","status='DISABLED'","expires_at=now()-interval '1 second'"] {
            sqlx::query(&format!("UPDATE organizations SET {mutation} WHERE id=$1")).bind(h.store).execute(&h.pool).await?;
            ensure!(h.get(&h.path(Some(h.store))).await?.0==StatusCode::NOT_FOUND,"invalidated STORE remained visible");
            sqlx::query("UPDATE organizations SET organization_type='STORE',status='ENABLED',expires_at=NULL WHERE id=$1").bind(h.store).execute(&h.pool).await?;
        }
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(h.store).bind(h.application).execute(&h.pool).await?;
        ensure!(h.get(&h.path(Some(h.store))).await?.0==StatusCode::NOT_FOUND,"revoked target application grant remained visible");
        sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1 AND menu_id=$2").bind(h.hq).bind(h.menu).execute(&h.pool).await?;
        ensure!(h.get(&h.path(None)).await?.0==StatusCode::FORBIDDEN,"revoked operation permission remained usable");
        Ok(())
    }.await;
    database.finish(result).await
}
