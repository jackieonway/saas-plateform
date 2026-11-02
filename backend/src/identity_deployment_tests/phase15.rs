//! Internal capability and service permissions tested with real LOGIN roles.
use super::*;

async fn application(pool: &PgPool, fixture: &Fixture) -> Result<(Uuid, String)> {
    let app = uuid7::new_uuid();
    let code = format!("owned-capability-{}", app.simple());
    sqlx::query("INSERT INTO applications(id,name,code,base_path,status,allowed_member_types) VALUES($1,'Owned capability app',$2,'/owned/capability','ENABLED',ARRAY['EMPLOYEE'])").bind(app).bind(&code).execute(pool).await?;
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
        .bind(fixture.tenant)
        .bind(app)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
    )
    .bind(fixture.org)
    .bind(app)
    .execute(pool)
    .await?;
    Ok((app, code))
}
async fn capability(
    pool: &PgPool,
    session: Uuid,
    app: Uuid,
    hash: &str,
    id: Uuid,
    issued: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<()> {
    let rows=sqlx::query("INSERT INTO platform_app_capabilities(id,jti,token_hash,user_id,tenant_id,organization_id,application_id,audience,scope,platform_session_id,membership_id,member_type,identity_version,account_version,application_policy_version,origin_issuer,origin_client_ref,origin_client_policy_version,issued_at,expires_at) SELECT $1,$1,$2,s.user_id,s.tenant_id,s.organization_id,a.id,a.code,'app:access',s.id,s.membership_id,s.member_type,s.identity_version,s.account_version,a.access_policy_version,s.origin_issuer,s.origin_client_ref,s.origin_client_policy_version,$3,$4 FROM platform_sessions s JOIN applications a ON a.id=$5 WHERE s.id=$6")
        .bind(id).bind(hash).bind(issued).bind(end).bind(app).bind(session).execute(pool).await?.rows_affected();
    ensure!(
        rows == 1,
        "owned capability fixture failed to preserve its original source tuple"
    );
    Ok(())
}
async fn active(pool: &PgPool, hash: &str) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT platform_app_capability_active($1)")
            .bind(hash)
            .fetch_one(pool)
            .await?,
    )
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase15_real_roles_keep_internal_capabilities_source_bound_and_nonreviving() -> Result<()>
{
    let mut db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        bind_sso_fixture(&db.pool,&f).await?;
        let (app,_)=application(&db.pool,&f).await?;
        let (bff,client,grant,sso):(Uuid,Uuid,Uuid,Uuid)=sqlx::query_as("SELECT id,oidc_client_id,oidc_grant_id,global_session_id FROM bff_sessions WHERE selected_session_id=$1").bind(f.sessions[0]).fetch_one(&db.pool).await?;
        db.provision().await?;
        let platform=db.platform()?;let identity=db.identity()?;
        let id=uuid7::new_uuid();let hash=hash::sha256_hex(&id.to_string());
        let issued:DateTime<Utc>=sqlx::query_scalar("SELECT date_trunc('second',clock_timestamp())").fetch_one(platform).await?;
        capability(platform,f.sessions[0],app,&hash,id,issued,issued+chrono::Duration::minutes(5)).await?;
        ensure!(active(platform,&hash).await?,"fresh internal capability rejected by real runtime");
        for table in ["platform_app_capabilities","platform_resource_services","platform_resource_bindings","platform_machine_delegations"] {
            denied(identity,&format!("SELECT * FROM {table}"),"identity receives no platform resource table read").await?;
        }
        denied(identity,"SELECT platform_app_capability_active(repeat('a',64))","identity receives no internal capability verifier EXECUTE").await?;
        denied(platform,"SELECT * FROM auth_access_tokens","internal verifier grants no raw identity token access").await?;
        let retired_absent:bool=sqlx::query_scalar("SELECT to_regclass('public.oauth_clients') IS NULL AND to_regclass('public.oauth_tokens') IS NULL AND to_regclass('public.oidc_clients') IS NULL").fetch_one(platform).await?;
        ensure!(retired_absent,"retired client or token registries remain in the fresh baseline");
        for mutation in [
            format!("UPDATE platform_app_capabilities SET revoked=true WHERE token_hash='{hash}'"),
            format!("UPDATE platform_sessions SET revoked_at=clock_timestamp() WHERE id='{}'",f.sessions[0]),
            format!("UPDATE bff_sessions SET revoked_at=clock_timestamp() WHERE id='{bff}'"),
            format!("UPDATE bff_sessions SET refresh_uncertain=true WHERE id='{bff}'"),
            format!("UPDATE users SET session_version=session_version+1 WHERE id='{}'",f.user),
            format!("UPDATE memberships SET identity_version=identity_version+1 WHERE id='{}'",f.member),
            format!("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id='{}'",f.member),
            format!("UPDATE tenants SET status='DISABLED' WHERE id='{}'",f.tenant),
            format!("UPDATE organizations SET status='DISABLED' WHERE id='{}'",f.org),
            format!("UPDATE applications SET status='DISABLED' WHERE id='{app}'"),
            format!("UPDATE applications SET access_policy_version=access_policy_version+1 WHERE id='{app}'"),
            format!("DELETE FROM tenant_applications WHERE tenant_id='{}' AND application_id='{app}'",f.tenant),
            format!("DELETE FROM organization_applications WHERE organization_id='{}' AND application_id='{app}'",f.org),
            format!("UPDATE auth_clients SET policy_version=policy_version+1 WHERE id='{client}'"),
            format!("UPDATE auth_grants SET revoked_at=clock_timestamp() WHERE id='{grant}'"),
            format!("UPDATE sso_sessions SET revoked_at=clock_timestamp() WHERE id='{sso}'"),
        ] {
            let mut tx=db.pool.begin().await?;
            sqlx::query(&mutation).execute(&mut *tx).await?;
            let valid:bool=sqlx::query_scalar("SELECT platform_app_capability_active($1)").bind(&hash).fetch_one(&mut *tx).await?;
            ensure!(!valid,"internal capability survived an original authority shrink");
            tx.rollback().await?;
            ensure!(active(platform,&hash).await?,"isolated rollback changed owned capability source");
        }
        constraint_rejected(platform,&format!("UPDATE platform_app_capabilities SET account_version=account_version+1 WHERE token_hash='{hash}'"),"23514","immutable capability account binding").await?;
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(f.org).bind(app).execute(platform).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(f.org).bind(app).execute(platform).await?;
        ensure!(!active(platform,&hash).await?,"regrant resurrected a revoked original capability");
        constraint_rejected(platform,&format!("UPDATE platform_app_capabilities SET revoked=false WHERE token_hash='{hash}'"),"23514","revocation cannot be reversed").await?;
        // Expiry is sampled after a real row-lock wait, not transaction now().
        let expiring_id=uuid7::new_uuid();let expiring_hash=hash::sha256_hex(&expiring_id.to_string());
        let issued:DateTime<Utc>=sqlx::query_scalar("SELECT clock_timestamp()").fetch_one(platform).await?;
        let end=issued+chrono::Duration::milliseconds(700);
        capability(platform,f.sessions[0],app,&expiring_hash,expiring_id,issued,end).await?;
        ensure!(active(platform,&expiring_hash).await?,"short-lived capability started inactive");
        let mut owner=db.pool.begin().await?;
        sqlx::query("SELECT id FROM platform_app_capabilities WHERE id=$1 FOR UPDATE").bind(expiring_id).execute(&mut *owner).await?;
        let mut connection=platform.acquire().await?;
        let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *connection).await?;
        let hash_for_wait=expiring_hash.clone();
        let pending=tokio::spawn(async move {
            let mut tx=sqlx::Acquire::begin(&mut *connection).await?;
            sqlx::query("SELECT id FROM platform_app_capabilities WHERE token_hash=$1 FOR UPDATE").bind(&hash_for_wait).execute(&mut *tx).await?;
            let value=sqlx::query_scalar::<_,bool>("SELECT platform_app_capability_active($1)").bind(hash_for_wait).fetch_one(&mut *tx).await?;
            tx.rollback().await?;Ok::<_,sqlx::Error>(value)
        });
        let mut waited=false;
        for _ in 0..150 {
            waited=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock')").bind(pid).fetch_one(&db.pool).await?;
            let expired:bool=sqlx::query_scalar("SELECT clock_timestamp()>$1::timestamptz").bind(end).fetch_one(&db.pool).await?;
            if waited && expired {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        owner.commit().await?;
        let valid=tokio::time::timeout(Duration::from_secs(3),pending).await???;
        ensure!(waited && !valid,"expired capability accepted after its real lock wait");
        db.verify().await?;Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase15_resource_service_and_machine_scope_generations_are_monotonic() -> Result<()> {
    let mut db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        let (app,code)=application(&db.pool,&f).await?;
        let identity_service=uuid7::new_uuid();
        let identity_digest=hash::sha256_hex(&identity_service.to_string());
        sqlx::query("INSERT INTO identity_service_clients(id,secret_hash,purpose,status,created_at) VALUES($1,$2,'RESOURCE_ACCESS_PROOF','ENABLED',clock_timestamp())").bind(identity_service).bind(&identity_digest).execute(&db.pool).await?;
        db.provision().await?;
        let platform=db.platform()?;
        let identity=db.identity()?;
        for (table,role) in [("platform_resource_services","platform_runtime"),("auth_invitations","identity_runtime")] {
            sqlx::raw_sql(&format!("REVOKE UPDATE ON public.{table} FROM \"{}\"",db.role(role))).execute(&db.pool).await?;
            let incomplete_rejected=db.verify().await.is_err();
            sqlx::raw_sql(&format!("GRANT UPDATE ON public.{table} TO \"{}\"",db.role(role))).execute(&db.pool).await?;
            ensure!(incomplete_rejected,"deployment accepted an incomplete {table} DML grant");
        }
        let allowed_resources:Vec<Uuid>=sqlx::query_scalar("SELECT allowed_auth_resources FROM identity_service_clients WHERE id=$1").bind(identity_service).fetch_one(identity).await?;
        ensure!(allowed_resources.is_empty(),"resource proof service default scope is not empty");
        denied(identity,"UPDATE identity_service_clients SET allowed_auth_resources=ARRAY[]::uuid[]","identity cannot expand its own resource service scope").await?;
        let reused:bool=sqlx::query_scalar("SELECT identity_credential_digest_in_use($1)").bind(&identity_digest).fetch_one(platform).await?;
        ensure!(reused,"resource service isolation missed a private identity service digest");
        let service=uuid7::new_uuid();
        sqlx::query("INSERT INTO platform_resource_services(id,service_id,secret_hash,purpose,application_id,allowed_member_types,allowed_operations,audience) VALUES($1,$2,repeat('a',64),'APP_RESOURCE',$3,ARRAY['EMPLOYEE'],ARRAY['store:read'],$4)").bind(service).bind(format!("owned-{}",service.simple())).bind(app).bind(code).execute(platform).await?;
        sqlx::query("UPDATE platform_resource_services SET allowed_operations=ARRAY['store:read','store:write'] WHERE id=$1").bind(service).execute(platform).await?;
        sqlx::query("UPDATE platform_resource_services SET status='DISABLED' WHERE id=$1").bind(service).execute(platform).await?;
        sqlx::query("UPDATE platform_resource_services SET status='ENABLED' WHERE id=$1").bind(service).execute(platform).await?;
        let generation:i64=sqlx::query_scalar("SELECT credential_version FROM platform_resource_services WHERE id=$1").bind(service).fetch_one(platform).await?;
        ensure!(generation==4,"service policy shrink/re-enable repaired an old credential generation");
        constraint_rejected(platform,&format!("UPDATE platform_resource_services SET credential_version=1 WHERE id='{service}'"),"23514","service generation cannot decrease").await?;
        constraint_rejected(platform,&format!("INSERT INTO platform_resource_services(id,service_id,secret_hash,purpose,application_id,allowed_member_types,allowed_operations,audience) SELECT '{}','owned-duplicate',secret_hash,purpose,application_id,allowed_member_types,allowed_operations,audience FROM platform_resource_services WHERE id='{service}'",uuid7::new_uuid()),"23505","resource services cannot share a secret digest").await?;
        let delegation=uuid7::new_uuid();
        sqlx::query("INSERT INTO platform_machine_delegations(id,client_id,source_application_id,credential_version,application_id,tenant_id,organization_id,resource,action,scope_type,organization_ids) VALUES($1,$2,$3,1,$4,$5,$6,'store','read','ORG',ARRAY[$6])").bind(delegation).bind(uuid7::new_uuid()).bind(uuid7::new_uuid()).bind(app).bind(f.tenant).bind(f.org).execute(platform).await?;
        constraint_rejected(platform,&format!("UPDATE platform_machine_delegations SET organization_ids=ARRAY['{}'::uuid] WHERE id='{delegation}'",uuid7::new_uuid()),"23514","machine ORG cannot substitute an unrelated organization").await?;
        sqlx::query("UPDATE platform_machine_delegations SET status='DISABLED' WHERE id=$1").bind(delegation).execute(platform).await?;
        sqlx::query("UPDATE platform_machine_delegations SET status='ENABLED' WHERE id=$1").bind(delegation).execute(platform).await?;
        let version:i64=sqlx::query_scalar("SELECT version FROM platform_machine_delegations WHERE id=$1").bind(delegation).fetch_one(platform).await?;
        ensure!(version==3,"machine scope re-enable repaired an old approval version");
        let child=uuid7::new_uuid();
        sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Owned child scope')").bind(child).bind(f.tenant).bind(f.org).execute(platform).await?;
        let complete:bool=sqlx::query_scalar("SELECT platform_machine_scope_valid($1,$2,'ORG_SUBTREE',ARRAY[$2,$3])").bind(f.tenant).bind(f.org).bind(child).fetch_one(platform).await?;
        let incomplete:bool=sqlx::query_scalar("SELECT platform_machine_scope_valid($1,$2,'ORG_SUBTREE',ARRAY[$2])").bind(f.tenant).bind(f.org).fetch_one(platform).await?;
        ensure!(complete && !incomplete,"machine subtree did not enforce its complete current organization ceiling");
        let tenant_scope:bool=sqlx::query_scalar("SELECT platform_machine_scope_valid($1,NULL,'TENANT',ARRAY[]::uuid[])").bind(f.tenant).fetch_one(platform).await?;
        ensure!(tenant_scope,"explicit TENANT machine scope was rejected");
        sqlx::query("UPDATE tenants SET status='DISABLED' WHERE id=$1").bind(f.tenant).execute(platform).await?;
        let inactive:bool=sqlx::query_scalar("SELECT platform_machine_scope_valid($1,NULL,'TENANT',ARRAY[]::uuid[])").bind(f.tenant).fetch_one(platform).await?;
        ensure!(!inactive,"machine tenant ceiling survived tenant withdrawal");
        db.verify().await?;Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase15_consumed_context_sources_survive_expiry_and_cannot_be_repaired() -> Result<()> {
    let mut db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        let (app,_)=application(&db.pool,&f).await?;
        let client=uuid7::new_uuid();
        sqlx::query("INSERT INTO context_selection_registrations(client_id,client_registry,client_policy_version,member_type,application_id,application_policy_version) VALUES($1,'AUTH',1,'EMPLOYEE',$2,1)").bind(client).bind(app).execute(&db.pool).await?;
        db.provision().await?;
        let platform=db.platform()?;
        let consumed=uuid7::new_uuid();let unused=uuid7::new_uuid();
        for transaction in [consumed,unused] {
            let interaction=uuid7::new_uuid();
            sqlx::query("INSERT INTO context_selections(transaction_id,interaction_id,client_id,credential_hash,credential_ciphertext,selection,created_at,expires_at) VALUES($1,$2,$3,$4,'owned-expired-selection',jsonb_build_object('binding',jsonb_build_object('transactionId',$1::text,'source',jsonb_build_object('interactionId',$2::text),'registration',jsonb_build_object('clientId',$3::text,'application',jsonb_build_object('id',$5::text))))||jsonb_build_object('member',jsonb_build_object('tenantId',$6::text,'organizationId',$7::text)),clock_timestamp()-interval '5 minutes',clock_timestamp()-interval '3 minutes 1 second')")
                .bind(transaction).bind(interaction).bind(client).bind(hash::sha256_hex(&transaction.to_string())).bind(app).bind(f.tenant).bind(f.org).execute(platform).await?;
        }
        sqlx::query("UPDATE context_selections SET consumed_at=created_at+interval '1 second',credential_ciphertext=NULL,result='{}'::jsonb,result_ciphertext='owned-committed-result' WHERE transaction_id=$1").bind(consumed).execute(platform).await?;
        let removed=sqlx::query("DELETE FROM context_selections WHERE expires_at<=clock_timestamp() AND consumed_at IS NULL").execute(platform).await?.rows_affected();
        ensure!(removed==1,"expiry cleanup failed to remove only the unused selection");
        constraint_rejected(platform,&format!("DELETE FROM context_selections WHERE transaction_id='{consumed}'"),"23514","expired consumed source facts cannot be deleted by runtime").await?;
        sqlx::query("UPDATE context_selection_registrations SET enabled=false WHERE client_id=$1").bind(client).execute(platform).await?;
        let revoked:DateTime<Utc>=sqlx::query_scalar("SELECT resource_revoked_at FROM context_selections WHERE transaction_id=$1").bind(consumed).fetch_one(platform).await?;
        sqlx::query("UPDATE context_selection_registrations SET enabled=true WHERE client_id=$1").bind(client).execute(platform).await?;
        let restored:DateTime<Utc>=sqlx::query_scalar("SELECT resource_revoked_at FROM context_selections WHERE transaction_id=$1").bind(consumed).fetch_one(platform).await?;
        ensure!(revoked==restored,"RP re-enable repaired an old resource source");
        constraint_rejected(platform,&format!("UPDATE context_selections SET resource_revoked_at=NULL WHERE transaction_id='{consumed}'"),"P0001","resource revocation cannot be cleared").await?;
        let source_grant_only=uuid7::new_uuid();
        sqlx::query("INSERT INTO context_selections(transaction_id,interaction_id,client_id,credential_hash,credential_ciphertext,selection,created_at,expires_at,consumed_at,result,result_ciphertext) SELECT $1,$2,client_id,$3,NULL,jsonb_set(jsonb_set(selection,'{binding,transactionId}',to_jsonb($1::text)),'{binding,source,interactionId}',to_jsonb($2::text)),created_at,expires_at,consumed_at,result,result_ciphertext FROM context_selections WHERE transaction_id=$4")
            .bind(source_grant_only).bind(uuid7::new_uuid()).bind(hash::sha256_hex(&source_grant_only.to_string())).bind(consumed).execute(platform).await?;
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(f.org).bind(app).execute(platform).await?;
        let source_grant_revoked:DateTime<Utc>=sqlx::query_scalar("SELECT resource_revoked_at FROM context_selections WHERE transaction_id=$1").bind(source_grant_only).fetch_one(platform).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(f.org).bind(app).execute(platform).await?;
        let after_regrant:DateTime<Utc>=sqlx::query_scalar("SELECT resource_revoked_at FROM context_selections WHERE transaction_id=$1").bind(consumed).fetch_one(platform).await?;
        ensure!(after_regrant==revoked,"application regrant repaired or removed durable source revocation");
        let source_grant_restored:DateTime<Utc>=sqlx::query_scalar("SELECT resource_revoked_at FROM context_selections WHERE transaction_id=$1").bind(source_grant_only).fetch_one(platform).await?;
        ensure!(source_grant_restored==source_grant_revoked,"application regrant revived a source revoked only by the withdrawn organization grant");
        db.verify().await?;Ok(())
    }.await;
    db.finish(result).await
}
