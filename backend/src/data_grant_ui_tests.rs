//! Actual Vite UI -> real Axum API -> owned PostgreSQL grant-management loop.
//! Invoked separately from the ordinary PostgreSQL suites in the browser gate.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::{middleware, Router};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::state::AppState;
use crate::util::jwt;

const SECRET: &str = "owned-phase15-browser-session-signing-only";
const CLIENT_SECRET: &str = "owned-phase15-browser-client-secret";
const CODE: &str = "work-orders";
const RESULT_MARKER: &str = "PHASE15_LIVE_RESULT ";

fn config(policy: &Value) -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/owned-live-browser".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("DATA_GRANT_POLICY".into(), policy.to_string()),
    ]))
}

async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<String> {
    let version = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let sid: Uuid = sqlx::query_scalar("INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id")
        .bind(user).bind(tenant).bind(org).fetch_one(pool).await?;
    let now = chrono::Utc::now().timestamp();
    jwt::sign(
        &jwt::Claims {
            sub: user.to_string(),
            typ: "session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            sid: Some(sid.to_string()),
            ver: Some(version),
            cid: None,
            scope: None,
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )
}

async fn member(pool: &PgPool, username: &str, org: Uuid, active: bool) -> Result<(Uuid, Uuid)> {
    let user: Uuid = sqlx::query_scalar("INSERT INTO users(username,real_name,password_hash,status) VALUES($1,$1,'unused-browser-fixture-password',$2) RETURNING id")
        .bind(username).bind(if active {"ENABLED"} else {"DISABLED"}).fetch_one(pool).await?;
    let membership = sqlx::query_scalar(
        "INSERT INTO memberships(user_id,organization_id) VALUES($1,$2) RETURNING id",
    )
    .bind(user)
    .bind(org)
    .fetch_one(pool)
    .await?;
    Ok((user, membership))
}

async fn role(pool: &PgPool, tenant: Uuid, org: Uuid, user: Uuid, menus: &[Uuid]) -> Result<()> {
    let role: Uuid = sqlx::query_scalar(
        "INSERT INTO roles(tenant_id,organization_id,name) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(org)
    .bind(format!("owned-phase15-role-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
        .bind(user)
        .bind(role)
        .bind(org)
        .execute(pool)
        .await?;
    for menu in menus {
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(role)
            .bind(menu)
            .execute(pool)
            .await?;
    }
    Ok(())
}

fn target(tenant: Uuid, org: Uuid, user: Uuid, membership: Uuid, application: Uuid) -> Value {
    json!({"tenantId":tenant,"organizationId":org,"userId":user,"membershipId":membership,"applicationId":application})
}

struct Fixture {
    app: Router,
    wire: Value,
    tenant: Uuid,
    org: Uuid,
    user: Uuid,
    membership: Uuid,
    application: Uuid,
    administrator: Uuid,
    secrets: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct BootstrapRow {
    tenant_id: Uuid,
    organization_id: Uuid,
    user_id: Uuid,
    application_id: Uuid,
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        crate::seed::initialize(
            pool,
            &config(&json!({"applications":[],"managementScopes":[]}))?,
        )
        .await?;
        let BootstrapRow {tenant_id:platform,organization_id:root,user_id:administrator,application_id:system} = sqlx::query_as(
            "SELECT t.id AS tenant_id,o.id AS organization_id,u.id AS user_id,a.id AS application_id FROM tenants t JOIN organizations o ON o.tenant_id=t.id AND o.parent_id IS NULL JOIN users u ON u.username='admin' JOIN applications a ON a.code='system' WHERE t.is_platform")
            .fetch_one(pool).await?;
        let read_menu: Uuid = sqlx::query_scalar("SELECT id FROM menus WHERE application_id=$1 AND permission='system:data-grant:read' AND type='BUTTON'")
            .bind(system).fetch_one(pool).await?;
        let write_menu: Uuid = sqlx::query_scalar("SELECT id FROM menus WHERE application_id=$1 AND permission='system:data-grant:write' AND type='BUTTON'")
            .bind(system).fetch_one(pool).await?;
        let page_menu: Uuid = sqlx::query_scalar("SELECT id FROM menus WHERE application_id=$1 AND path='/system/data-grants' AND type='MENU'")
            .bind(system).fetch_one(pool).await?;
        let user_menu: Uuid = sqlx::query_scalar("SELECT id FROM menus WHERE application_id=$1 AND permission='system:user' AND type='MENU'")
            .bind(system).fetch_one(pool).await?;
        let tenant: Uuid = sqlx::query_scalar("INSERT INTO tenants(name,code) VALUES('Phase15 business tenant','phase15-business') RETURNING id")
            .fetch_one(pool).await?;
        let business_root: Uuid = sqlx::query_scalar("INSERT INTO organizations(tenant_id,name) VALUES($1,'Phase15 business root') RETURNING id")
            .bind(tenant).fetch_one(pool).await?;
        let org: Uuid = sqlx::query_scalar("INSERT INTO organizations(tenant_id,parent_id,name) VALUES($1,$2,'Phase15 business organization') RETURNING id")
            .bind(tenant).bind(business_root).fetch_one(pool).await?;
        let outside: Uuid = sqlx::query_scalar("INSERT INTO organizations(tenant_id,parent_id,name) VALUES($1,$2,'Phase15 outside organization') RETURNING id")
            .bind(tenant).bind(business_root).fetch_one(pool).await?;
        let application: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code,base_path,entry_type) VALUES('Phase15 Ticket App',$1,'/micro/phase15','HTML') RETURNING id")
            .bind(CODE).fetch_one(pool).await?;
        let read_ticket_menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,title,type,permission,visible) VALUES($1,'Read ticket','BUTTON','ticket:read',false) RETURNING id")
            .bind(application).fetch_one(pool).await?;
        let update_menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,title,type,permission,visible) VALUES($1,'Update ticket','BUTTON','ticket:update',false) RETURNING id")
            .bind(application).fetch_one(pool).await?;
        for app in [system, application] {
            sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
                .bind(tenant)
                .bind(app)
                .execute(pool)
                .await?;
            for organization in [org, outside] {
                sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                    .bind(organization).bind(app).execute(pool).await?;
            }
        }
        for menu in [
            read_menu,
            write_menu,
            page_menu,
            read_ticket_menu,
            update_menu,
        ] {
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(tenant)
                .bind(menu)
                .execute(pool)
                .await?;
            for organization in [org, outside] {
                sqlx::query(
                    "INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)",
                )
                .bind(organization)
                .bind(menu)
                .execute(pool)
                .await?;
            }
        }
        sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_secret_hash,application_id,client_type,grant_types,scopes) VALUES('phase15-owned-client','Owned live browser client',$1,$2,'CONFIDENTIAL','password,refresh_token','profile')")
            .bind(crate::util::hash::sha256_hex(CLIENT_SECRET)).bind(application).execute(pool).await?;
        let (user, membership) = member(pool, "phase15-business-user", org, true).await?;
        role(pool, tenant, org, user, &[read_ticket_menu, update_menu]).await?;
        let (alternate_user, alternate_membership) =
            member(pool, "phase15-assignee", org, true).await?;
        let (inactive, inactive_membership) =
            member(pool, "phase15-inactive-member", org, false).await?;
        let (outside_user, outside_membership) =
            member(pool, "phase15-outside-member", outside, true).await?;
        let (org_admin, _) = member(pool, "phase15-org-admin", org, true).await?;
        role(
            pool,
            tenant,
            org,
            org_admin,
            &[read_menu, write_menu, page_menu],
        )
        .await?;
        let (reader, _) = member(pool, "phase15-root-reader", root, true).await?;
        role(pool, platform, root, reader, &[read_menu, page_menu]).await?;
        let (no_read, _) = member(pool, "phase15-root-broad-admin", root, true).await?;
        role(pool, platform, root, no_read, &[user_menu]).await?;
        let policy = json!({"applications":[{"applicationId":application,"capabilities":[
            {"resource":"ticket","action":"read","scopes":["SELF","ORG"]},
            {"resource":"ticket","action":"update","scopes":["ORG"]}]}],
            "managementScopes":[{"tenantId":tenant,"organizationId":org,"includeDescendants":false,"read":true,"write":true}]});
        let state = AppState::new(pool.clone(), Arc::new(config(&policy)?));
        let app = crate::routes::api_router()
            .nest("/oauth", crate::routes::oauth2::oauth_router())
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::observability::observe,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::normalize_client_ip,
            ))
            .with_state(state);
        let admin_session = session(pool, administrator, platform, root).await?;
        let reader_session = session(pool, reader, platform, root).await?;
        let org_admin_session = session(pool, org_admin, tenant, org).await?;
        let no_read_session = session(pool, no_read, platform, root).await?;
        let business_session = session(pool, user, tenant, org).await?;
        let wire = json!({"schemaVersion":1,"application":{"id":application,"name":"Phase15 Ticket App","code":CODE},
            "systemApplicationId":system,"target":target(tenant,org,user,membership,application),
            "alternateTarget":target(tenant,org,alternate_user,alternate_membership,application),
            "inactiveTarget":target(tenant,org,inactive,inactive_membership,application),
            "outsideTarget":target(tenant,outside,outside_user,outside_membership,application),
            "targetUsername":"phase15-business-user","targetDisplayName":"phase15-business-user",
            "sessions":{"admin":admin_session,"reader":reader_session,"organizationAdmin":org_admin_session,
                "noRead":no_read_session,"business":business_session}});
        let secrets = vec![
            SECRET.into(),
            CLIENT_SECRET.into(),
            crate::util::hash::sha256_hex(CLIENT_SECRET),
            admin_session,
            reader_session,
            org_admin_session,
            no_read_session,
            business_session,
        ];
        Ok(Self {
            app,
            wire,
            tenant,
            org,
            user,
            membership,
            application,
            administrator,
            secrets,
        })
    }
}

#[derive(sqlx::FromRow)]
struct AuditRow {
    user_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    application_id: Option<Uuid>,
    request_id: Option<Uuid>,
    detail: String,
}

struct BusinessDatabase {
    pool: PgPool,
    admin: PgPool,
    name: String,
    url: String,
    password: String,
}

impl BusinessDatabase {
    async fn create() -> Result<Self> {
        let source = std::env::var("TEST_DATABASE_URL")
            .map_err(|_| anyhow!("isolated database configuration missing"))?;
        let options = PgConnectOptions::from_str(&source)
            .map_err(|_| anyhow!("invalid isolated database configuration"))?;
        let mut url = reqwest::Url::parse(&source)
            .map_err(|_| anyhow!("invalid isolated database configuration"))?;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("owned business database administrator unavailable"))?;
        let name = format!("phase15_work_orders_{}", Uuid::new_v4().simple());
        if sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .is_err()
        {
            admin.close().await;
            return Err(anyhow!(
                "unable to create owned independent business database"
            ));
        }
        let pool = match PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                admin.close().await;
                return Err(anyhow!("owned independent business database unavailable"));
            }
        };
        // No platform migrations run here. The existing business service owns
        // its checksummed schema, in a distinct, initially empty database.
        url.set_path(&format!("/{name}"));
        let password = url.password().unwrap_or_default().to_owned();
        Ok(Self {
            pool,
            admin,
            name,
            url: url.to_string(),
            password,
        })
    }

    async fn finish(self, assertions: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("phase15_work_orders_")
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "refusing cleanup outside the owned business database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await
            .is_ok();
        let closed = closed
            || tokio::time::timeout(Duration::from_secs(5), self.pool.close())
                .await
                .is_ok();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.admin.close()).await;
        println!(
            "PHASE15_BUSINESS_CLEANUP assertions_ok={} database_removed={} pool_closed={}",
            assertions.is_ok(),
            removed,
            closed
        );
        assertions?;
        ensure!(
            removed && closed,
            "owned independent business database cleanup failed"
        );
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires managed browser runtime and explicit owned TEST_DATABASE_URL"]
async fn browser_management_closes_grant_loop() -> Result<()> {
    ensure!(
        std::env::var("TEST_DATABASE_URL").is_ok_and(|value| !value.trim().is_empty()),
        "explicit isolated TEST_DATABASE_URL is required"
    );
    let product = std::env::var("SAAS_TEST_BROWSER").unwrap_or_else(|_| "chromium".into());
    ensure!(
        ["chromium", "chrome", "msedge"].contains(&product.as_str()),
        "unsupported required browser product"
    );
    let db = Database::create().await?;
    let result = run_browser(&db.pool, &product).await;
    db.finish(result).await
}

async fn run_browser(pool: &PgPool, product: &str) -> Result<()> {
    let mut fixture = Fixture::create(pool).await?;
    let business = BusinessDatabase::create().await?;
    fixture.secrets.push(business.url.clone());
    if !business.password.is_empty() {
        fixture.secrets.push(business.password.clone());
    }
    let result = run_live_services(pool, product, &mut fixture, &business).await;
    business.finish(result).await
}

async fn run_live_services(
    pool: &PgPool,
    product: &str,
    fixture: &mut Fixture,
    business: &BusinessDatabase,
) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    fixture.wire["apiOrigin"] = json!(format!("http://{address}"));
    let app = fixture.app.clone();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let mut server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stop_rx.await;
        })
        .await
    });
    let assertions = async {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or_else(|| anyhow!("workspace root unavailable"))?;
        let mut command = tokio::process::Command::new("node");
        command
            .current_dir(root)
            .arg("tests/phase15-data-grants-browser.mjs")
            .env_clear()
            .env("SAAS_TEST_BROWSER", product)
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in [
            "PATH",
            "Path",
            "SystemRoot",
            "SYSTEMROOT",
            "WINDIR",
            "COMSPEC",
            "TEMP",
            "TMP",
            "TMPDIR",
            "HOME",
            "USERPROFILE",
            "LOCALAPPDATA",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "DISPLAY",
            "XDG_RUNTIME_DIR",
            "PLAYWRIGHT_BROWSERS_PATH",
            "PHASE15_SCREENSHOT_PATH",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|_| anyhow!("unable to start managed Node browser driver"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("browser driver input unavailable"))?;
        let input=json!({"schemaVersion":1,"browser":fixture.wire,
            "workOrders":{"databaseUrl":business.url,"clientId":"phase15-owned-client","clientSecret":CLIENT_SECRET}});
        stdin.write_all(&serde_json::to_vec(&input)?).await?;
        stdin.shutdown().await?;
        drop(stdin);
        let output = tokio::time::timeout(Duration::from_secs(300), child.wait_with_output())
            .await
            .map_err(|_| anyhow!("live browser driver exceeded owned run deadline"))??;
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        ensure!(
            !combined.contains("postgres://") && !combined.contains("postgresql://"),
            "browser output exposed database configuration"
        );
        for secret in &fixture.secrets {
            ensure!(
                !combined.contains(secret),
                "browser output exposed fixture credential material"
            );
        }
        // Only bounded, credential-checked driver diagnostics enter the CI log.
        ensure!(
            combined.len() <= 1024 * 1024,
            "browser output exceeded diagnostic budget"
        );
        print!("{combined}");
        ensure!(output.status.success(), "actual live browser driver failed");
        let reports: Vec<Value> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix(RESULT_MARKER))
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        ensure!(
            reports.len() == 1,
            "live browser driver did not emit exactly one structured result"
        );
        let report = &reports[0];
        ensure!(
            report["schemaVersion"] == 1
                && report["product"] == product
                && report["failed"] == 0
                && report["skipped"] == 0
                && report["browserVersion"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty()),
            "live browser result is invalid"
        );
        assert_database(pool, fixture, report).await?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM work_orders")
            .fetch_one(&business.pool).await?;
        let independent:bool=sqlx::query_scalar("SELECT to_regclass('public.membership_data_grants') IS NULL")
            .fetch_one(&business.pool).await?;
        ensure!(count==3&&independent,"existing business service did not use its separate three-record database");
        Ok(())
    }
    .await;
    let _ = stop_tx.send(());
    let shutdown = tokio::time::timeout(Duration::from_secs(10), &mut server).await;
    if shutdown.is_err() {
        server.abort();
        let _ = server.await;
    }
    ensure!(
        matches!(shutdown, Ok(Ok(Ok(())))),
        "owned Axum listener failed graceful cleanup"
    );
    assertions
}

async fn assert_database(pool: &PgPool, fixture: &Fixture, report: &Value) -> Result<()> {
    let ids: Vec<Uuid> = report["requestIds"]
        .as_array()
        .ok_or_else(|| anyhow!("browser request IDs missing"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| anyhow!("invalid browser request ID"))?
                .parse()
                .map_err(|_| anyhow!("invalid browser request ID"))
        })
        .collect::<Result<_>>()?;
    ensure!(
        ids.len() == 4 && ids.iter().collect::<std::collections::HashSet<_>>().len() == 4,
        "browser must complete exactly four distinct controlled changes"
    );
    let rows: Vec<AuditRow>=sqlx::query_as("SELECT user_id,tenant_id,organization_id,application_id,request_id,detail FROM audit_logs WHERE action='DATA_GRANT_CHANGE' ORDER BY id")
        .fetch_all(pool).await?;
    ensure!(
        rows.len() == ids.len(),
        "browser writes and committed grant audit count differ"
    );
    let expected_scopes = [
        (None, Some("SELF")),
        (Some("SELF"), Some("ORG")),
        (Some("ORG"), Some("SELF")),
        (Some("SELF"), None),
    ];
    let mut grant_id: Option<String> = None;
    for (index, (row, request_id)) in rows.iter().zip(&ids).enumerate() {
        ensure!(
            row.user_id == Some(fixture.administrator)
                && row.tenant_id == Some(fixture.tenant)
                && row.organization_id == Some(fixture.org)
                && row.application_id == Some(fixture.application)
                && row.request_id == Some(*request_id),
            "browser mutation audit attribution differs"
        );
        let detail: Value = serde_json::from_str(&row.detail)?;
        ensure!(
            detail["before"]["target"] == fixture.wire["target"]
                && detail["after"]["target"] == fixture.wire["target"],
            "browser mutation audit lost exact direct-member target"
        );
        for (side, expected_scope) in [
            ("before", expected_scopes[index].0),
            ("after", expected_scopes[index].1),
        ] {
            let grants = detail[side]["grants"]
                .as_array()
                .ok_or_else(|| anyhow!("browser audit grant snapshot missing"))?;
            if let Some(scope) = expected_scope {
                ensure!(
                    grants.len() == 1
                        && grants[0]["resource"] == "ticket"
                        && grants[0]["action"] == "read"
                        && grants[0]["scope"] == scope,
                    "committed audit does not match the UI scope transition"
                );
                let id = grants[0]["id"]
                    .as_str()
                    .ok_or_else(|| anyhow!("browser audit grant UUID missing"))?;
                ensure!(
                    Uuid::parse_str(id).is_ok(),
                    "browser audit grant UUID invalid"
                );
                if let Some(original) = &grant_id {
                    ensure!(
                        original == id,
                        "browser scope update replaced stored grant UUID"
                    );
                } else {
                    grant_id = Some(id.to_owned());
                }
            } else {
                ensure!(
                    grants.is_empty(),
                    "browser revoke/initial audit grants are not empty"
                );
            }
        }
        for secret in &fixture.secrets {
            ensure!(
                !row.detail.contains(secret),
                "browser audit leaked credential material"
            );
        }
    }
    let membership: Uuid =
        sqlx::query_scalar("SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2")
            .bind(fixture.user)
            .bind(fixture.org)
            .fetch_one(pool)
            .await?;
    ensure!(
        membership == fixture.membership,
        "browser grant management replaced direct membership UUID"
    );
    let grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM membership_data_grants WHERE membership_id=$1 AND application_id=$2",
    )
    .bind(fixture.membership)
    .bind(fixture.application)
    .fetch_one(pool)
    .await?;
    ensure!(grants == 0, "browser revoke did not remove persisted grant");
    println!(
        "PHASE15_LIVE_DATABASE {}",
        json!({"auditRows":rows.len(),"requestIds":ids,
        "requestIdsMatch":true,"membershipUuidPreserved":true,"grantRevoked":true})
    );
    Ok(())
}
