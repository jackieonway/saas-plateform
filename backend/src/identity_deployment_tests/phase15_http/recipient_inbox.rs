//! Public inbox deletion with real canonical source rows and an explicit
//! account-proof/UserInfo transport fixture; no retired authentication routes.
use super::*;

pub(super) struct Inbox {
    router: Router,
    token: String,
}

impl Inbox {
    pub(super) async fn remove(&self, notification: Uuid) -> Result<()> {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::delete(format!("/api/notifications/{notification}"))
                    .header("authorization", format!("Bearer {}", self.token))
                    .body(Body::empty())?,
            )
            .await?;
        let status = response.status();
        let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 65_536).await?)?;
        ensure!(
            status == StatusCode::OK && body["ok"] == true,
            "owned public recipient deletion failed: status={status}, code={}",
            body["code"]
        );
        Ok(())
    }
}

struct RecipientAuthority {
    proof: Value,
    access: String,
    authorization: String,
}

impl crate::bff::OidcTransport for RecipientAuthority {
    fn send<'a>(&'a self, request: crate::bff::OidcRequest) -> crate::bff::OidcFuture<'a> {
        Box::pin(async move {
            let body = match request.endpoint {
                crate::bff::Endpoint::AccountProof
                    if request.authorization.as_deref() == Some(&self.authorization)
                        && request.json.as_ref()
                            == Some(
                                &json!({"contractVersion":"platform-account-proof-request/1","oidcAccessToken":self.access,"expectedClientId":self.proof["clientId"]}),
                            ) =>
                {
                    self.proof.clone()
                }
                crate::bff::Endpoint::UserInfo
                    if request.authorization.as_deref()
                        == Some(&format!("Bearer {}", self.access)) =>
                {
                    json!({"sub":self.proof["sub"]})
                }
                _ => return Err(crate::error::AppError::Unavailable),
            };
            Ok(crate::bff::OidcResponse {
                status: 200,
                body: serde_json::to_vec(&body).map_err(|_| crate::error::AppError::Unavailable)?,
            })
        })
    }
}

pub(super) async fn create(
    owner: &PgPool,
    runtime: &PgPool,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    member: Uuid,
) -> Result<Inbox> {
    sqlx::query("UPDATE users SET session_version=3 WHERE id=$1")
        .bind(user)
        .execute(owner)
        .await?;
    sqlx::query("UPDATE memberships SET identity_version=5 WHERE id=$1")
        .bind(member)
        .execute(owner)
        .await?;
    let sid = uuid7::new_uuid();
    sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) VALUES($1,$2,$3,$4,clock_timestamp()+interval '1 hour',3,$5,'EMPLOYEE',5,'urn:saas:legacy-platform','platform-console-legacy',1)")
        .bind(sid).bind(user).bind(tenant).bind(org).bind(member).execute(owner).await?;
    bind_sso_fixture(
        owner,
        &Fixture {
            user,
            tenant,
            org,
            member,
            sessions: vec![sid],
        },
    )
    .await?;
    let (bff,client,global,grant,auth_time,end,subject):(Uuid,Uuid,Uuid,Uuid,i64,DateTime<Utc>,String)=sqlx::query_as("SELECT id,oidc_client_id,global_session_id,oidc_grant_id,auth_time,access_expires_at,oidc_public_sub FROM bff_sessions WHERE selected_session_id=$1").bind(sid).fetch_one(owner).await?;
    let access = hash::sha256_hex(&uuid7::new_uuid().to_string());
    let keys = Arc::new(crate::bff::CipherKeys::fixture());
    sqlx::query("UPDATE bff_sessions SET access_ciphertext=$2 WHERE id=$1")
        .bind(bff)
        .bind(keys.fixture_access(bff, ISSUER, client, &access))
        .execute(owner)
        .await?;
    let mut config = crate::config::Config::from_values(&Default::default())?;
    config.bff_oidc_issuer = Some(ISSUER.into());
    config.bff_oidc_client_id = Some(client);
    config.bff_token_keys = Some(keys);
    config.context_source_service_client_id = Some("owned-recipient-source".into());
    config.context_source_service_client_secret = Some("e".repeat(64));
    let authorization = format!(
        "Basic {}",
        STANDARD.encode(format!("owned-recipient-source:{}", "e".repeat(64)))
    );
    let now = Utc::now().timestamp();
    let token = crate::util::jwt::sign_internal(
        &crate::util::jwt::Claims {
            sub: user.to_string(),
            typ: "platform_session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            cid: None,
            scope: None,
            sid: Some(sid.to_string()),
            ver: Some(3),
            aud: Some(crate::util::jwt::PLATFORM_API_AUDIENCE.into()),
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        crate::util::jwt::InternalUse::PlatformSession,
        &config.jwt_secret,
    )?;
    let mut state = crate::state::AppState::new(runtime.clone(), Arc::new(config));
    state.bff_transport = Arc::new(RecipientAuthority {
        access,
        authorization,
        proof: json!({"contractVersion":"platform-account-proof/1","issuer":ISSUER,"clientId":client,"clientPurpose":"PLATFORM_CONSOLE","contextMode":"PLATFORM_CONTEXT","allowedMemberTypes":["EMPLOYEE"],"scope":["openid","profile"],"expiresAt":end,"absoluteExpiresAt":end,"sub":subject,"userId":user,"globalSessionId":global,"grantId":grant,"accountVersion":"3","clientPolicyVersion":"1","authTime":auth_time}),
    });
    Ok(Inbox {
        router: crate::routes::notifications::routes().with_state(state),
        token,
    })
}
