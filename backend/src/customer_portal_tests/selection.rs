//! Real platform transactions and customer resources, with an explicit identity
//! interaction port. No test fabricates a customer resource capability.
use super::*;
use crate::context_selection_http::authority::{Client, SourceFuture, SourcePort, SourceProof};
use saas_platform_backend::{
    context_selection::{
        ApplicationBinding, Binding, ContractVersion, Credential, Id, Intent, Member, MemberType,
        Registration, Selection, Source, Version,
    },
    context_selection_store::{self, Consumer},
};
use std::sync::Mutex;

mod lock_regression;

struct Sources(Mutex<HashMap<Uuid, (SourceProof, String)>>);
impl SourcePort for Sources {
    fn verify<'a>(
        &'a self,
        transaction: &'a str,
        client: Id,
        access: Option<&'a str>,
    ) -> SourceFuture<'a> {
        Box::pin(async move {
            let id = Uuid::parse_str(transaction).map_err(|_| AppError::Unavailable)?;
            let entries = self.0.lock().unwrap();
            let (proof, token) = entries.get(&id).ok_or(AppError::Unavailable)?;
            if proof.client.id != client || access != Some(token.as_str()) {
                return Err(AppError::unauthorized("fixture interaction mismatch"));
            }
            Ok(proof.clone())
        })
    }
}
fn id(v: Uuid) -> Id {
    Id::new(v).unwrap()
}
fn ver(v: i64) -> Version {
    Version::new(v).unwrap()
}

async fn issue(
    f: &Fixture,
    sources: &Sources,
    service: Uuid,
    transaction: Uuid,
    authentication: (&str, Uuid, Uuid),
    intent: Intent,
) -> Result<Value> {
    let (access, grant, sid) = authentication;
    let (auth,expiry):(chrono::DateTime<chrono::Utc>,chrono::DateTime<chrono::Utc>)=sqlx::query_as("SELECT auth_time,(SELECT expires_at FROM auth_access_tokens WHERE grant_id=$1 LIMIT 1) FROM auth_grants WHERE id=$1").bind(grant).fetch_one(&f.state.pool).await?;
    let proof = SourceProof {
        contract_version: ContractVersion::V1,
        transaction_id: id(transaction),
        intent: intent.clone(),
        source: Source {
            issuer: ISSUER.into(),
            user_id: id(f.user),
            account_version: ver(3),
            global_session_id: id(sid),
            interaction_id: id(Uuid::now_v7()),
            auth_time: auth,
        },
        client: Client {
            id: id(f.rp),
            policy_version: ver(1),
            purpose: "CUSTOMER_PORTAL".into(),
            allowed_member_types: vec!["CUSTOMER".into()],
            redirect_uri: "https://phase11-customer.example.invalid/callback".into(),
            client_registry: Some("AUTH".into()),
            subject_mode: Some("USER".into()),
            context_mode: Some("PLATFORM_CONTEXT".into()),
            application_id: Some(id(f.app)),
        },
        source_expires_at: expiry,
        interaction_expires_at: chrono::Utc::now() + chrono::Duration::minutes(4),
    };
    let binding = Binding {
        transaction_id: id(transaction),
        source: proof.source.clone(),
        intent,
        registration: Registration {
            client_id: id(f.rp),
            client_policy_version: ver(1),
            member_type: MemberType::Customer,
            redirect_uri: proof.client.redirect_uri.clone(),
            application: Some(ApplicationBinding {
                id: id(f.app),
                policy_version: ver(1),
            }),
        },
    };
    let member = Member {
        user_id: id(f.user),
        tenant_id: id(f.tenant),
        organization_id: id(f.org),
        membership_id: id(f.customer),
        member_type: MemberType::Customer,
        identity_version: ver(5),
    };
    let selected = Selection::confirm(
        binding,
        member,
        ISSUER,
        chrono::Utc::now(),
        proof.source_expires_at,
        proof.interaction_expires_at,
    )?;
    let credential = Credential::parse(crate::sso::opaque_secret()?)?;
    let credential_ciphertext = f
        .state
        .config
        .bff_token_keys
        .as_deref()
        .ok_or_else(|| anyhow!("selection test BFF keyring missing"))?
        .seal(
            selected.binding.transaction_id.uuid(),
            &selected.binding.source.issuer,
            selected.binding.registration.client_id.uuid(),
            "context-selection-credential",
            credential.as_secret(),
        )?;
    let mut tx = f.state.pool.begin().await?;
    context_selection_store::issue(
        &mut tx,
        &selected,
        &credential,
        &credential_ciphertext,
        Consumer {
            client_id: id(f.rp),
            service_id: Some(id(service)),
        },
        100,
    )
    .await?;
    tx.commit().await?;
    sources
        .0
        .lock()
        .unwrap()
        .insert(transaction, (proof, access.to_owned()));
    Ok(
        json!({"contractVersion":"context-selection/1","transactionId":transaction,"credential":credential,"expectedClientId":f.rp,"oidcAccessToken":access,"bffSessionId":Uuid::now_v7(),"version":"1","oldOidcAccessToken":null}),
    )
}
async fn call(app: &Router, route: &str, auth: &str, body: Value) -> Result<Reply> {
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/internal/platform/context-selection/{route}"))
                .header("content-type", "application/json")
                .header("authorization", auth)
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    ensure!(
        response.headers()["cache-control"] == "no-store",
        "selection results must not be cached"
    );
    let status = response.status();
    Ok(Reply {
        status,
        body: serde_json::from_slice(&to_bytes(response.into_body(), 65536).await?)?,
    })
}

#[tokio::test]
#[ignore = "requires explicit task-owned IDENTITY_PHASE11_TEST_DATABASE_URL"]
async fn centralized_customer_consumption_recovery_and_switch_are_exact() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let mut f=Fixture::create(&db.pool).await?;
        let service=Uuid::now_v7();
        sqlx::query("INSERT INTO context_selection_services(id,secret_hash,purpose,allowed_auth_clients,status) VALUES($1,$2,'CONTEXT_SELECTION_CONSUMER',ARRAY[$3]::uuid[],'ENABLED')").bind(service).bind(hash::sha256_hex(&"c".repeat(64))).bind(f.rp).execute(&db.pool).await?;
        sqlx::query("INSERT INTO context_selection_registrations(client_id,client_policy_version,member_type,application_id,application_policy_version,consumer_service_id) VALUES($1,1,'CUSTOMER',$2,1,$3)").bind(f.rp).bind(f.app).bind(service).execute(&db.pool).await?;
        let mut config=(*f.state.config).clone();
        config.bff_token_keys=Some(Arc::new(crate::bff::CipherKeys::fixture()));
        f.state.config=Arc::new(config);
        let sources=Arc::new(Sources(Mutex::new(HashMap::new())));
        f.state.context_selection_authority=sources.clone();
        let app=crate::context_selection_http::routes().with_state(f.state.clone());
        let auth=format!("Basic {}",STANDARD.encode(format!("{service}:{}","c".repeat(64))));
        let transaction=Uuid::now_v7();
        let body=issue(&f,&sources,service,transaction,(&f.access,f.grant,f.sid),Intent::Login {}).await?;
        let pending=call(&app,"result",&auth,body.clone()).await?;
        ensure!(pending.status==StatusCode::OK && pending.body["status"]=="PENDING","initial result is not pending: {} {}",pending.status,pending.body);
        let issued=call(&app,"consume",&auth,body.clone()).await?;
        ensure!(issued.status==StatusCode::OK,"customer consume failed: {} {}",issued.status,issued.body);
        let token=issued.body["customerContext"].as_str().ok_or_else(||anyhow!("missing customer context"))?;
        expect(&f.profile(token).await?,StatusCode::OK,"selected customer reaches real resource")?;
        let recovered=call(&app,"result",&auth,body.clone()).await?;
        ensure!(recovered.status==StatusCode::OK && recovered.body==issued.body,"recovery minted a different result");
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM customer_portal_contexts").fetch_one(&db.pool).await?;
        ensure!(count==1,"recovery created a new customer context");
        let (encoded,cipher):(String,String)=sqlx::query_as("SELECT result::text,result_ciphertext FROM context_selections WHERE transaction_id=$1").bind(transaction).fetch_one(&db.pool).await?;
        ensure!(!encoded.contains("cpc_") && !cipher.contains(token) && cipher.starts_with("v1."),"plaintext capability persisted in selection result");
        expect(&call(&app,"consume",&auth,body.clone()).await?,StatusCode::CONFLICT,"consumption replay")?;
        let mut wrong=body.clone();wrong["bffSessionId"]=json!(Uuid::now_v7());
        expect(&call(&app,"result",&auth,wrong).await?,StatusCode::FORBIDDEN,"result for another BFF session")?;
        expect(&call(&app,"result",&f.basic(),body.clone()).await?,StatusCode::FORBIDDEN,"profile credentials cannot consume selection")?;
        expect(&f.call("revoke",json!({"oidcAccessToken":f.access,"customerContext":token})).await?,StatusCode::OK,"old capability is revoked before access-token rotation")?;
        let renewed=f.select().await?;
        expect(&f.profile(&renewed).await?,StatusCode::OK,"renewal of the selected identity remains available")?;
        expect(&f.profile(token).await?,StatusCode::UNAUTHORIZED,"renewal never revives old capability")?;
        let associated:bool=sqlx::query_scalar("SELECT old.revoked_at IS NOT NULL AND renewed.revoked_at IS NULL AND old.id<>renewed.id AND old.bff_session_id=renewed.bff_session_id AND old.bff_context_version=renewed.bff_context_version AND old.oidc_grant_id=renewed.oidc_grant_id AND old.global_session_id=renewed.global_session_id AND old.membership_id=renewed.membership_id AND old.identity_version=renewed.identity_version AND (SELECT count(*)=1 FROM context_selections WHERE consumed_at IS NOT NULL) FROM customer_portal_contexts old JOIN customer_portal_contexts renewed ON renewed.token_hash=$2 WHERE old.token_hash=$1")
            .bind(hash::sha256_hex(token)).bind(hash::sha256_hex(&renewed)).fetch_one(&db.pool).await?;
        ensure!(associated,"renewal changed original selection/BFF/source or revived revoked capability");
        let old_session:Uuid=serde_json::from_value(body["bffSessionId"].clone())?;
        let access="d".repeat(64);let (sid,grant)=source(&db.pool,f.user,f.rp,&access).await?;
        let mut switched=issue(&f,&sources,service,Uuid::now_v7(),(&access,grant,sid),Intent::Switch{session_id:id(old_session),expected_version:ver(1)}).await?;
        switched["version"]=json!("2");switched["oldOidcAccessToken"]=json!(f.access);
        let new=call(&app,"consume",&auth,switched.clone()).await?;
        ensure!(new.status==StatusCode::OK,"customer switch failed: {} {}",new.status,new.body);
        expect(&f.profile(token).await?,StatusCode::UNAUTHORIZED,"old customer context was revoked")?;
        expect(&f.profile(&renewed).await?,StatusCode::UNAUTHORIZED,"all old renewed contexts were revoked")?;
        expect(&f.call("context",json!({"oidcAccessToken":f.access,"membershipId":f.customer,"organizationId":f.org,"expectedIdentityVersion":"5"})).await?,StatusCode::FORBIDDEN,"committed switch permanently fences old BFF renewal even while old grant is live")?;
        let recovered=call(&app,"result",&auth,switched.clone()).await?;
        ensure!(recovered.status==StatusCode::OK && recovered.body==new.body,"switch recovery changed the committed result");
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.customer).execute(&db.pool).await?;
        expect(&call(&app,"result",&auth,switched).await?,StatusCode::FORBIDDEN,"revoked member cannot recover a capability")?;
        Ok(())
    }.await;
    db.finish(result).await
}
