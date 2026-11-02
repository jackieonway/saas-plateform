//! Canonical logout/delivery boundaries exercised through actual LOGIN roles.
use super::*;

async fn target(
    pool: &PgPool,
    purpose: &str,
    client: Option<Uuid>,
    recipient: Option<&str>,
    channel: Option<&str>,
) -> Result<Uuid> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO auth_delivery_targets(id,purpose,status,client_id,recipient_ref,channel_purpose,url,allowed_ips,verified_reference,verified_at,expires_at) VALUES($1,$2,'ACTIVE',$3,$4,$5,'https://owned-recipient.example.invalid/receive',ARRAY['203.0.113.20'],$6,clock_timestamp()-interval '1 minute',clock_timestamp()+interval '1 day')")
        .bind(id).bind(purpose).bind(client).bind(recipient).bind(channel).bind(uuid7::new_uuid()).execute(pool).await?;
    Ok(id)
}

async fn fresh_grant_for_rp(pool: &PgPool, original: &OidcSource) -> Result<Uuid> {
    let request = uuid7::new_uuid();
    let context = uuid7::new_uuid();
    let grant = uuid7::new_uuid();
    sqlx::query("INSERT INTO auth_authorization_requests SELECT (jsonb_populate_record(NULL::public.auth_authorization_requests,to_jsonb(request)||jsonb_build_object('id',$1::text,'client_policy_version',client.policy_version))).* FROM auth_authorization_requests request JOIN auth_context_interactions selected ON selected.authorization_request_id=request.id JOIN auth_grants original ON original.context_ref=selected.transaction_id JOIN auth_clients client ON client.id=original.client_id WHERE original.id=$2")
        .bind(request).bind(original.grant).execute(pool).await?;
    sqlx::query("INSERT INTO auth_context_interactions SELECT (jsonb_populate_record(NULL::public.auth_context_interactions,to_jsonb(selected)||jsonb_build_object('transaction_id',$1::text,'authorization_request_id',$2::text,'request_hash',$3,'source_digest',$4,'client_policy_version',client.policy_version,'grant_id',NULL,'delivered_at',NULL,'credential_ciphertext','owned-fresh-selection'))).* FROM auth_context_interactions selected JOIN auth_grants original ON original.context_ref=selected.transaction_id JOIN auth_clients client ON client.id=original.client_id WHERE original.id=$5")
        .bind(context).bind(request).bind(hash::sha256_hex(&context.to_string())).bind(hash::sha256_hex(&request.to_string())).bind(original.grant).execute(pool).await?;
    sqlx::query("INSERT INTO auth_grants SELECT (jsonb_populate_record(NULL::public.auth_grants,to_jsonb(original)||jsonb_build_object('id',$1::text,'context_ref',$2::text,'client_policy_version',client.policy_version,'revoked_at',NULL))).* FROM auth_grants original JOIN auth_clients client ON client.id=original.client_id WHERE original.id=$3")
        .bind(grant).bind(context).bind(original.grant).execute(pool).await?;
    sqlx::query("UPDATE auth_context_interactions SET grant_id=$2,delivered_at=clock_timestamp(),credential_ciphertext=NULL WHERE transaction_id=$1").bind(context).bind(grant).execute(pool).await?;
    sqlx::query("INSERT INTO auth_grant_scopes(grant_id,resource_id,scope_id) SELECT $1,resource_id,scope_id FROM auth_grant_scopes WHERE grant_id=$2").bind(grant).bind(original.grant).execute(pool).await?;
    sqlx::query("INSERT INTO auth_rp_sessions(grant_id,client_id,user_id,source_session_id,public_sid,public_sub) SELECT $1,client_id,user_id,source_session_id,$2,public_sub FROM auth_rp_sessions WHERE grant_id=$3").bind(grant).bind(hash::sha256_hex(&grant.to_string())).bind(original.grant).execute(pool).await?;
    Ok(grant)
}

#[tokio::test]
#[ignore = "requires explicit task-owned IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase20_real_roles_preserve_original_rp_sources_and_atomic_delivery_facts() -> Result<()> {
    let mut db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        bind_sso_fixture(&db.pool,&f).await?;
        let one=phase13::console_source(&db.pool,f.user).await?;
        let two=phase13::console_source(&db.pool,f.user).await?;
        let destination=target(&db.pool,"BACK_CHANNEL",Some(one.client),None,None).await?;
        db.provision().await?;
        let identity=db.identity()?;let platform=db.platform()?;
        let ready:bool=sqlx::query_scalar("SELECT identity_phase20_schema_ready()").fetch_one(identity).await?;
        ensure!(ready,"fresh phase20 canonical owner/ACL closure rejected");
        for table in ["auth_rp_sessions","auth_logout_command_receipts","auth_delivery_targets","auth_delivery_jobs","auth_invitation_verifications","auth_invitation_activation_receipts"] {
            denied(platform,&format!("SELECT * FROM {table}"),"platform has no identity delivery/private source rows").await?;
        }
        for table in ["auth_delivery_targets","auth_invitation_verifications"] {
            denied(identity,&format!("UPDATE {table} SET status='REVOKED'"),"runtime cannot self-attest owner channel facts").await?;
            denied(identity,&format!("DELETE FROM {table}"),"runtime cannot delete owner channel facts").await?;
        }
        denied(identity,&format!("SELECT auth_enqueue_logout('{}',NULL)",one.grant),"internal trigger enqueue is not a runtime RPC").await?;
        denied(platform,"SELECT * FROM auth_consent_revocation_outbox","platform receives no consent delivery rows").await?;
        for source in [&one,&two] {
            sqlx::query("INSERT INTO auth_rp_sessions(grant_id,client_id,user_id,source_session_id,public_sid,public_sub) VALUES($1,$2,$3,$4,$5,'opaque-pairwise-subject')")
                .bind(source.grant).bind(source.client).bind(f.user).bind(source.sid).bind(hash::sha256_hex(&source.grant.to_string())).execute(identity).await?;
        }
        sqlx::query("UPDATE auth_clients SET logout_mode='BACK_CHANNEL',backchannel_target_id=$2,backchannel_logout_uri='https://owned-recipient.example.invalid/receive' WHERE id=$1")
            .bind(one.client).bind(destination).execute(identity).await?;
        denied(identity,"UPDATE auth_rp_sessions SET public_sub='forged'","RP sources are append-only to runtime").await?;
        denied(identity,"DELETE FROM auth_rp_sessions","RP source deletion denied").await?;
        sqlx::query("UPDATE auth_grants SET revoked_at=clock_timestamp() WHERE id=$1").bind(one.grant).execute(identity).await?;
        let first:(String,Uuid,i64)=sqlx::query_as("SELECT state,target_id,target_version FROM auth_delivery_jobs WHERE grant_id=$1").bind(one.grant).fetch_one(identity).await?;
        ensure!(first==("PENDING".into(),destination,1),"original RP revocation lost its exact verified destination");
        let untouched:bool=sqlx::query_scalar("SELECT revoked_at IS NULL FROM auth_grants WHERE id=$1").bind(two.grant).fetch_one(identity).await?;
        ensure!(untouched,"one grant source revocation expanded to another RP");
        sqlx::query("UPDATE auth_grants SET revoked_at=clock_timestamp() WHERE id=$1").bind(one.grant).execute(identity).await?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM auth_delivery_jobs WHERE grant_id=$1").bind(one.grant).fetch_one(identity).await?;
        ensure!(count==1,"repeated source mutation duplicated a stable delivery event");
        denied(identity,"DELETE FROM auth_delivery_jobs","runtime cannot erase delivery history").await?;
        constraint_rejected(identity,&format!("UPDATE auth_delivery_jobs SET target_version=2 WHERE grant_id='{}'",one.grant),"23514","delivery destination generation cannot be rebound").await?;
        sqlx::query("UPDATE auth_delivery_jobs SET state='LEASED',lease_id=$2,lease_until=clock_timestamp()+interval '30 seconds',attempt_count=1,last_jti=$3 WHERE grant_id=$1")
            .bind(one.grant).bind(uuid7::new_uuid()).bind(uuid7::new_uuid()).execute(identity).await?;
        sqlx::query("UPDATE auth_delivery_jobs SET state='DELIVERED',delivered_at=clock_timestamp(),lease_id=NULL,lease_until=NULL WHERE grant_id=$1").bind(one.grant).execute(identity).await?;
        constraint_rejected(identity,&format!("UPDATE auth_delivery_jobs SET state='PENDING' WHERE grant_id='{}'",one.grant),"23514","terminal delivery cannot be revived").await?;
        let consent_event=uuid7::new_uuid();
        sqlx::query("INSERT INTO auth_consent_revocation_outbox(id,consent_id,user_id,group_id,client_id,consent_generation,event_kind) SELECT $1,c.id,c.user_id,c.group_id,$2,c.generation,'CONSENT_REVOKED' FROM auth_consents c JOIN auth_grants g ON g.consent_id=c.id WHERE g.id=$3")
            .bind(consent_event).bind(one.client).bind(one.grant).execute(identity).await?;
        let linked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_delivery_jobs WHERE grant_id=$1 AND consent_event_id=$2 AND state='DELIVERED')").bind(one.grant).bind(consent_event).fetch_one(identity).await?;
        ensure!(linked,"consent source did not reuse its already-delivered original grant event");
        let summarized:String=sqlx::query_scalar("SELECT delivery_state FROM auth_consent_revocation_outbox WHERE id=$1").bind(consent_event).fetch_one(identity).await?;
        ensure!(summarized=="DELIVERED","consent expansion overwrote the existing job's terminal delivery summary");
        let later=fresh_grant_for_rp(identity,&one).await?;
        sqlx::query("UPDATE auth_delivery_targets SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(destination).execute(&db.pool).await?;
        sqlx::query("UPDATE auth_clients SET policy_version=policy_version+1,version=version+1 WHERE id=$1").bind(one.client).execute(identity).await?;
        let emergency:(bool,String)=sqlx::query_as("SELECT grant_row.revoked_at IS NOT NULL,job.state FROM auth_grants grant_row JOIN auth_delivery_jobs job ON job.grant_id=grant_row.id WHERE grant_row.id=$1").bind(later).fetch_one(identity).await?;
        ensure!(emergency==(true,"DEGRADED".into()),"expired notification target blocked version-only emergency credential revocation");
        sqlx::query("UPDATE auth_delivery_targets SET expires_at=clock_timestamp()+interval '1 day' WHERE id=$1").bind(destination).execute(&db.pool).await?;
        let stopped_grant=fresh_grant_for_rp(identity,&one).await?;
        sqlx::query("UPDATE auth_delivery_targets SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(destination).execute(&db.pool).await?;
        sqlx::query("UPDATE auth_clients SET status='DISABLED',policy_version=policy_version+1,version=version+1 WHERE id=$1").bind(one.client).execute(identity).await?;
        let stopped:(bool,String)=sqlx::query_as("SELECT grant_row.revoked_at IS NOT NULL,job.state FROM auth_grants grant_row JOIN auth_delivery_jobs job ON job.grant_id=grant_row.id WHERE grant_row.id=$1").bind(stopped_grant).fetch_one(identity).await?;
        ensure!(stopped==(true,"DEGRADED".into()),"expired notification target blocked pure client shrink or lost original grant invalidation");
        let other_live:bool=sqlx::query_scalar("SELECT revoked_at IS NULL FROM auth_grants WHERE id=$1").bind(two.grant).fetch_one(identity).await?;
        ensure!(other_live,"one client shutdown revoked another client's original grant");
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(f.user).execute(identity).await?;
        let second:(bool,bool,String)=sqlx::query_as("SELECT g.revoked_at IS NOT NULL,s.revoked_at IS NOT NULL,j.state FROM auth_grants g JOIN sso_sessions s ON s.id=g.source_session_id JOIN auth_delivery_jobs j ON j.grant_id=g.id WHERE g.id=$1").bind(two.grant).fetch_one(identity).await?;
        ensure!(second==(true,true,"DEGRADED".into()),"account source invalidation lost the explicit short-session downgrade");
        let active_consents:i64=sqlx::query_scalar("SELECT count(*) FROM auth_consents WHERE user_id=$1 AND status='ACTIVE'").bind(f.user).fetch_one(identity).await?;
        ensure!(active_consents>=2,"logout source invalidation deleted or revoked consent");
        let security=uuid7::new_uuid();
        sqlx::query("INSERT INTO auth_security_notification_outbox(id,user_id,event_kind) VALUES($1,$2,'MFA_RECOVERED')").bind(security).bind(f.user).execute(identity).await?;
        let degraded:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_delivery_jobs WHERE security_event_id=$1 AND state='DEGRADED' AND last_error_code='VERIFIED_CHANNEL_MISSING')").bind(security).fetch_one(identity).await?;
        ensure!(degraded,"security event inferred an unverified recipient");
        for table in ["platform_backchannel_logout_receipts","platform_backchannel_logout_sources"] {
            denied(identity,&format!("SELECT * FROM {table}"),"identity cannot read BFF replay/source facts").await?;
            denied(platform,&format!("DELETE FROM {table}"),"platform cannot erase replay/source facts").await?;
        }
        constraint_rejected(platform,"UPDATE bff_sessions SET oidc_public_sub='forged'","23514","BFF public subject cannot be replaced").await?;
        db.verify().await?;Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit task-owned IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase20_real_roles_accept_only_owner_verified_new_account_activation() -> Result<()> {
    let mut db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        let service=uuid7::new_uuid();let invitation=uuid7::new_uuid();let verification=uuid7::new_uuid();
        let recipient="owned-invitation-recipient";
        let destination=target(&db.pool,"VERIFIED_CHANNEL",None,Some(recipient),Some("ACCOUNT_ONBOARDING")).await?;
        sqlx::query("INSERT INTO identity_service_clients(id,secret_hash,purpose,status,created_at) VALUES($1,$2,'AUTH_INVITATION','ENABLED',clock_timestamp())").bind(service).bind(hash::sha256_hex(&service.to_string())).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_invitations(id,service_id,operation_id,request_digest,recipient_ref,purpose,actor_user_id,actor_session_id,key_version,token_digest,status,version,created_at,expires_at) VALUES($1,$2,$1,$3,$4,'ACCOUNT_ONBOARDING',$5,$6,1,$7,'ACTIVE',1,statement_timestamp(),statement_timestamp()+interval '24 hours')")
            .bind(invitation).bind(service).bind(hash::sha256_hex(&invitation.to_string())).bind(recipient).bind(f.user).bind(f.sessions[0]).bind(hash::sha256_hex("owned-invitation-token")).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_delivery_jobs(id,kind,event_key,target_id,target_version,invitation_id) VALUES($1,'INVITATION',$2,$3,1,$4)")
            .bind(uuid7::new_uuid()).bind(hash::sha256_hex("owned-invitation-event")).bind(destination).bind(invitation).execute(&db.pool).await?;
        sqlx::query("INSERT INTO auth_invitation_verifications(id,invitation_id,recipient_ref,purpose,delivery_target_id,delivery_target_version,verified_reference,verified_at,expires_at,status) VALUES($1,$2,$3,'ACCOUNT_ONBOARDING',$4,1,$5,clock_timestamp()-interval '1 minute',clock_timestamp()+interval '1 hour','ACTIVE')")
            .bind(verification).bind(invitation).bind(recipient).bind(destination).bind(uuid7::new_uuid()).execute(&db.pool).await?;
        db.provision().await?;
        let identity=db.identity()?;
        let fenced:bool=sqlx::query_scalar("SELECT identity_delivery_target_active($1,1,'VERIFIED_CHANNEL',NULL,$2,'ACCOUNT_ONBOARDING') AND identity_invitation_verification_active($3,1,$4,$1,1,$2,'ACCOUNT_ONBOARDING')")
            .bind(destination).bind(recipient).bind(verification).bind(invitation).fetch_one(identity).await?;
        ensure!(fenced,"real identity LOGIN could not lock exact readonly owner facts through the narrow source fences");
        let substituted:bool=sqlx::query_scalar("SELECT identity_invitation_verification_active($1,1,$2,$3,1,$4,'ACCOUNT_ONBOARDING')")
            .bind(uuid7::new_uuid()).bind(invitation).bind(destination).bind(recipient).fetch_one(identity).await?;
        ensure!(!substituted,"an arbitrary verification UUID substituted for trusted owner evidence");
        denied(identity,"INSERT INTO auth_invitation_verifications SELECT * FROM auth_invitation_verifications WHERE false","runtime cannot create trusted verification facts").await?;
        let new_account=uuid7::new_uuid();let operation=uuid7::new_uuid();
        let mut tx=identity.begin().await?;
        sqlx::query("INSERT INTO users(id,username,password_hash) VALUES($1,$2,'owned-role-contract-password-placeholder')").bind(new_account).bind(format!("owned-activation-{}",new_account.simple())).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO auth_invitation_activation_receipts(operation_id,invitation_id,verification_id,user_id,invitation_version,verification_version,delivery_target_id,delivery_target_version,request_digest,key_version,result_json) VALUES($1,$2,$3,$4,1,1,$5,1,$6,1,jsonb_build_object('operationId',$1::text,'invitationId',$2::text,'status','ACCOUNT_CREATED'))")
            .bind(operation).bind(invitation).bind(verification).bind(new_account).bind(destination).bind(hash::sha256_hex("owned-activation-purpose-mac")).execute(&mut *tx).await?;
        sqlx::query("UPDATE auth_invitations SET status='CONSUMED',consumed_at=clock_timestamp(),verification_ref=$2,activated_account_id=$3,activation_operation_id=$4,version=version+1 WHERE id=$1")
            .bind(invitation).bind(verification).bind(new_account).bind(operation).execute(&mut *tx).await?;
        tx.commit().await?;
        let pure:(i64,i64)=sqlx::query_as("SELECT u.session_version,(SELECT count(*) FROM memberships WHERE user_id=u.id) FROM users u WHERE u.id=$1").bind(new_account).fetch_one(&db.pool).await?;
        ensure!(pure==(1,0),"activation created a business identity or invalid account generation");
        constraint_rejected(identity,&format!("UPDATE auth_invitations SET status='ACTIVE',consumed_at=NULL,verification_ref=NULL,activated_account_id=NULL,activation_operation_id=NULL WHERE id='{invitation}'"),"23514","consumed invitation cannot return to the active token state").await?;
        denied(identity,"UPDATE auth_invitation_activation_receipts SET key_version=2","activation result cannot be rewritten by runtime").await?;
        denied(identity,"DELETE FROM auth_invitation_activation_receipts","activation receipt deletion denied").await?;
        denied(identity,"INSERT INTO memberships SELECT * FROM memberships WHERE false","account activation cannot create business members").await?;
        db.verify().await?;Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit task-owned IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase20_owner_fact_fence_samples_expiry_after_a_confirmed_row_lock_wait() -> Result<()> {
    let mut db = Database::create().await?;
    let result=async {
        let destination=target(&db.pool,"VERIFIED_CHANNEL",None,Some("owned-fence-expiry"),Some("ACCOUNT_ONBOARDING")).await?;
        db.provision().await?;
        let identity=db.identity()?;
        let end:DateTime<Utc>=sqlx::query_scalar("UPDATE auth_delivery_targets SET expires_at=clock_timestamp()+interval '1200 milliseconds' WHERE id=$1 RETURNING expires_at")
            .bind(destination).fetch_one(&db.pool).await?;
        let live:bool=sqlx::query_scalar("SELECT identity_delivery_target_active($1,2,'VERIFIED_CHANNEL',NULL,'owned-fence-expiry','ACCOUNT_ONBOARDING')").bind(destination).fetch_one(identity).await?;
        ensure!(live,"short-lived owner fact started inactive");
        let mut locked=db.pool.begin().await?;
        sqlx::query("SELECT id FROM auth_delivery_targets WHERE id=$1 FOR UPDATE").bind(destination).execute(&mut *locked).await?;
        let mut connection=identity.acquire().await?;
        let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *connection).await?;
        let waiting=tokio::spawn(async move {sqlx::query_scalar::<_,bool>("SELECT identity_delivery_target_active($1,2,'VERIFIED_CHANNEL',NULL,'owned-fence-expiry','ACCOUNT_ONBOARDING')").bind(destination).fetch_one(&mut *connection).await});
        let mut waited=false;
        for _ in 0..250 {
            waited=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock' AND cardinality(pg_blocking_pids(pid))>0)").bind(pid).fetch_one(&db.pool).await?;
            let expired:bool=sqlx::query_scalar("SELECT clock_timestamp()>$1::timestamptz").bind(end).fetch_one(&db.pool).await?;
            if waited && expired {break;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        locked.commit().await?;
        let accepted=tokio::time::timeout(Duration::from_secs(3),waiting).await???;
        ensure!(waited && !accepted,"readonly owner fact was accepted after expiry during its actual row-lock wait");
        denied(identity,"UPDATE auth_delivery_targets SET status='REVOKED'","owner lock fence does not grant fact UPDATE").await?;
        db.verify().await?;Ok(())
    }.await;
    db.finish(result).await
}
