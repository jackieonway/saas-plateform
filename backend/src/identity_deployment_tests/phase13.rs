//! Canonical employee source fixtures and the narrow cross-owner deny fence.
use super::*;
use crate::context_selection_http::authority::{Client, SourceFuture, SourcePort, SourceProof};
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use saas_platform_backend::context_selection::{ContractVersion, Id, Intent, Source, Version};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

pub(super) async fn console_source(pool: &PgPool, user: Uuid) -> Result<OidcSource> {
    user_source(pool, user, "EMPLOYEE").await
}

pub(super) async fn user_source(
    pool: &PgPool,
    user: Uuid,
    member_type: &str,
) -> Result<OidcSource> {
    let purpose = match member_type {
        "EMPLOYEE" => "PLATFORM_CONSOLE",
        "CUSTOMER" => "CUSTOMER_PORTAL",
        _ => {
            return Err(anyhow!(
                "owned source fixture requires one human member type"
            ))
        }
    };
    let app = uuid7::new_uuid();
    let client = uuid7::new_uuid();
    let resource = uuid7::new_uuid();
    let scopes = [uuid7::new_uuid(), uuid7::new_uuid()];
    let group = uuid7::new_uuid();
    let consent = uuid7::new_uuid();
    let sid = uuid7::new_uuid();
    let request = uuid7::new_uuid();
    let context = uuid7::new_uuid();
    let grant = uuid7::new_uuid();
    let digest = hash::sha256_hex(&uuid7::new_uuid().to_string());
    sqlx::query("INSERT INTO auth_applications(application_id,ownership,status,display_name,purpose,approved_scopes) VALUES($1,'FIRST_PARTY','APPROVED','Owned canonical BFF',$2,ARRAY['openid','profile'])")
        .bind(app).bind(purpose).execute(pool).await?;
    sqlx::query("INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,allowed_member_types,grant_types,token_endpoint_auth_method,redirect_uris,status) VALUES($1,$2,'Owned canonical BFF',$3,'test','CONFIDENTIAL','WEB_BFF','USER','PLATFORM_CONTEXT',ARRAY[$4],ARRAY['authorization_code','refresh_token'],'client_secret_basic',ARRAY['https://phase13.example.invalid/auth/callback'],'ENABLED')")
        .bind(client).bind(client.to_string()).bind(app).bind(member_type).execute(pool).await?;
    let audience = format!("https://phase13.example.invalid/userinfo/{resource}");
    // The BFF proof deliberately requires the one canonical userinfo resource.
    let actual_resource: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM auth_resources WHERE resource_key='userinfo'")
            .fetch_optional(pool)
            .await?;
    let resource = if let Some(existing) = actual_resource {
        existing
    } else {
        sqlx::query("INSERT INTO auth_resources(id,resource_key,audience,status) VALUES($1,'userinfo',$2,'ACTIVE')")
            .bind(resource).bind(&audience).execute(pool).await?;
        resource
    };
    let audience: String = sqlx::query_scalar("SELECT audience FROM auth_resources WHERE id=$1")
        .bind(resource)
        .fetch_one(pool)
        .await?;
    let mut actual_scopes = Vec::new();
    for (scope, name) in scopes.into_iter().zip(["openid", "profile"]) {
        let existing: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM auth_scopes WHERE resource_id=$1 AND scope_name=$2")
                .bind(resource)
                .bind(name)
                .fetch_optional(pool)
                .await?;
        let scope = if let Some(existing) = existing {
            existing
        } else {
            sqlx::query("INSERT INTO auth_scopes(id,resource_id,scope_name,status) VALUES($1,$2,$3,'ACTIVE')").bind(scope).bind(resource).bind(name).execute(pool).await?;
            scope
        };
        actual_scopes.push(scope);
        sqlx::query(
            "INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) VALUES($1,$2,$3)",
        )
        .bind(client)
        .bind(resource)
        .bind(scope)
        .execute(pool)
        .await?;
    }
    sqlx::query("INSERT INTO auth_consent_groups(id,application_id,environment,display_name,purpose,context_mode,status) VALUES($1,$2,'test','Owned BFF consent',$3,'PLATFORM_CONTEXT','ACTIVE')")
        .bind(group).bind(app).bind(purpose).execute(pool).await?;
    sqlx::query("INSERT INTO auth_consent_group_clients(group_id,client_id,application_id,environment,context_mode) VALUES($1,$2,$3,'test','PLATFORM_CONTEXT')")
        .bind(group).bind(client).bind(app).execute(pool).await?;
    sqlx::query("INSERT INTO auth_consents(id,user_id,group_id,status,authorized_at) VALUES($1,$2,$3,'ACTIVE',clock_timestamp())")
        .bind(consent).bind(user).bind(group).execute(pool).await?;
    for scope in &actual_scopes {
        sqlx::query(
            "INSERT INTO auth_consent_scopes(consent_id,resource_id,scope_id) VALUES($1,$2,$3)",
        )
        .bind(consent)
        .bind(resource)
        .bind(scope)
        .execute(pool)
        .await?;
    }
    sqlx::query("INSERT INTO sso_sessions(id,user_id,token_hash,csrf_hash,account_version,authenticated_at,expires_at) VALUES($1,$2,$3,$4,3,date_trunc('second',clock_timestamp()),clock_timestamp()+interval '1 hour')")
        .bind(sid).bind(user).bind(hash::sha256_hex(&sid.to_string())).bind(hash::sha256_hex(&format!("csrf-{sid}"))).execute(pool).await?;
    sqlx::query("INSERT INTO auth_authorization_requests(id,client_id,user_id,source_session_id,account_version,auth_time,client_policy_version,application_projection_version,redirect_uri,state_digest,state_value,nonce_digest,nonce_value,code_challenge,code_challenge_method,scope_ids,browser_digest,csrf_digest,status,expires_at,consumed_at) SELECT $1,$2,$3,$4,3,s.authenticated_at,1,1,'https://phase13.example.invalid/auth/callback',repeat('a',64),'owned-state',repeat('b',64),'owned-nonce',repeat('c',43),'S256',$5,repeat('d',64),repeat('e',64),'CONSUMED',clock_timestamp()+interval '5 minutes',clock_timestamp() FROM sso_sessions s WHERE s.id=$4")
        .bind(request).bind(client).bind(user).bind(sid).bind(&actual_scopes).execute(pool).await?;
    let member: Uuid = sqlx::query_scalar(
        "SELECT id FROM memberships WHERE user_id=$1 AND member_type=$2 ORDER BY id LIMIT 1",
    )
    .bind(user)
    .bind(member_type)
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO auth_context_interactions(transaction_id,request_hash,client_id,client_policy_version,application_id,application_projection_version,redirect_uri,state_digest,nonce_digest,code_challenge,intent,authorization_request_id,source_digest,selected_membership_id,credential_ciphertext,selection_expires_at,expires_at) VALUES($1,$2,$3,1,$4,1,'https://phase13.example.invalid/auth/callback',repeat('a',64),repeat('b',64),repeat('c',43),'{\"kind\":\"LOGIN\"}',$5,$6,$7,'owned-fixture-ciphertext',clock_timestamp()+interval '2 minutes',clock_timestamp()+interval '5 minutes')")
        .bind(context).bind(hash::sha256_hex(&context.to_string())).bind(client).bind(app).bind(request).bind(&digest).bind(member).execute(pool).await?;
    sqlx::query("INSERT INTO auth_grants(id,client_id,subject_mode,user_id,consent_id,consent_generation,consent_group_version,source_session_id,context_mode,context_ref,account_version,client_policy_version,application_projection_version,scope_ids,audiences,auth_time,expires_at) SELECT $1,$2,'USER',$3,$4,1,1,$5,'PLATFORM_CONTEXT',$6,3,1,1,$7,ARRAY[$8],s.authenticated_at,clock_timestamp()+interval '30 minutes' FROM sso_sessions s WHERE s.id=$5")
        .bind(grant).bind(client).bind(user).bind(consent).bind(sid).bind(context).bind(&actual_scopes).bind(&audience).execute(pool).await?;
    sqlx::query("UPDATE auth_context_interactions SET grant_id=$2,delivered_at=clock_timestamp(),credential_ciphertext=NULL WHERE transaction_id=$1")
        .bind(context).bind(grant).execute(pool).await?;
    for scope in &actual_scopes {
        sqlx::query(
            "INSERT INTO auth_grant_scopes(grant_id,resource_id,scope_id) VALUES($1,$2,$3)",
        )
        .bind(grant)
        .bind(resource)
        .bind(scope)
        .execute(pool)
        .await?;
    }
    sqlx::query("INSERT INTO auth_refresh_families(id,grant_id,absolute_expires_at,idle_expires_at) VALUES($1,$2,clock_timestamp()+interval '30 minutes',clock_timestamp()+interval '20 minutes')")
        .bind(uuid7::new_uuid()).bind(grant).execute(pool).await?;
    let access = uuid7::new_uuid();
    sqlx::query("INSERT INTO auth_access_tokens(id,grant_id,token_digest,expires_at) VALUES($1,$2,$3,clock_timestamp()+interval '5 minutes')").bind(access).bind(grant).bind(&digest).execute(pool).await?;
    for scope in &actual_scopes {
        sqlx::query(
            "INSERT INTO auth_access_token_scopes(token_id,resource_id,scope_id) VALUES($1,$2,$3)",
        )
        .bind(access)
        .bind(resource)
        .bind(scope)
        .execute(pool)
        .await?;
    }
    let (auth_time, expires): (i64, DateTime<Utc>) = sqlx::query_as("SELECT floor(extract(epoch FROM g.auth_time))::bigint,LEAST(tok.expires_at,g.expires_at,s.expires_at) FROM auth_access_tokens tok JOIN auth_grants g ON g.id=tok.grant_id JOIN sso_sessions s ON s.id=g.source_session_id WHERE tok.id=$1").bind(access).fetch_one(pool).await?;
    Ok(OidcSource {
        client,
        grant,
        sid,
        digest,
        auth_time,
        expires,
    })
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase13_canonical_employee_source_rejects_all_original_authority_changes() -> Result<()> {
    let mut db = Database::create().await?;
    let result = async {
        let f = Fixture::create(&db.pool).await?;
        let source = console_source(&db.pool, f.user).await?;
        db.provision().await?;
        let platform = db.platform()?;
        let query = "SELECT bff_auth_proof_active($1,$2,$3,3,$4,1,$5)";
        let active = || sqlx::query_scalar::<_, bool>(query).bind(f.user).bind(source.sid).bind(source.grant).bind(source.client).bind(ISSUER).fetch_one(platform);
        ensure!(active().await?, "fresh canonical employee authority is inactive");
        for (sid, issuer) in [(uuid7::new_uuid(), ISSUER), (source.sid, "https://foreign.example.invalid")] {
            let foreign: bool = sqlx::query_scalar(query).bind(f.user).bind(sid).bind(source.grant).bind(source.client).bind(issuer).fetch_one(platform).await?;
            ensure!(!foreign, "canonical employee source accepted a different original session or issuer");
        }
        denied(platform, "SELECT * FROM auth_grants", "BFF projection grants no raw auth SELECT").await?;
        denied(db.identity()?, "SELECT * FROM memberships", "BFF projection grants no identity membership SELECT").await?;
        let member: Uuid = sqlx::query_scalar("SELECT selected_membership_id FROM auth_context_interactions WHERE grant_id=$1").bind(source.grant).fetch_one(&db.pool).await?;
        for (candidate, expected) in [(member, true), (uuid7::new_uuid(), false)] {
            let matched: bool = sqlx::query_scalar("SELECT bff_context_member_matches($1,$2)").bind(source.grant).bind(candidate).fetch_one(platform).await?;
            ensure!(matched == expected, "local employee context did not retain the grant's original selected member");
        }
        assert_definer_owner_privileges(platform, "public.bff_context_member_matches(uuid,uuid)", &db.role("identity_owner"), "public.auth_grants").await?;
        // A fresh transaction for each mutation proves each independent fence.
        for mutation in [
            format!("UPDATE auth_clients SET policy_version=policy_version+1 WHERE id='{}'",source.client),
            format!("UPDATE auth_clients SET status='DISABLED' WHERE id='{}'",source.client),
            format!("UPDATE auth_clients SET allowed_member_types=ARRAY['CUSTOMER'] WHERE id='{}'",source.client),
            format!("UPDATE auth_applications SET projection_version=projection_version+1 WHERE application_id=(SELECT application_id FROM auth_clients WHERE id='{}')",source.client),
            format!("UPDATE auth_applications SET status='SUSPENDED' WHERE application_id=(SELECT application_id FROM auth_clients WHERE id='{}')",source.client),
            format!("UPDATE auth_applications SET purpose='CUSTOMER_PORTAL' WHERE application_id=(SELECT application_id FROM auth_clients WHERE id='{}')",source.client),
            format!("UPDATE auth_consents SET generation=generation+1 WHERE id=(SELECT consent_id FROM auth_grants WHERE id='{}')",source.grant),
            format!("UPDATE auth_consents SET status='REVOKED',revoked_at=clock_timestamp() WHERE id=(SELECT consent_id FROM auth_grants WHERE id='{}')",source.grant),
            format!("UPDATE auth_consent_groups SET version=version+1 WHERE id=(SELECT group_id FROM auth_consents WHERE id=(SELECT consent_id FROM auth_grants WHERE id='{}'))",source.grant),
            format!("DELETE FROM auth_consent_group_clients WHERE client_id='{}'",source.client),
            format!("DELETE FROM auth_client_permissions WHERE client_id='{}'",source.client),
            format!("UPDATE auth_client_permissions SET expires_at=clock_timestamp()-interval '1 second' WHERE client_id='{}'",source.client),
            format!("DELETE FROM auth_consent_scopes WHERE consent_id=(SELECT consent_id FROM auth_grants WHERE id='{}')",source.grant),
            format!("UPDATE auth_scopes SET status='SUSPENDED' WHERE id=ANY((SELECT scope_ids FROM auth_grants WHERE id='{}')::uuid[])",source.grant),
            format!("UPDATE auth_resources SET status='SUSPENDED' WHERE id IN(SELECT resource_id FROM auth_grant_scopes WHERE grant_id='{}')",source.grant),
            format!("UPDATE auth_refresh_families SET revoked_at=clock_timestamp() WHERE grant_id='{}'",source.grant),
            format!("UPDATE auth_refresh_families SET created_at=clock_timestamp()-interval '2 hours',idle_expires_at=clock_timestamp()-interval '1 second' WHERE grant_id='{}'",source.grant),
            format!("UPDATE auth_authorization_requests SET status='AUTHORIZED' WHERE id=(SELECT authorization_request_id FROM auth_context_interactions WHERE grant_id='{}')",source.grant),
            format!("UPDATE auth_grants SET revoked_at=clock_timestamp() WHERE id='{}'",source.grant),
            format!("UPDATE sso_sessions SET revoked_at=clock_timestamp() WHERE id='{}'",source.sid),
            format!("UPDATE users SET session_version=session_version+1 WHERE id='{}'",f.user),
        ] {
            let mut tx = db.pool.begin().await?;
            sqlx::query(&mutation).execute(&mut *tx).await?;
            let valid: bool = sqlx::query_scalar(query).bind(f.user).bind(source.sid).bind(source.grant).bind(source.client).bind(ISSUER).fetch_one(&mut *tx).await?;
            ensure!(!valid, "canonical source fence failed for mutation: {mutation}");
            tx.rollback().await?;
            ensure!(active().await?, "rolling back one authority mutation changed the fixture");
        }
        Ok(())
    }.await;
    db.finish(result).await
}

struct SelectionSource(SourceProof);

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase13_real_identity_source_service_lock_fences_revocation_without_runtime_update(
) -> Result<()> {
    let mut db = Database::create().await?;
    let result = async {
        let service = uuid7::new_uuid();
        let client = uuid7::new_uuid();
        let digest = hash::sha256_hex(&"b".repeat(64));
        sqlx::query("INSERT INTO identity_service_clients(id,secret_hash,purpose,status,allowed_auth_clients,created_at) VALUES($1,$2,'CONTEXT_SELECTION_SOURCE','ENABLED',ARRAY[$3],clock_timestamp())").bind(service).bind(&digest).bind(client).execute(&db.pool).await?;
        db.provision().await?;
        let identity = db.identity()?;
        let query = "SELECT identity_context_source_service_active($1,$2,$3)";
        denied(identity,&format!("UPDATE identity_service_clients SET status='DISABLED' WHERE id='{service}'"),"identity runtime cannot update source-service credentials").await?;
        denied(identity,&format!("SELECT id FROM identity_service_clients WHERE id='{service}' FOR SHARE"),"identity runtime cannot use a raw credential-row lock").await?;
        denied(db.platform()?,&format!("SELECT identity_context_source_service_active('{service}',repeat('a',64),'{client}')"),"source-service lock is identity-only").await?;
        assert_definer_owner_privileges(identity,"public.identity_context_source_service_active(uuid,text,uuid)",&db.role("identity_owner"),"public.identity_service_clients").await?;
        for (candidate, hash, expected) in [(client,digest.clone(),true),(uuid7::new_uuid(),digest.clone(),false),(client,"a".repeat(64),false)] {
            let active: bool = sqlx::query_scalar(query).bind(service).bind(hash).bind(candidate).fetch_one(identity).await?;
            ensure!(active==expected,"source-service owner lock crossed its original digest or client allowlist");
        }
        let mut lock = identity.begin().await?;
        let active: bool = sqlx::query_scalar(query).bind(service).bind(&digest).bind(client).fetch_one(&mut *lock).await?;
        ensure!(active,"identity LOGIN runtime could not hold its source-service owner lock");
        let mut revoke = db.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='100ms'").execute(&mut *revoke).await?;
        let blocked = sqlx::query("UPDATE identity_service_clients SET status='DISABLED' WHERE id=$1").bind(service).execute(&mut *revoke).await;
        ensure!(matches!(blocked,Err(sqlx::Error::Database(ref error)) if error.code().as_deref()==Some("55P03")),"credential revocation crossed an uncommitted source-proof transaction");
        revoke.rollback().await?;lock.commit().await?;
        // A waiting verifier must observe a revocation committed while its row
        // lock was blocked; the original authenticated secret cannot repair it.
        let mut owner = db.pool.begin().await?;
        sqlx::query("UPDATE identity_service_clients SET status='DISABLED' WHERE id=$1").bind(service).execute(&mut *owner).await?;
        let mut connection = identity.acquire().await?;
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *connection).await?;
        let expected_digest = digest.clone();
        let verifier = tokio::spawn(async move { sqlx::query_scalar::<_,bool>(query).bind(service).bind(expected_digest).bind(client).fetch_one(&mut *connection).await });
        let mut waited = false;
        for _ in 0..100 {
            waited = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')").bind(pid).fetch_one(&db.pool).await?;
            if waited { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        owner.commit().await?;
        let current = tokio::time::timeout(std::time::Duration::from_secs(3),verifier).await???;
        ensure!(waited && !current,"source-service lock reused pre-wait credential state after revocation");
        db.verify().await?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[derive(Clone)]
struct TargetFixture {
    paths: Arc<std::sync::Mutex<Vec<String>>>,
    mode: Arc<std::sync::atomic::AtomicU8>,
    application: Uuid,
    user: Uuid,
    service: Uuid,
}

async fn source_target_reply(
    axum::extract::State(fixture): axum::extract::State<TargetFixture>,
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
    axum::Json(request): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    let path = uri.path();
    fixture.paths.lock().unwrap().push(path.to_owned());
    let expected_auth = format!(
        "Basic {}",
        STANDARD.encode(format!("{}:{}", fixture.service, "b".repeat(64)))
    );
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(expected_auth.as_str()) {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(json!({"error":"fixture_service_auth"})),
        );
    }
    let canonical = path == "/internal/identity/auth/context-selection/source";
    let mode = fixture.mode.load(std::sync::atomic::Ordering::SeqCst);
    if canonical && mode == 1 {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({"error":"fixture_source_revoked"})),
        );
    }
    let registry = if canonical && mode != 2 {
        "AUTH"
    } else {
        "OIDC"
    };
    (
        StatusCode::OK,
        axum::Json(json!({
            "contractVersion":"context-selection/1","transactionId":uuid7::new_uuid(),"intent":{"kind":"LOGIN"},
            "source":{"issuer":ISSUER,"userId":fixture.user,"accountVersion":"3","globalSessionId":uuid7::new_uuid(),"interactionId":uuid7::new_uuid(),"authTime":Utc::now()-chrono::Duration::seconds(1)},
            "client":{"id":request["expectedClientId"],"policyVersion":"1","purpose":"PLATFORM_CONSOLE","allowedMemberTypes":["EMPLOYEE"],"redirectUri":"https://phase13.example.invalid/auth/callback","clientRegistry":registry,"subjectMode":"USER","contextMode":"PLATFORM_CONTEXT","applicationId":fixture.application},
            "sourceExpiresAt":Utc::now()+chrono::Duration::minutes(5),"interactionExpiresAt":Utc::now()+chrono::Duration::minutes(5)
        })),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase13_source_http_uses_registered_registry_without_legacy_fallback() -> Result<()> {
    let db = Database::create().await?;
    let fixture = TargetFixture {
        paths: Arc::default(),
        mode: Arc::new(std::sync::atomic::AtomicU8::new(0)),
        application: uuid7::new_uuid(),
        user: uuid7::new_uuid(),
        service: uuid7::new_uuid(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let app = axum::Router::new()
        .fallback(source_target_reply)
        .with_state(fixture.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let result = async {
        let canonical = uuid7::new_uuid();
        let unregistered = uuid7::new_uuid();
        sqlx::query("INSERT INTO applications(id,name,code,base_path,status,allowed_member_types) VALUES($1,'Owned transport app','source-target-app','/source-target','ENABLED',ARRAY['EMPLOYEE'])").bind(fixture.application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO context_selection_registrations(client_id,client_registry,client_policy_version,member_type,application_id,application_policy_version) VALUES($1,'AUTH',1,'EMPLOYEE',$2,1)").bind(canonical).bind(fixture.application).execute(&db.pool).await?;
        let mut config = crate::config::Config::from_values(&Default::default())?;
        config.oidc_issuer=Some(ISSUER.into());config.identity_service_url=Some(origin);
        config.identity_http_client=Some(reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(2)).redirect(reqwest::redirect::Policy::none()).build()?);
        config.context_source_service_client_id=Some(fixture.service.to_string());config.context_source_service_client_secret=Some("b".repeat(64));
        let authority = crate::context_selection_http::authority::configured(&config,&db.pool);
        let first = authority.verify(&"a".repeat(64),Id::new(canonical)?,None).await;
        if first.is_err() { eprintln!("source_target_fixture reached_paths={:?}",fixture.paths.lock().unwrap()); }
        first.context("canonical source-proof HTTP target")?;
        authority.verify(&uuid7::new_uuid().to_string(),Id::new(canonical)?,Some(&"c".repeat(64))).await.context("canonical access-source HTTP target")?;
        ensure!(authority.verify(&"a".repeat(64),Id::new(unregistered)?,None).await.is_err(),"unregistered client reached an authority fallback");
        fixture.mode.store(1,std::sync::atomic::Ordering::SeqCst);
        ensure!(authority.verify(&"a".repeat(64),Id::new(canonical)?,None).await.is_err(),"canonical source revocation was replaced by a legacy response");
        fixture.mode.store(2,std::sync::atomic::Ordering::SeqCst);
        ensure!(authority.verify(&"a".repeat(64),Id::new(canonical)?,None).await.is_err(),"canonical target accepted a legacy registry proof");
        sqlx::query("UPDATE context_selection_registrations SET enabled=false WHERE client_id=$1").bind(canonical).execute(&db.pool).await?;
        ensure!(authority.verify(&"a".repeat(64),Id::new(canonical)?,None).await.is_err(),"disabled registration reached its upstream");
        ensure!(*fixture.paths.lock().unwrap()==[
            "/internal/identity/auth/context-selection/source","/internal/identity/auth/context-selection/source",
            "/internal/identity/auth/context-selection/source",
            "/internal/identity/auth/context-selection/source"],"source transport used an unexpected target or retried a legacy path");
        Ok(())
    }.await;
    let _ = stop.send(());
    let joined = tokio::time::timeout(std::time::Duration::from_secs(3), server).await;
    let cleanup = match joined {
        Ok(Ok(Ok(()))) => Ok(()),
        _ => Err(anyhow!("owned source transport listener did not stop")),
    };
    db.finish(result.and(cleanup)).await
}

impl SourcePort for SelectionSource {
    fn verify<'a>(
        &'a self,
        source: &'a str,
        client: Id,
        _access: Option<&'a str>,
    ) -> SourceFuture<'a> {
        Box::pin(async move {
            if source != "a".repeat(64) || client != self.0.client.id {
                return Err(crate::error::AppError::unauthorized(
                    "owned role fixture source mismatch",
                ));
            }
            Ok(self.0.clone())
        })
    }
}

async fn role_choices(
    app: &axum::Router,
    authorization: &str,
    client: Uuid,
) -> Result<(StatusCode, Value)> {
    let response = app.clone().oneshot(Request::post("/internal/platform/context-selection/choices")
        .header("authorization", authorization).header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({"contractVersion":"context-selection/1","sourceProof":"a".repeat(64),"expectedClientId":client}))?))?).await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), 65536).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase13_real_platform_role_authenticates_context_http_without_reading_identity_credentials(
) -> Result<()> {
    let mut db = Database::create().await?;
    let result = async {
        let f = Fixture::create(&db.pool).await?;
        let source = console_source(&db.pool, f.user).await?;
        let service = uuid7::new_uuid();
        let secret = "b".repeat(64);
        let digest = hash::sha256_hex(&secret);
        let application: Uuid = sqlx::query_scalar("SELECT application_id FROM auth_clients WHERE id=$1").bind(source.client).fetch_one(&db.pool).await?;
        sqlx::query("INSERT INTO applications(id,name,code,base_path,status,allowed_member_types) VALUES($1,'Owned role HTTP app','role-http-app','/role-http','ENABLED',ARRAY['EMPLOYEE'])").bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)").bind(f.tenant).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(f.org).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO context_selection_services(id,secret_hash,purpose,allowed_auth_clients,status) VALUES($1,$2,'CONTEXT_SELECTION_AUTHORITY',ARRAY[$3],'ENABLED')").bind(service).bind(&digest).bind(source.client).execute(&db.pool).await?;
        sqlx::query("INSERT INTO context_selection_registrations(client_id,client_registry,client_policy_version,member_type,application_id,application_policy_version) VALUES($1,'AUTH',1,'EMPLOYEE',$2,1)").bind(source.client).bind(application).execute(&db.pool).await?;
        db.provision().await?;
        let platform = db.platform()?;
        denied(platform,"SELECT secret_digest FROM auth_client_credentials","context authority has no raw identity credential access").await?;
        let mut config = crate::config::Config::from_values(&Default::default())?;
        config.oidc_issuer = Some(ISSUER.into());
        let mut state = crate::state::AppState::new(platform.clone(), Arc::new(config));
        let to_id = |value| Id::new(value).unwrap();
        state.context_selection_authority = Arc::new(SelectionSource(SourceProof {
            contract_version: ContractVersion::V1,transaction_id:to_id(uuid7::new_uuid()),intent:Intent::Login {},
            source:Source {issuer:ISSUER.into(),user_id:to_id(f.user),account_version:Version::new(3).unwrap(),global_session_id:to_id(source.sid),interaction_id:to_id(uuid7::new_uuid()),auth_time:chrono::DateTime::from_timestamp(source.auth_time,0).unwrap()},
            client:Client {id:to_id(source.client),policy_version:Version::new(1).unwrap(),purpose:"PLATFORM_CONSOLE".into(),allowed_member_types:vec!["EMPLOYEE".into()],redirect_uri:"https://phase13.example.invalid/auth/callback".into(),client_registry:Some("AUTH".into()),subject_mode:Some("USER".into()),context_mode:Some("PLATFORM_CONTEXT".into()),application_id:Some(to_id(application))},
            source_expires_at:Utc::now()+chrono::Duration::minutes(5),interaction_expires_at:Utc::now()+chrono::Duration::minutes(5),
        }));
        let app = crate::context_selection_http::routes().with_state(state);
        let authorization = format!("Basic {}", STANDARD.encode(format!("{service}:{secret}")));
        let (status, body) = role_choices(&app,&authorization,source.client).await?;
        ensure!(status==StatusCode::OK && body["members"].as_array().is_some_and(|members| members.len()==1),"real platform role could not authenticate/revalidate canonical context HTTP: {status} {body}");
        // A reused credential must fail through the same private HTTP boundary,
        // while the caller still has no table or secret projection privilege.
        let credential = uuid7::new_uuid();
        sqlx::query("INSERT INTO auth_client_credentials(id,client_id,credential_version,secret_digest) VALUES($1,$2,1,$3)").bind(credential).bind(source.client).bind(&digest).execute(db.identity()?).await?;
        let (status, _) = role_choices(&app,&authorization,source.client).await?;
        ensure!(status==StatusCode::FORBIDDEN,"context service reused a unified client credential");
        let in_use: bool = sqlx::query_scalar("SELECT identity_credential_digest_in_use($1)").bind(&digest).fetch_one(platform).await?;
        ensure!(in_use,"narrow credential reuse projection missed the registered client");
        assert_definer_owner_privileges(platform,"public.identity_credential_digest_in_use(text)",&db.role("identity_owner"),"public.auth_client_credentials").await?;
        sqlx::query("DELETE FROM auth_client_credentials WHERE id=$1").bind(credential).execute(db.identity()?).await?;
        ensure!(role_choices(&app,&authorization,source.client).await?.0==StatusCode::OK,"independent service credential did not recover after removing the reused fixture");
        db.verify().await?;
        Ok(())
    }.await;
    db.finish(result).await
}
