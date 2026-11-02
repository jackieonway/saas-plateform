//! Real PostgreSQL roles plus the platform HTTP resource boundary. The remote
//! proof responder is a bounded protocol fixture, not identity interoperability.
use super::*;
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use saas_platform_backend::context_selection::{
    ApplicationBinding, Binding, Id, Intent, Member, MemberType, Registration, Selection, Source,
    Version,
};
use serde_json::{json, Value};
use std::future::IntoFuture;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tower::ServiceExt;

mod recipient_inbox;

#[derive(Clone)]
struct Authority {
    reply: Arc<Mutex<Value>>,
    calls: Arc<AtomicUsize>,
    mode: Arc<AtomicUsize>,
    mutation: Arc<Mutex<Option<Mutation>>>,
}
enum Mutation {
    Member { pool: PgPool, id: Uuid, at: usize },
    Service { pool: PgPool, id: Uuid, at: usize },
    Source { at: usize },
}
async fn proof(
    State(authority): State<Authority>,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let count = authority.calls.fetch_add(1, Ordering::SeqCst) + 1;
    let valid = headers.get("authorization").and_then(|v| v.to_str().ok())
        == Some(&format!(
            "Basic {}",
            STANDARD.encode(format!(
                "{}:{}",
                crate::util::uuid7::fixture_uuid(99),
                "b".repeat(64)
            ))
        ))
        && input["contractVersion"] == "resource-access-proof-request/1"
        && input["token"] == "a".repeat(64)
        && input["resource"] == "stores"
        && input["audience"] == "stores"
        && matches!(
            input["scope"].as_str(),
            Some("stores:read" | "stores:notification:publish")
        );
    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"invalid_token"})),
        );
    }
    if authority.mode.load(Ordering::SeqCst) == 1 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"temporarily_unavailable"})),
        );
    }
    let mutation = {
        let mut pending = authority.mutation.lock().unwrap();
        let due = pending.as_ref().is_some_and(|m| match m {
            Mutation::Member { at, .. }
            | Mutation::Service { at, .. }
            | Mutation::Source { at } => *at == count,
        });
        if due {
            pending.take()
        } else {
            None
        }
    };
    let changed=match mutation {
        Some(Mutation::Member{pool,id,..})=>sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1").bind(id).execute(&pool).await.map(|_|()),
        Some(Mutation::Service{pool,id,..})=>sqlx::query("UPDATE platform_resource_services SET credential_version=credential_version+1 WHERE id=$1").bind(id).execute(&pool).await.map(|_|()),
        Some(Mutation::Source{..})=>{authority.reply.lock().unwrap()["source"]["grantId"]=json!(uuid7::new_uuid());Ok(())},
        None=>Ok(()),
    };
    if changed.is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"fixture_mutation_failed"})),
        );
    }
    (
        StatusCode::OK,
        Json(authority.reply.lock().unwrap().clone()),
    )
}

async fn directory_call(
    router: &Router,
    path: &str,
    authorization: &str,
    parameters: &[(&str, String)],
) -> Result<(StatusCode, Value)> {
    let mut encoded = reqwest::Url::parse("https://fixture.invalid/")?;
    encoded
        .query_pairs_mut()
        .extend_pairs(parameters.iter().map(|(key, value)| (*key, value.as_str())));
    let response = router
        .clone()
        .oneshot(
            Request::post(path)
                .header("authorization", authorization)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    encoded
                        .query()
                        .ok_or_else(|| anyhow!("missing directory form"))?
                        .to_owned(),
                ))?,
        )
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 65536).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

async fn call(
    router: &Router,
    authorization: &str,
    token: &str,
    resource: &str,
    action: &str,
) -> Result<(StatusCode, Value)> {
    let mut encoded = reqwest::Url::parse("https://fixture.invalid/")?;
    encoded.query_pairs_mut().extend_pairs([
        ("token", token),
        ("resource", resource),
        ("action", action),
    ]);
    let body = encoded
        .query()
        .ok_or_else(|| anyhow!("fixture form missing"))?
        .to_owned();
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/apps/stores/authorize")
                .header("authorization", authorization)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))?,
        )
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 65536).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase15_resource_http_enforces_private_service_user_and_machine_boundaries() -> Result<()>
{
    resource_http_matrix(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires explicit isolated IDENTITY_PHASE12_TEST_DATABASE_URL"]
async fn phase18_notification_http_retains_original_source_and_recipient_authority() -> Result<()> {
    resource_http_matrix(true).await
}

async fn resource_http_matrix(notifications: bool) -> Result<()> {
    let mut db = Database::create_with_namespace(
        "IDENTITY_PHASE12_TEST_DATABASE_URL",
        "identity_phase15_http_",
        "id15h_",
    )
    .await?;
    let authority = Authority {
        reply: Arc::new(Mutex::new(Value::Null)),
        calls: Arc::default(),
        mode: Arc::default(),
        mutation: Arc::default(),
    };
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(error) => return db.finish(Err(error.into())).await,
    };
    let origin = match listener.local_addr() {
        Ok(address) => format!("http://{address}"),
        Err(error) => return db.finish(Err(error.into())).await,
    };
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/internal/identity/auth/resource-access-proof",
                    axum::routing::post(proof),
                )
                .with_state(authority.clone()),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future(),
    );
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        let source=phase13::console_source(&db.pool,f.user).await?;
        let (caller_app,context_ref,interaction):(Uuid,Uuid,Uuid)=sqlx::query_as("SELECT c.application_id,g.context_ref,x.authorization_request_id FROM auth_grants g JOIN auth_clients c ON c.id=g.client_id JOIN auth_context_interactions x ON x.transaction_id=g.context_ref WHERE g.id=$1").bind(source.grant).fetch_one(&db.pool).await?;
        let now=Utc::now();let app=uuid7::new_uuid();let menu=uuid7::new_uuid();let service=uuid7::new_uuid();
        let contact=uuid7::new_uuid();let contact_member=uuid7::new_uuid();
        sqlx::query("INSERT INTO users(id,username,password_hash,real_name) VALUES($1,$2,'owned-no-login','Owned directory contact')").bind(contact).bind(format!("contact-{}",contact.simple())).execute(&db.pool).await?;
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE')").bind(contact_member).bind(contact).bind(f.org).execute(&db.pool).await?;
        sqlx::query("UPDATE organizations SET organization_type='STORE' WHERE id=$1").bind(f.org).execute(&db.pool).await?;

        sqlx::query("UPDATE users SET is_super_admin=true WHERE id=$1").bind(f.user).execute(&db.pool).await?;
        sqlx::query("INSERT INTO applications(id,name,code,base_path,entry_url) VALUES($1,'Owned resource target','stores','/stores','https://stores.test/index.html')").bind(app).execute(&db.pool).await?;
        sqlx::query("INSERT INTO applications(id,name,code,base_path) VALUES($1,'Owned OAuth caller',$2,'/caller')").bind(caller_app).bind(format!("caller-{}",caller_app.simple())).execute(&db.pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)").bind(f.tenant).bind(caller_app).execute(&db.pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(f.org).bind(caller_app).execute(&db.pool).await?;
        sqlx::query("INSERT INTO context_selection_registrations(client_id,client_registry,client_policy_version,member_type,application_id,application_policy_version) VALUES($1,'AUTH',1,'EMPLOYEE',$2,1)").bind(source.client).bind(caller_app).execute(&db.pool).await?;

        sqlx::query("INSERT INTO application_url_entries(application_id,environment,entry_url,revision) VALUES($1,'development','https://stores.test/index.html',$2)").bind(app).bind(uuid7::new_uuid()).execute(&db.pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)").bind(f.tenant).bind(app).execute(&db.pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(f.org).bind(app).execute(&db.pool).await?;
        sqlx::query("INSERT INTO menus(id,application_id,title,permission) VALUES($1,$2,'Owned read operation','store:read')").bind(menu).bind(app).execute(&db.pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)").bind(f.tenant).bind(menu).execute(&db.pool).await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)").bind(f.org).bind(menu).execute(&db.pool).await?;
        sqlx::query("INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,$3,'store','read','ORG')").bind(uuid7::new_uuid()).bind(f.member).bind(app).execute(&db.pool).await?;
        sqlx::query("INSERT INTO platform_resource_services(id,service_id,secret_hash,credential_version,purpose,application_id,audience,allowed_member_types,allowed_operations,status) VALUES($1,'owned-stores-resource',$2,1,'APP_RESOURCE',$3,'stores',ARRAY['EMPLOYEE'],ARRAY['store:read'],'ENABLED')").bind(service).bind(hash::sha256_hex(&"c".repeat(64))).bind(app).execute(&db.pool).await?;
        sqlx::query("INSERT INTO platform_resource_bindings(application_id,resource_code,action_code,canonical_resource,audience,oauth_scope,enabled) VALUES($1,'store','read','stores','stores','stores:read',true)").bind(app).execute(&db.pool).await?;
        let selection=Selection::confirm(Binding {transaction_id:Id::new(context_ref)?,source:Source{issuer:ISSUER.into(),user_id:Id::new(f.user)?,account_version:Version::new(3)?,global_session_id:Id::new(source.sid)?,interaction_id:Id::new(interaction)?,auth_time:DateTime::from_timestamp(source.auth_time,0).ok_or_else(||anyhow!("invalid fixture time"))?},registration:Registration{client_id:Id::new(source.client)?,client_policy_version:Version::new(1)?,member_type:MemberType::Employee,redirect_uri:"https://phase13.example.invalid/auth/callback".into(),application:Some(ApplicationBinding{id:Id::new(caller_app)?,policy_version:Version::new(1)?})},intent:Intent::Login{}},Member{user_id:Id::new(f.user)?,tenant_id:Id::new(f.tenant)?,organization_id:Id::new(f.org)?,membership_id:Id::new(f.member)?,member_type:MemberType::Employee,identity_version:Version::new(5)?},ISSUER,now,now+chrono::Duration::seconds(120),now+chrono::Duration::seconds(120))?;
        sqlx::query("INSERT INTO context_selections(transaction_id,interaction_id,client_id,credential_hash,credential_ciphertext,selection,created_at,expires_at) VALUES($1,$2,$3,$4,'owned-fixture',$5::jsonb,$6,$7)").bind(context_ref).bind(interaction).bind(source.client).bind(hash::sha256_hex(&context_ref.to_string())).bind(serde_json::to_string(&selection)?).bind(selection.created_at).bind(selection.expires_at).execute(&db.pool).await?;
        let baseline=json!({"contractVersion":"resource-access-proof/1","issuer":ISSUER,"tokenUse":"access_token","subjectMode":"USER","clientId":source.client,"applicationId":caller_app,"resource":"stores","audience":"stores","scope":["stores:read","stores:notification:publish"],"expiresAt":now+chrono::Duration::seconds(90),"sub":"opaque-do-not-parse-as-uuid","source":{"grantId":source.grant,"clientPolicyVersion":"1","applicationProjectionVersion":"1","globalSessionId":source.sid,"userId":f.user,"accountVersion":"3","authTime":source.auth_time,"contextRef":context_ref},"context":{"membershipId":f.member,"memberType":"EMPLOYEE"}});
        if notifications {
            bind_sso_fixture(&db.pool,&f).await?;
            let publish_menu=uuid7::new_uuid();
            sqlx::query("INSERT INTO menus(id,application_id,title,permission) VALUES($1,$2,'Owned publication','notification:publish')").bind(publish_menu).bind(app).execute(&db.pool).await?;
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)").bind(f.tenant).bind(publish_menu).execute(&db.pool).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)").bind(f.org).bind(publish_menu).execute(&db.pool).await?;
            sqlx::query("INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,$3,'notification','publish','ORG')").bind(uuid7::new_uuid()).bind(f.member).bind(app).execute(&db.pool).await?;
            sqlx::query("UPDATE platform_resource_services SET allowed_operations=ARRAY['store:read','notification:publish'] WHERE id=$1").bind(service).execute(&db.pool).await?;
            sqlx::query("INSERT INTO platform_resource_bindings(application_id,resource_code,action_code,canonical_resource,audience,oauth_scope,enabled) VALUES($1,'notification','publish','stores','stores','stores:notification:publish',true)").bind(app).execute(&db.pool).await?;
        }
        *authority.reply.lock().unwrap()=baseline.clone();
        db.provision().await?;
        let mut config=crate::config::Config::from_values(&Default::default())?;
        config.oidc_issuer=Some(ISSUER.into());config.identity_service_url=Some(origin);
        config.identity_http_client=Some(reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(2)).build()?);
        config.resource_proof_service_client_id=Some(crate::util::uuid7::fixture_uuid(99).to_string());config.resource_proof_service_client_secret=Some("b".repeat(64));
        config.data_grant_policy=crate::data_grants::policy::Policy::from_json(&json!({"applications":[{"applicationId":app,"capabilities":[{"resource":"store","action":"read","scopes":["ORG"]},{"resource":"notification","action":"publish","scopes":["ORG"]}],"memberLookups":[{"resource":"store","action":"read"}],"organizationLookups":[{"resource":"store","action":"read","organizationType":"STORE"}]}],"managementScopes":[]}).to_string())?;
        let state=crate::state::AppState::new(db.platform()?.clone(),Arc::new(config));
        let router=crate::routes::api_router_for_deployment(true).with_state(state);
        let authorization=format!("Basic {}",STANDARD.encode(format!("owned-stores-resource:{}","c".repeat(64))));
        let token="a".repeat(64);
        ensure!(call(&router,&authorization,&token,"store","read").await?.0==StatusCode::UNAUTHORIZED,"pending local selection entered business resource");
        sqlx::query("UPDATE context_selections SET consumed_at=clock_timestamp(),credential_ciphertext=NULL,result=$2::jsonb WHERE transaction_id=$1").bind(context_ref).bind(json!({"kind":"EMPLOYEE","contextId":f.sessions[0],"version":"1"}).to_string()).execute(&db.pool).await?;
        authority.calls.store(0,Ordering::SeqCst);
        let (status,body)=call(&router,&authorization,&token,"store","read").await?;
        ensure!(status==StatusCode::OK,"canonical USER resource HTTP failed: status={status} code={} fixture_authority_calls={}",body["code"],authority.calls.load(Ordering::SeqCst));
        ensure!(body["contractVersion"]=="resource-authorization/1" && body["subjectMode"]=="USER" && body["principal"]["userId"]==f.user.to_string() && body["context"]["contractVersion"]=="1.0" && body["introspection"]["sub"]=="opaque-do-not-parse-as-uuid","private/public identity boundary changed");
        ensure!(body["sourceApplicationId"]==caller_app.to_string() && body["targetApplicationId"]==app.to_string() && caller_app!=app,"caller and target application were conflated");
        ensure!(authority.calls.load(Ordering::SeqCst)==2,"positive authorization did not verify two online snapshots");
        if notifications {
            let inbox=recipient_inbox::create(&db.pool,db.platform()?,contact,f.tenant,f.org,contact_member).await?;
            notification_matrix(&router,&db.pool,&authority,&authorization,&token,&baseline,(contact,f.org,f.member,app,&inbox)).await?;
            let internal=crate::notifications::publish(db.platform()?,crate::notifications::Publisher{user_id:f.user,tenant_id:f.tenant,organization_id:f.org,application_id:app,session_id:f.sessions[0],token_hash:None,token_expires_at:Some(Utc::now()+chrono::Duration::seconds(60))},serde_json::from_value(json!({"idempotencyKey":"phase18-internal-current-source","target":{"kind":"USER","userId":contact,"organizationId":f.org},"type":"INFO","title":"Internal source still publishes","content":"","link":"/micro/stores/owned"}))?,"192.0.2.18","phase18-component").await?;
            ensure!(internal.delivery_count==1 && !internal.duplicate,"current internal canonical parent no longer publishes");
        }
        let directory_params=vec![("token",token.clone()),("resource","store".into()),("action","read".into()),("organizationId",f.org.to_string())];
        let members_path=format!("/internal/platform/apps/stores/members/{contact}");
        let (status,body)=directory_call(&router,&members_path,&authorization,&directory_params).await?;
        ensure!(status==StatusCode::OK && body["contractVersion"]=="resource-directory/1" && body["directory"]=="members" && body["data"]["id"]==contact.to_string(),"canonical private member directory failed: status={status} code={}",body["code"]);
        let directory_deadline=DateTime::parse_from_rfc3339(body["expiresAt"].as_str().ok_or_else(||anyhow!("missing directory expiry"))?)?;
        ensure!(directory_deadline<=Utc::now()+chrono::Duration::seconds(3),"private directory exceeded three-second window");
        let org_params=vec![("token",token.clone()),("resource","store".into()),("action","read".into())];
        let org_path=format!("/internal/platform/apps/stores/organizations/{}",f.org);
        let (status,body)=directory_call(&router,&org_path,&authorization,&org_params).await?;
        ensure!(status==StatusCode::OK && body["directory"]=="organizations" && body["data"]["organizationType"]=="STORE" && body["data"]["id"]==f.org.to_string(),"canonical private STORE directory failed: status={status} code={}",body["code"]);
        ensure!(directory_call(&router,&format!("{members_path}?token=forbidden"),&authorization,&directory_params).await?.0==StatusCode::BAD_REQUEST,"private directory accepted URL credentials");
        ensure!(directory_call(&router,&members_path,&format!("Bearer {token}"),&directory_params).await?.0==StatusCode::UNAUTHORIZED,"private directory accepted missing service identity");
        let trigger=authority.calls.load(Ordering::SeqCst)+2;
        *authority.mutation.lock().unwrap()=Some(Mutation::Service{pool:db.pool.clone(),id:service,at:trigger});
        ensure!(directory_call(&router,&members_path,&authorization,&directory_params).await?.0==StatusCode::UNAUTHORIZED,"service version changed during authority wait but directory was returned");
        let trigger=authority.calls.load(Ordering::SeqCst)+2;
        *authority.mutation.lock().unwrap()=Some(Mutation::Member{pool:db.pool.clone(),id:contact_member,at:trigger});
        let status=directory_call(&router,&members_path,&authorization,&directory_params).await?.0;
        ensure!(matches!(status,StatusCode::NOT_FOUND|StatusCode::SERVICE_UNAVAILABLE),"target shrank during authority wait but directory was returned: {status}");
        let before=authority.calls.load(Ordering::SeqCst);
        ensure!(call(&router,"Basic invalid",&token,"store","read").await?.0==StatusCode::UNAUTHORIZED,"unknown service admitted");
        ensure!(call(&router,&authorization,&token,"store","write").await?.0==StatusCode::FORBIDDEN,"unregistered operation admitted");
        ensure!(authority.calls.load(Ordering::SeqCst)==before,"unregistered service/operation reached identity");
        for path in ["/api/auth/profile","/api/memberships","/api/apps/stores/context"] {
            let response=router.clone().oneshot(Request::get(path).header("authorization",format!("Bearer {token}")).body(Body::empty())?).await?;
            ensure!(response.status()==StatusCode::UNAUTHORIZED,"external account token entered internal-only resource {path}");
        }
        ensure!(authority.calls.load(Ordering::SeqCst)==before,"internal-only endpoint implicitly mixed external authority");
        let registry_id=uuid7::new_uuid();
        sqlx::query("INSERT INTO platform_app_capabilities(token_hash,id,jti,user_id,tenant_id,organization_id,application_id,audience,scope,platform_session_id,membership_id,member_type,identity_version,account_version,application_policy_version,origin_issuer,origin_client_ref,origin_client_policy_version,issued_at,expires_at,revoked) VALUES($1,$2,$2,$3,$4,$5,$6,'stores','app:access',$7,$8,'EMPLOYEE',5,3,1,$9,$10,1,date_trunc('second',clock_timestamp()),date_trunc('second',clock_timestamp())+interval '60 seconds',true)")
            .bind(hash::sha256_hex(&token)).bind(registry_id).bind(f.user).bind(f.tenant).bind(f.org).bind(app).bind(f.sessions[0]).bind(f.member).bind(ISSUER).bind(source.client.to_string()).execute(&db.pool).await?;
        ensure!(call(&router,&authorization,&token,"store","read").await?.0==StatusCode::UNAUTHORIZED,"known revoked internal digest fell through to canonical authority");
        ensure!(authority.calls.load(Ordering::SeqCst)==before,"known internal credential reached canonical fallback");
        sqlx::query("DELETE FROM platform_app_capabilities WHERE id=$1").bind(registry_id).execute(&db.pool).await?;
        for (field,value) in [("tokenUse",json!("id_token")),("audience",json!("work-orders")),("scope",json!(["profile"])),("subjectMode",json!("ACCOUNT_ONLY"))] {
            let mut changed=baseline.clone();changed[field]=value;*authority.reply.lock().unwrap()=changed;
            ensure!(call(&router,&authorization,&token,"store","read").await?.0==StatusCode::UNAUTHORIZED,"wrong {field} admitted");
        }
        *authority.reply.lock().unwrap()=baseline.clone();
        let business_end=Utc::now()+chrono::Duration::seconds(15);
        sqlx::query("UPDATE tenants SET expires_at=$2 WHERE id=$1").bind(f.tenant).bind(business_end).execute(&db.pool).await?;
        let (status,body)=call(&router,&authorization,&token,"store","read").await?;
        ensure!(status==StatusCode::OK,"active short business deadline was rejected");
        for field in [&body["expiresAt"],&body["context"]["expiresAt"]] {let end=DateTime::parse_from_rfc3339(field.as_str().ok_or_else(||anyhow!("missing resource deadline"))?)?;ensure!(end<=business_end,"USER response extended business deadline");}
        ensure!(body["introspection"]["exp"]==baseline["expiresAt"].as_str().and_then(|value|DateTime::parse_from_rfc3339(value).ok()).map(|end|json!(end.timestamp())).unwrap_or(Value::Null),"resource clamp rewrote original token expiry");
        sqlx::query("UPDATE tenants SET expires_at=NULL WHERE id=$1").bind(f.tenant).execute(&db.pool).await?;
        sqlx::query("DELETE FROM membership_data_grants WHERE membership_id=$1 AND application_id=$2").bind(f.member).bind(app).execute(&db.pool).await?;
        ensure!(call(&router,&authorization,&token,"store","read").await?.0==StatusCode::FORBIDDEN,"scope without independent data grant admitted");
        sqlx::query("UPDATE context_selection_registrations SET enabled=false WHERE client_id=$1").bind(source.client).execute(&db.pool).await?;
        ensure!(call(&router,&authorization,&token,"store","read").await?.0==StatusCode::UNAUTHORIZED,"disabled source RP registration retained old selection");
        sqlx::query("UPDATE context_selection_registrations SET enabled=true WHERE client_id=$1").bind(source.client).execute(&db.pool).await?;
        ensure!(call(&router,&authorization,&token,"store","read").await?.0==StatusCode::UNAUTHORIZED,"re-enabling source RP revived old immutable selection");
        let mut machine=baseline.clone();machine["subjectMode"]=json!("MACHINE");machine.as_object_mut().unwrap().remove("sub");machine.as_object_mut().unwrap().remove("context");machine["credentialVersion"]=json!("4");machine["source"]=json!({"grantId":source.grant,"clientPolicyVersion":"1","applicationProjectionVersion":"1"});
        *authority.reply.lock().unwrap()=machine;
        ensure!(directory_call(&router,&members_path,&authorization,&directory_params).await?.0==StatusCode::FORBIDDEN,"MACHINE entered personnel directory");

        ensure!(call(&router,&authorization,&token,"store","read").await?.0==StatusCode::FORBIDDEN,"machine without explicit delegation admitted");
        let delegation_end=Utc::now()+chrono::Duration::seconds(15);
        sqlx::query("INSERT INTO platform_machine_delegations(id,client_id,source_application_id,credential_version,application_id,tenant_id,organization_id,resource,action,scope_type,organization_ids,version,status,expires_at) VALUES($1,$2,$3,4,$4,$5,$6,'store','read','ORG',ARRAY[$6],1,'ENABLED',$7)").bind(uuid7::new_uuid()).bind(source.client).bind(caller_app).bind(app).bind(f.tenant).bind(f.org).bind(delegation_end).execute(&db.pool).await?;
        let (status,body)=call(&router,&authorization,&token,"store","read").await?;
        ensure!(status==StatusCode::OK && body["subjectMode"]=="MACHINE" && body.get("principal").is_none() && body.get("context").is_none() && body.get("sub").is_none(),"machine result fabricated person or failed registered delegation");
        ensure!(DateTime::parse_from_rfc3339(body["expiresAt"].as_str().ok_or_else(||anyhow!("missing machine deadline"))?)?<=delegation_end,"MACHINE response extended delegation deadline");
        authority.mode.store(1,Ordering::SeqCst);
        ensure!(call(&router,&authorization,&token,"store","read").await?.0==StatusCode::SERVICE_UNAVAILABLE,"authority outage did not fail unavailable");
        db.verify().await?;
        Ok(())
    }.await;
    let _ = stop.send(());
    let cleanup = match tokio::time::timeout(Duration::from_secs(3), server).await {
        Ok(Ok(Ok(()))) => Ok(()),
        _ => Err(anyhow!("owned resource proof listener did not stop")),
    };
    db.finish(result.and(cleanup)).await
}

async fn notification_matrix(
    router: &Router,
    pool: &PgPool,
    authority: &Authority,
    authorization: &str,
    token: &str,
    baseline: &Value,
    ids: (Uuid, Uuid, Uuid, Uuid, &recipient_inbox::Inbox),
) -> Result<()> {
    let (contact, org, member, app, inbox) = ids;
    let publish = "/internal/platform/apps/stores/notifications/publish";
    let result = "/internal/platform/apps/stores/notifications/result";
    let request = json!({"idempotencyKey":"phase18-original-operation","target":{"kind":"USER","userId":contact,"organizationId":org},"type":"INFO","title":"Owned original reminder","content":"Source bound publication fixture","link":"/micro/stores/owned"});
    let form = |request: &Value| {
        vec![
            ("token", token.to_owned()),
            ("resource", "notification".into()),
            ("action", "publish".into()),
            ("request", request.to_string()),
        ]
    };
    let count = || {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM notification_publications")
            .fetch_one(pool)
    };
    let before = count().await?;
    let (status, body) = directory_call(router, result, authorization, &form(&request)).await?;
    ensure!(
        status == StatusCode::NOT_FOUND && body["code"] == "NOT_FOUND" && count().await? == before,
        "result endpoint created a publication or concealed explicit absence"
    );
    let (status, body) = directory_call(router, publish, authorization, &form(&request)).await?;
    ensure!(
        status == StatusCode::OK
            && body["contractVersion"] == "resource-notification/1"
            && body["publication"]["duplicate"] == false
            && body["publication"]["deliveryCount"] == 1,
        "private canonical USER publication failed: status={status} code={}",
        body["code"]
    );
    let publication = body["publication"]["publicationId"].clone();
    ensure!(
        DateTime::parse_from_rfc3339(
            body["expiresAt"]
                .as_str()
                .ok_or_else(|| anyhow!("missing private publication expiry"))?
        )? <= Utc::now() + chrono::Duration::seconds(3),
        "publication wrapper exceeded bounded authority lifetime"
    );
    for path in [publish, result] {
        let (status, body) = directory_call(router, path, authorization, &form(&request)).await?;
        ensure!(
            status == StatusCode::OK
                && body["publication"]["publicationId"] == publication
                && body["publication"]["duplicate"] == true,
            "original operation was not recovered exactly once"
        );
    }
    ensure!(
        count().await? == before + 1,
        "retry duplicated notification publication"
    );
    let stored: Value =
        sqlx::query_scalar("SELECT request_body FROM notification_publications WHERE id=$1")
            .bind(
                publication
                    .as_str()
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .ok_or_else(|| anyhow!("invalid publication id"))?,
            )
            .fetch_one(pool)
            .await?;
    ensure!(
        stored["authority"]["contractVersion"] == "resource-notification/1"
            && stored["authority"]["sourceDigest"]
                .as_str()
                .is_some_and(|digest| digest.len() == 64)
            && !stored.to_string().contains(token),
        "private source provenance was missing or stored raw credentials"
    );
    ensure!(
        stored["authority"]["recipientProofs"]
            .as_array()
            .is_some_and(|proofs| proofs.len() == 1
                && proofs[0]["userId"] == contact.to_string()
                && proofs[0]["organizationId"] == org.to_string()
                && proofs[0]["memberType"] == "EMPLOYEE"
                && proofs[0]["identityVersion"] == 5),
        "original recipient membership proof was not persisted in immutable publication authority"
    );
    let publication_id = publication
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or_else(|| anyhow!("invalid publication id"))?;
    let mut partial = request.clone();
    partial["idempotencyKey"] = json!("phase18-original-partial-inbox");
    partial["target"] = json!({"kind":"ORGANIZATION","organizationId":org});
    let (status, partial_body) =
        directory_call(router, publish, authorization, &form(&partial)).await?;
    ensure!(
        status == StatusCode::OK && partial_body["publication"]["deliveryCount"] == 2,
        "partial inbox fixture did not publish both original recipients"
    );
    let partial_id = partial_body["publication"]["publicationId"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or_else(|| anyhow!("invalid partial publication id"))?;
    for id in [publication_id, partial_id] {
        let notification: Uuid = sqlx::query_scalar(
            "SELECT id FROM notifications WHERE publication_id=$1 AND user_id=$2",
        )
        .bind(id)
        .bind(contact)
        .fetch_one(pool)
        .await?;
        inbox.remove(notification).await?;
    }
    for (original, expected, remaining) in [
        (&request, publication_id, 0_i64),
        (&partial, partial_id, 1_i64),
    ] {
        for path in [result, publish] {
            let (status, recovered) =
                directory_call(router, path, authorization, &form(original)).await?;
            ensure!(status==StatusCode::OK && recovered["publication"]["publicationId"]==expected.to_string() && recovered["publication"]["duplicate"]==true,"valid original recipient could not recover its receipt after public inbox deletion");
        }
        let actual: i64 =
            sqlx::query_scalar("SELECT count(*) FROM notifications WHERE publication_id=$1")
                .bind(expected)
                .fetch_one(pool)
                .await?;
        ensure!(
            actual == remaining,
            "receipt recovery recreated a deleted inbox delivery"
        );
    }
    let mut changed = request.clone();
    changed["title"] = json!("Changed operation payload");
    ensure!(
        directory_call(router, result, authorization, &form(&changed))
            .await?
            .0
            == StatusCode::CONFLICT,
        "result accepted altered original request"
    );
    let mut changed_source = baseline.clone();
    changed_source["source"]["grantId"] = json!(uuid7::new_uuid());
    *authority.reply.lock().unwrap() = changed_source;
    ensure!(
        directory_call(router, result, authorization, &form(&request))
            .await?
            .0
            == StatusCode::NOT_FOUND,
        "replacement source recovered an original operation"
    );
    *authority.reply.lock().unwrap() = baseline.clone();
    let trigger = authority.calls.load(Ordering::SeqCst) + 2;
    *authority.mutation.lock().unwrap() = Some(Mutation::Source { at: trigger });
    ensure!(
        directory_call(router, result, authorization, &form(&request))
            .await?
            .0
            == StatusCode::UNAUTHORIZED,
        "a source that changed between proof checkpoints recovered the original publication"
    );
    *authority.reply.lock().unwrap() = baseline.clone();
    ensure!(
        directory_call(
            router,
            &format!("{publish}?token=forbidden"),
            authorization,
            &form(&request)
        )
        .await?
        .0 == StatusCode::BAD_REQUEST,
        "private publication admitted URL token"
    );
    ensure!(
        directory_call(router, publish, &format!("Bearer {token}"), &form(&request))
            .await?
            .0
            == StatusCode::UNAUTHORIZED,
        "private publication omitted service authentication"
    );
    let mut extra = form(&request);
    extra.push(("token", token.to_owned()));
    ensure!(
        directory_call(router, publish, authorization, &extra)
            .await?
            .0
            == StatusCode::BAD_REQUEST,
        "duplicate token form accepted"
    );
    let mut extra = form(&request);
    extra.push(("unknown", "value".into()));
    ensure!(
        directory_call(router, publish, authorization, &extra)
            .await?
            .0
            == StatusCode::BAD_REQUEST,
        "unknown private publication form accepted"
    );
    let mut missing_scope = baseline.clone();
    missing_scope["scope"] = json!(["stores:read"]);
    *authority.reply.lock().unwrap() = missing_scope;
    ensure!(
        directory_call(router, publish, authorization, &form(&request))
            .await?
            .0
            == StatusCode::UNAUTHORIZED,
        "read scope became notification publish scope"
    );
    let mut customer = baseline.clone();
    customer["context"]["memberType"] = json!("CUSTOMER");
    *authority.reply.lock().unwrap() = customer;
    ensure!(
        directory_call(router, publish, authorization, &form(&request))
            .await?
            .0
            == StatusCode::FORBIDDEN,
        "CUSTOMER published employee notifications"
    );
    let mut machine = baseline.clone();
    machine["subjectMode"] = json!("MACHINE");
    machine.as_object_mut().unwrap().remove("sub");
    machine.as_object_mut().unwrap().remove("context");
    machine["credentialVersion"] = json!("1");
    machine["source"] = json!({"grantId":uuid7::new_uuid(),"clientPolicyVersion":"1","applicationProjectionVersion":"1"});
    *authority.reply.lock().unwrap() = machine;
    ensure!(
        directory_call(router, publish, authorization, &form(&request))
            .await?
            .0
            == StatusCode::FORBIDDEN,
        "MACHINE published personnel notifications"
    );
    *authority.reply.lock().unwrap() = baseline.clone();
    sqlx::query("DELETE FROM membership_data_grants WHERE membership_id=$1 AND application_id=$2 AND resource_code='notification'").bind(member).bind(app).execute(pool).await?;
    ensure!(
        directory_call(router, result, authorization, &form(&request))
            .await?
            .0
            == StatusCode::FORBIDDEN,
        "result recovery bypassed independent data grant shrink"
    );
    sqlx::query("INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,$3,'notification','publish','ORG')").bind(uuid7::new_uuid()).bind(member).bind(app).execute(pool).await?;
    let recipient:Uuid=sqlx::query_scalar("SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2 AND member_type='EMPLOYEE'").bind(contact).bind(org).fetch_one(pool).await?;
    sqlx::query(
        "UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1",
    )
    .bind(recipient)
    .execute(pool)
    .await?;
    for original in [&request, &partial] {
        for path in [result, publish] {
            let (status, denied) =
                directory_call(router, path, authorization, &form(original)).await?;
            ensure!(status==StatusCode::FORBIDDEN && denied["code"]!="NOT_FOUND","deleted inbox erased the immutable recipient revocation fence: path={path}, status={status}");
        }
    }
    ensure!(
        directory_call(router, result, authorization, &form(&request))
            .await?
            .0
            == StatusCode::FORBIDDEN,
        "result recovery returned a revoked recipient"
    );
    sqlx::query(
        "UPDATE memberships SET status='ENABLED',identity_version=identity_version+1 WHERE id=$1",
    )
    .bind(recipient)
    .execute(pool)
    .await?;
    for original in [&request, &partial] {
        for path in [result, publish] {
            ensure!(
                directory_call(router, path, authorization, &form(original))
                    .await?
                    .0
                    == StatusCode::FORBIDDEN,
                "new recipient generation revived the original immutable receipt authority"
            );
        }
    }
    let mut next = request.clone();
    next["idempotencyKey"] = json!("phase18-source-wait-shrink");
    let trigger = authority.calls.load(Ordering::SeqCst) + 2;
    *authority.mutation.lock().unwrap() = Some(Mutation::Member {
        pool: pool.clone(),
        id: recipient,
        at: trigger,
    });
    ensure!(
        directory_call(router, publish, authorization, &form(&next))
            .await?
            .0
            == StatusCode::NOT_FOUND,
        "recipient changed during source wait but publication committed"
    );
    sqlx::query(
        "UPDATE memberships SET status='ENABLED',identity_version=identity_version+1 WHERE id=$1",
    )
    .bind(recipient)
    .execute(pool)
    .await?;
    authority.mode.store(1, Ordering::SeqCst);
    ensure!(
        directory_call(router, result, authorization, &form(&request))
            .await?
            .0
            == StatusCode::SERVICE_UNAVAILABLE,
        "unknown authority result became permission or success"
    );
    authority.mode.store(0, Ordering::SeqCst);
    ensure!(
        count().await? == before + 2,
        "negative private publication scenarios wrote or replayed notifications"
    );
    println!("phase18_private_notification_matrix: original operation/result/opaque USER/source namespace/scope/data-grant/service/form/customer-machine/recipient-shrink/public-inbox-delete-all-and-partial/immutable-recipient-recovery/outage PASS");
    Ok(())
}
