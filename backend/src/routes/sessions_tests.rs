use anyhow::{ensure, Result};
use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tower::ServiceExt;

#[tokio::test]
#[ignore = "creates and drops an owned database using TEST_DATABASE_URL"]
async fn administrators_revoke_devices_with_permissions_and_atomic_audit() -> Result<()> {
    let db = crate::entity_creation_tests::Database::create().await?;
    let result:Result<()>=async {
        let init=crate::database::bootstrap::initialize_platform(&db.pool,"admin","Owned-Session-Test9!").await?;
        // Exercise delegated role permissions instead of the super-admin bypass.
        sqlx::query("UPDATE users SET is_super_admin=false WHERE id=$1").bind(init.admin_id).execute(&db.pool).await?;
        let secret="owned-session-tests-only";
        let config=crate::config::Config::from_values(&HashMap::from([("DATABASE_URL".into(),"postgres://unused/session-tests".into()),("JWT_SECRET".into(),secret.into())]))?;
        let router=crate::routes::api_router().with_state(crate::sso_test_fixture::state(db.pool.clone(),Arc::new(config)));
        let admin=crate::sso_test_fixture::token(&db.pool,init.admin_id,init.tenant_id,init.organization_id,secret).await?;
        let target=crate::util::uuid7::new_uuid();
        sqlx::query("INSERT INTO users(id,username,password_hash) VALUES($1,'managed-device','unused-fixture')").bind(target).execute(&db.pool).await?;
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id) VALUES($1,$2,$3)").bind(crate::util::uuid7::new_uuid()).bind(target).bind(init.organization_id).execute(&db.pool).await?;
        let ordinary=crate::sso_test_fixture::token(&db.pool,target,init.tenant_id,init.organization_id,secret).await?;
        let sid:uuid::Uuid=sqlx::query_scalar("SELECT id FROM platform_sessions WHERE user_id=$1").bind(target).fetch_one(&db.pool).await?;
        let second=crate::sso_test_fixture::session(&db.pool,target,init.tenant_id,init.organization_id).await?;
        let call=|token:&str,method:Method,path:String| {
            let app=router.clone();let token=token.to_owned();
            async move { let response=app.oneshot(Request::builder().method(method).uri(path).header("Authorization",format!("Bearer {token}")).body(Body::empty())?).await?;
                let status=response.status();let bytes=to_bytes(response.into_body(),1024*1024).await?;let value:Value=serde_json::from_slice(&bytes)?;Ok::<_,anyhow::Error>((status,value)) }
        };
        ensure!(call(&ordinary,Method::GET,"/api/admin/sessions".into()).await?.0==StatusCode::FORBIDDEN,"ordinary user listed global sessions");
        let (status,page)=call(&admin,Method::GET,"/api/admin/sessions?keyword=managed-device&size=1".into()).await?;
        ensure!(status==StatusCode::OK && page["total"]==2 && page["records"].as_array().unwrap().len()==1,"session filtering or paging failed: {page}");
        let (_,page)=call(&admin,Method::GET,"/api/admin/session-users?keyword=managed-device".into()).await?;
        ensure!(page["total"]==1 && page["records"][0]["activeSessions"]==2,"user session aggregation failed");
        let path=format!("/api/admin/sessions/{sid}/revoke");
        let removed=sqlx::query_scalar::<_,uuid::Uuid>("DELETE FROM role_menus WHERE menu_id=(SELECT id FROM menus WHERE permission='system:session:revoke') RETURNING role_id").fetch_one(&db.pool).await?;
        ensure!(call(&admin,Method::POST,path.clone()).await?.0==StatusCode::FORBIDDEN,"read permission allowed revocation");
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) SELECT $1,id FROM menus WHERE permission='system:session:revoke'").bind(removed).execute(&db.pool).await?;
        sqlx::raw_sql("CREATE FUNCTION reject_session_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='ADMIN_REVOKE_SESSION' THEN RAISE EXCEPTION 'owned audit rejection'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_session_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION reject_session_audit();").execute(&db.pool).await?;
        ensure!(call(&admin,Method::POST,path.clone()).await?.0==StatusCode::INTERNAL_SERVER_ERROR,"audit failure was ignored");
        let active:bool=sqlx::query_scalar("SELECT platform_session_active($1)").bind(sid).fetch_one(&db.pool).await?;
        ensure!(active,"audit failure partially revoked the session");
        sqlx::query("DROP TRIGGER reject_session_audit ON audit_logs").execute(&db.pool).await?;
        ensure!(call(&admin,Method::POST,path.clone()).await?.0==StatusCode::OK,"device revocation failed");
        ensure!(call(&ordinary,Method::GET,"/api/auth/profile".into()).await?.0==StatusCode::UNAUTHORIZED,"revoked token still works");
        let unaffected:bool=sqlx::query_scalar("SELECT platform_session_active($1)").bind(second).fetch_one(&db.pool).await?;
        ensure!(unaffected,"device revocation affected another device");
        ensure!(call(&admin,Method::POST,path).await?.1["revoked"]==0,"repeat revocation was not idempotent");
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='ADMIN_REVOKE_SESSION'").fetch_one(&db.pool).await?;
        ensure!(count==1,"audit was missing or repeated");
        let (_,page)=call(&admin,Method::GET,"/api/admin/sessions?keyword=managed-device&activeOnly=true".into()).await?;
        ensure!(page["total"]==1,"active filter returned a revoked device");
        let missing=crate::util::uuid7::new_uuid();
        ensure!(call(&admin,Method::POST,format!("/api/admin/sessions/{missing}/revoke")).await?.0==StatusCode::NOT_FOUND,"unknown session was accepted");
        ensure!(call(&ordinary,Method::POST,format!("/api/admin/session-users/{}/revoke-all",init.admin_id)).await?.0==StatusCode::UNAUTHORIZED,"revoked actor can kick a user");
        Ok(())
    }.await;
    db.finish(result).await
}
