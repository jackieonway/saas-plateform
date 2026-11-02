//! Real customer source projection; employee and customer share no authority.
use super::*;

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase14_customer_source_retains_exact_token_grant_session_and_deadline() -> Result<()> {
    let mut db = Database::create().await?;
    let result = async {
        let f = Fixture::create(&db.pool).await?;
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,identity_version) VALUES($1,$2,$3,'CUSTOMER',5)").bind(uuid7::new_uuid()).bind(f.user).bind(f.org).execute(&db.pool).await?;
        let customer = phase13::user_source(&db.pool,f.user,"CUSTOMER").await?;
        let employee = phase13::console_source(&db.pool,f.user).await?;
        db.provision().await?;
        let platform = db.platform()?;
        ensure!(customer_projection(platform,f.user,&customer).await?,"fresh canonical customer source was rejected");
        ensure!(!customer_projection(platform,f.user,&employee).await?,"employee grant became a customer source");
        assert_definer_owner_privileges(platform,"public.customer_portal_account_source_active(text,text,uuid,uuid,uuid,uuid,bigint,bigint,bigint,timestamptz)",&db.role("identity_owner"),"public.auth_grants").await?;
        denied(platform,"SELECT * FROM auth_access_tokens","customer projection grants no raw token access").await?;
        denied(db.identity()?,"SELECT * FROM memberships","customer source projection grants no identity membership access").await?;
        let generic_allowed: bool = sqlx::query_scalar("SELECT has_function_privilege(current_user,'public.auth_user_grant_source_active(uuid,uuid,uuid,bigint,uuid,bigint,text,text,text)','EXECUTE')").fetch_one(platform).await?;
        ensure!(!generic_allowed,"platform runtime can invoke the generic identity source verifier directly");
        let query = "SELECT customer_portal_account_source_active($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)";
        for (digest,issuer,client,grant,sid,user,account,policy,auth,deadline) in [
            ("g".repeat(64),ISSUER,customer.client,customer.grant,customer.sid,f.user,3_i64,1_i64,customer.auth_time,customer.expires),
            (customer.digest.clone(),"https://foreign.example.invalid",customer.client,customer.grant,customer.sid,f.user,3,1,customer.auth_time,customer.expires),
            (customer.digest.clone(),ISSUER,employee.client,customer.grant,customer.sid,f.user,3,1,customer.auth_time,customer.expires),
            (customer.digest.clone(),ISSUER,customer.client,uuid7::new_uuid(),customer.sid,f.user,3,1,customer.auth_time,customer.expires),
            (customer.digest.clone(),ISSUER,customer.client,customer.grant,uuid7::new_uuid(),f.user,3,1,customer.auth_time,customer.expires),
            (customer.digest.clone(),ISSUER,customer.client,customer.grant,customer.sid,uuid7::new_uuid(),3,1,customer.auth_time,customer.expires),
            (customer.digest.clone(),ISSUER,customer.client,customer.grant,customer.sid,f.user,4,1,customer.auth_time,customer.expires),
            (customer.digest.clone(),ISSUER,customer.client,customer.grant,customer.sid,f.user,3,2,customer.auth_time,customer.expires),
            (customer.digest.clone(),ISSUER,customer.client,customer.grant,customer.sid,f.user,3,1,customer.auth_time+1,customer.expires),
            (customer.digest.clone(),ISSUER,customer.client,customer.grant,customer.sid,f.user,3,1,customer.auth_time,customer.expires+chrono::Duration::seconds(1)),
        ] {
            let active: bool = sqlx::query_scalar(query).bind(digest).bind(issuer).bind(client).bind(grant).bind(sid).bind(user).bind(account).bind(policy).bind(auth).bind(deadline).fetch_one(platform).await?;
            ensure!(!active,"customer source projection accepted a substituted original tuple");
        }
        for mutation in [
            format!("UPDATE auth_access_tokens SET revoked_at=clock_timestamp() WHERE token_digest='{}'",customer.digest),
            format!("UPDATE auth_grants SET revoked_at=clock_timestamp() WHERE id='{}'",customer.grant),
            format!("UPDATE auth_clients SET policy_version=policy_version+1 WHERE id='{}'",customer.client),
            format!("UPDATE auth_applications SET projection_version=projection_version+1 WHERE application_id=(SELECT application_id FROM auth_clients WHERE id='{}')",customer.client),
            format!("UPDATE auth_applications SET purpose='PLATFORM_CONSOLE' WHERE application_id=(SELECT application_id FROM auth_clients WHERE id='{}')",customer.client),
            format!("UPDATE auth_consents SET generation=generation+1 WHERE id=(SELECT consent_id FROM auth_grants WHERE id='{}')",customer.grant),
            format!("UPDATE auth_consent_groups SET version=version+1 WHERE id=(SELECT group_id FROM auth_consents WHERE id=(SELECT consent_id FROM auth_grants WHERE id='{}'))",customer.grant),
            format!("UPDATE sso_sessions SET revoked_at=clock_timestamp() WHERE id='{}'",customer.sid),
            format!("UPDATE sso_sessions SET expires_at=clock_timestamp()+interval '1 minute' WHERE id='{}'",customer.sid),
            format!("UPDATE auth_refresh_families SET revoked_at=clock_timestamp() WHERE grant_id='{}'",customer.grant),
            format!("DELETE FROM auth_access_token_scopes WHERE token_id=(SELECT id FROM auth_access_tokens WHERE token_digest='{}') AND scope_id IN(SELECT id FROM auth_scopes WHERE scope_name='profile')",customer.digest),
        ] {
            let mut tx = db.pool.begin().await?;
            sqlx::query(&mutation).execute(&mut *tx).await?;
            let active: bool = sqlx::query_scalar(query).bind(&customer.digest).bind(ISSUER).bind(customer.client).bind(customer.grant).bind(customer.sid).bind(f.user).bind(3_i64).bind(1_i64).bind(customer.auth_time).bind(customer.expires).fetch_one(&mut *tx).await?;
            ensure!(!active,"canonical customer source survived an invalidated original authority");
            tx.rollback().await?;
            ensure!(customer_projection(platform,f.user,&customer).await?,"rolling back an isolated customer mutation did not restore its fixture");
        }
        db.verify().await?;
        Ok(())
    }.await;
    db.finish(result).await
}
