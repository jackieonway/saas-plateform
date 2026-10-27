//! Real RP callback and business-session writes; the new identity extension is
//! supplied at its explicit transport boundary (identity has separate real tests).
use super::*;
use crate::context_selection_http::authority::{
    Client as SelectionClient, SourceFuture, SourcePort, SourceProof,
};
use crate::error::AppError;
use saas_platform_backend::{
    context_selection::{ContractVersion, Credential, Id, Intent, Selection, Source, Version},
    context_selection_store::{self, Consumer},
};

struct SelectionAuthority {
    pool: PgPool,
    proof: SourceProof,
    grant: Uuid,
}
impl SourcePort for SelectionAuthority {
    fn verify<'a>(
        &'a self,
        transaction: &'a str,
        client: Id,
        access: Option<&'a str>,
    ) -> SourceFuture<'a> {
        Box::pin(async move {
            let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM oidc_access_tokens WHERE token_hash=$1 AND grant_id=$2 AND revoked_at IS NULL AND expires_at>clock_timestamp())")
                .bind(hash::sha256_hex(access.ok_or(AppError::Unavailable)?)).bind(self.grant).fetch_one(&self.pool).await?;
            if !active
                || transaction != self.proof.transaction_id.uuid().to_string()
                || client != self.proof.client.id
            {
                return Err(AppError::unauthorized("fixture source mismatch"));
            }
            Ok(self.proof.clone())
        })
    }
}
fn id(v: Uuid) -> Id {
    Id::new(v).unwrap()
}
fn version(v: i64) -> Version {
    Version::new(v).unwrap()
}

#[tokio::test]
#[ignore = "requires task-owned IDENTITY_PHASE10_TEST_DATABASE_URL selecting postgres"]
async fn centralized_callback_commits_selected_employee_and_rejects_replay() -> Result<()> {
    callback_case(false).await?;
    callback_case(true).await
}

async fn callback_case(preconsumed: bool) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let fixture=Fixture::create(&db.pool).await?;
        let mut bff=Bff::new(&fixture)?;
        let provider=fixture.app();
        let sso=sso_login(&provider,&fixture.dual,PASSWORD).await?;
        let login=console_login(&bff.app,&fixture.employee,"/",None).await?;
        let (_,code)=provider_approval(&provider,&fixture.employee,&login,&sso,None).await?;
        let flow:Uuid=sqlx::query_scalar("SELECT id FROM console_login_flows WHERE state_hash=$1").bind(hash::sha256_hex(&login.state)).fetch_one(&db.pool).await?;
        let (grant,sid,user,av,pv,auth,expires):(Uuid,Uuid,Uuid,i64,i64,chrono::DateTime<chrono::Utc>,chrono::DateTime<chrono::Utc>)=sqlx::query_as("SELECT g.id,g.session_id,g.user_id,g.account_version,g.client_policy_version,g.auth_time,g.expires_at FROM oidc_grants g JOIN oidc_authorization_codes c ON c.grant_id=g.id WHERE c.token_hash=$1")
            .bind(hash::sha256_hex(&code)).fetch_one(&db.pool).await?;
        let (membership,org,tenant,iv):(Uuid,Uuid,Uuid,i64)=sqlx::query_as("SELECT m.id,m.organization_id,o.tenant_id,m.identity_version FROM memberships m JOIN organizations o ON o.id=m.organization_id WHERE m.user_id=$1 AND m.member_type='EMPLOYEE' LIMIT 1")
            .bind(user).fetch_one(&db.pool).await?;
        sqlx::query("INSERT INTO context_selection_registrations(client_id,client_policy_version,member_type) VALUES($1,$2,'EMPLOYEE')").bind(fixture.employee.id).bind(pv).execute(&db.pool).await?;
        sqlx::query("UPDATE console_login_flows SET context_selection_required=true WHERE id=$1").bind(flow).execute(&db.pool).await?;
        let proof=SourceProof {contract_version:ContractVersion::V1,transaction_id:id(flow),intent:Intent::Login {},
            source:Source{issuer:ORIGIN.into(),user_id:id(user),account_version:version(av),global_session_id:id(sid),interaction_id:id(Uuid::now_v7()),auth_time:auth},
            client:SelectionClient{id:id(fixture.employee.id),policy_version:version(pv),purpose:"PLATFORM_CONSOLE".into(),allowed_member_types:vec!["EMPLOYEE".into()],redirect_uri:REDIRECT.into()},
            source_expires_at:expires,interaction_expires_at:chrono::Utc::now()+chrono::Duration::minutes(4)};
        let binding=saas_platform_backend::context_selection::Binding {transaction_id:id(flow),source:proof.source.clone(),intent:Intent::Login {},registration:saas_platform_backend::context_selection::Registration {
            client_id:id(fixture.employee.id),client_policy_version:version(pv),member_type:saas_platform_backend::context_selection::MemberType::Employee,redirect_uri:REDIRECT.into(),application:None}};
        let member=saas_platform_backend::context_selection::Member {user_id:id(user),tenant_id:id(tenant),organization_id:id(org),membership_id:id(membership),member_type:saas_platform_backend::context_selection::MemberType::Employee,identity_version:version(iv)};
        let credential=Credential::parse("a".repeat(64))?;
        let selection=Selection::confirm(binding,member,ORIGIN,chrono::Utc::now(),proof.source_expires_at,proof.interaction_expires_at)?;
        let mut tx=db.pool.begin().await?;
        context_selection_store::issue(&mut tx,&selection,&credential,Consumer{client_id:id(fixture.employee.id),service_id:None},100).await?;
        if preconsumed {
            context_selection_store::consume(&mut tx,&credential,Consumer{client_id:id(fixture.employee.id),service_id:None},&selection.binding,&selection.member,
                &context_selection_store::ContextResult::Employee{context_id:id(Uuid::now_v7()),version:version(1)}).await?;
        }
        tx.commit().await?;
        *bff.transport.selection_extension.lock().unwrap()=Some(json!({"contractVersion":"context-selection/1","transactionId":flow,"credential":credential}));
        bff.state.context_selection_authority=Arc::new(SelectionAuthority{pool:db.pool.clone(),proof,grant});
        bff.app=router(bff.state.clone());
        let reply=callback(&bff.app,&login,&code).await?;
        if preconsumed {
            ensure!(reply.status==StatusCode::CONFLICT,"consumed credential must reject the callback");
            let (bffs,contexts):(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM bff_sessions WHERE user_id=$1),(SELECT count(*) FROM platform_sessions WHERE user_id=$1)")
                .bind(user).fetch_one(&db.pool).await?;
            ensure!(bffs==0 && contexts==0,"failed selection consumption left business or BFF sessions behind");
            let current:Option<Uuid>=sqlx::query_scalar("SELECT current_session_id FROM console_browsers WHERE token_hash=$1").bind(hash::sha256_hex(cookie_secret(&login.cookie)?)).fetch_one(&db.pool).await?;
            ensure!(current.is_none(),"failed selection changed the current browser session");
            return Ok(());
        }
        ensure!(reply.status==StatusCode::SEE_OTHER,"selected callback failed: {} {}",reply.status,reply.body);
        let session=ConsoleSession {cookie:response_cookie(&reply,CONSOLE_COOKIE)?};
        let view=console_session(&bff.app,&session).await?;
        expect(&view,StatusCode::OK,"selected employee survives real session projection")?;
        ensure!(view.body["context"]["membershipId"]==json!(membership) && view.body["centralizedContextSelection"]==true,"callback did not install selected context");
        let renewed=console(&bff.app,Method::POST,"/auth/context-token",Some(&session.cookie),Some(text(&view.body,"csrfToken")?),json!({"expectedVersion":"1"})).await?;
        expect(&renewed,StatusCode::OK,"callback initialized real employee renewal proof")?;
        let sid=text(&renewed.body,"accessToken")?;
        expect(&call(&bff.app,Method::GET,"/api/auth/profile",Some(sid),Value::Null).await?,StatusCode::OK,"derived bearer reaches real business API")?;
        let rejected_selection=console(&bff.app,Method::POST,"/auth/context-token",Some(&session.cookie),Some(text(&view.body,"csrfToken")?),json!({"expectedVersion":"1","membershipId":membership,"organizationId":org})).await?;
        expect(&rejected_selection,StatusCode::FORBIDDEN,"managed RP cannot bypass identity selection")?;
        let (consumed,contexts):(bool,i64)=sqlx::query_as("SELECT consumed_at IS NOT NULL,(SELECT count(*) FROM platform_sessions WHERE bff_session_id=$2) FROM context_selections WHERE transaction_id=$1")
            .bind(flow).bind(bff_id(&db.pool,&session).await?).fetch_one(&db.pool).await?;
        ensure!(consumed && contexts==1,"selection and business context must commit together");
        rejected(&callback(&bff.app,&login,&code).await?,"replayed callback")?;
        Ok(())
    }.await;
    db.finish(result).await
}
