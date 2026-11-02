use super::{assert_actor, global_operator, manifest_value, policy, require_application, store};
use crate::{error::AppError, extractors::CurrentUser, state::AppState};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

struct Command<'a> {
    tx: Transaction<'a, Postgres>,
    state: &'a AppState,
    user: &'a CurrentUser,
    headers: &'a HeaderMap,
    operation: Uuid,
    digest: String,
    permission: &'static str,
    application: Option<Uuid>,
}
impl<'a> Command<'a> {
    async fn begin(
        state: &'a AppState,
        user: &'a CurrentUser,
        headers: &'a HeaderMap,
        path: &str,
        body: &Value,
        permission: &'static str,
        application: Option<Uuid>,
    ) -> Result<Self, AppError> {
        let mut tx = store::management_tx(state, user, permission, true).await?;
        if let Some(application) = application {
            require_application(&mut tx, user, application).await?;
        }
        Ok(Self {
            tx,
            state,
            user,
            headers,
            operation: store::operation(headers)?,
            digest: store::digest(path, body)?,
            permission,
            application,
        })
    }
    async fn replay(&mut self) -> Result<Option<Value>, AppError> {
        store::replay(&mut self.tx, self.user, self.operation, &self.digest).await
    }
    async fn global(&mut self) -> Result<(), AppError> {
        if global_operator(&mut self.tx, self.user).await? {
            Ok(())
        } else {
            Err(AppError::forbidden(
                "此操作只允许平台接入管理人员，合作方委派不授予全局管理权",
            ))
        }
    }
    async fn finish(
        mut self,
        mut result: Value,
        action: &str,
        request: Option<Uuid>,
        remote: bool,
    ) -> Result<Value, AppError> {
        result["operationId"] = json!(self.operation);
        if remote {
            result["syncState"] = json!("PENDING");
        }
        assert_actor(&mut self.tx, self.state, self.user, self.permission, true).await?;
        if let Some(application) = self.application {
            require_application(&mut self.tx, self.user, application).await?;
        }
        store::record(
            &mut self.tx,
            self.user,
            self.headers,
            store::Record {
                operation: self.operation,
                digest: &self.digest,
                action,
                permission: self.permission,
                application: self.application,
                request,
                remote,
                result: &result,
            },
        )
        .await?;
        self.tx.commit().await?;
        Ok(result)
    }
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
struct Partner {
    id: Uuid,
    name: String,
    contact_name: String,
    contact_channel: String,
    subject_reference: Uuid,
    status: String,
    version: i64,
}
#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub(super) struct Request {
    pub id: Uuid,
    pub application_id: Uuid,
    pub environment: String,
    pub manifest: Value,
    pub requested_by: Uuid,
    pub submitted_by: Uuid,
    pub status: String,
    pub reviewed_by: Option<Uuid>,
    pub published_by: Option<Uuid>,
    pub reviewed_contact_reference: Option<Uuid>,
    pub reviewed_domain_reference: Option<Uuid>,
    pub test_evidence_reference: Option<Uuid>,
    pub reason: String,
    pub version: i64,
    pub synced_version: i64,
    pub identity_projection_version: Option<i64>,
    pub identity_admission_version: Option<i64>,
}
const REQUEST:&str="SELECT id,application_id,environment,manifest,requested_by,submitted_by,status,reviewed_by,published_by,reviewed_contact_reference,reviewed_domain_reference,test_evidence_reference,reason,version,synced_version,identity_projection_version,identity_admission_version FROM platform_auth_application_requests";
fn value<T: Serialize>(value: T) -> Result<Value, AppError> {
    serde_json::to_value(value).map_err(|_| AppError::Unavailable)
}
fn input<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, AppError> {
    serde_json::from_value(value.clone())
        .map_err(|_| AppError::bad_request("接入管理字段或类型无效"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PartnerInput {
    name: String,
    contact_name: String,
    contact_channel: String,
    subject_reference: String,
}
pub(super) async fn partner_create(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    body: Value,
) -> Result<Value, AppError> {
    let mut command = Command::begin(
        state,
        user,
        headers,
        "/api/identity/review/partners",
        &body,
        policy::REGISTER,
        None,
    )
    .await?;
    command.global().await?;
    if let Some(result) = command.replay().await? {
        return Ok(result);
    }
    let parsed: PartnerInput = input(&body)?;
    let row:Partner=sqlx::query_as("INSERT INTO platform_auth_partners(id,name,contact_name,contact_channel,subject_reference,created_by) VALUES($1,$2,$3,$4,$5,$6) RETURNING id,name,contact_name,contact_channel,subject_reference,status,version")
        .bind(command.operation).bind(policy::bounded_text(&parsed.name,120)?).bind(policy::bounded_text(&parsed.contact_name,80)?).bind(policy::bounded_text(&parsed.contact_channel,256)?).bind(policy::evidence(&parsed.subject_reference)?).bind(user.user_id).fetch_one(&mut *command.tx).await?;
    command
        .finish(value(row)?, "PARTNER_CREATE", None, false)
        .await
}
pub(super) async fn partners(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let mut tx = store::management_tx(state, user, policy::VIEW, false).await?;
    let global = global_operator(&mut tx, user).await?;
    let rows:Vec<Partner>=sqlx::query_as("SELECT id,name,contact_name,contact_channel,subject_reference,status,version FROM platform_auth_partners partner WHERE $1 OR EXISTS(SELECT 1 FROM platform_auth_applications app JOIN platform_auth_application_operators operator USING(application_id) WHERE app.partner_id=partner.id AND operator.user_id=$2) ORDER BY created_at DESC,id DESC LIMIT 100")
        .bind(global).bind(user.user_id).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(json!({"partners":rows}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplicationInput {
    partner_id: String,
    display_name: String,
}
pub(super) async fn application_create(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    body: Value,
) -> Result<Value, AppError> {
    let mut command = Command::begin(
        state,
        user,
        headers,
        "/api/identity/review/applications",
        &body,
        policy::REGISTER,
        None,
    )
    .await?;
    command.global().await?;
    if let Some(result) = command.replay().await? {
        return Ok(result);
    }
    let parsed: ApplicationInput = input(&body)?;
    let partner = policy::id(&parsed.partner_id)?;
    let name = policy::bounded_text(&parsed.display_name, 120)?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM platform_auth_partners WHERE id=$1 AND status='ACTIVE')",
    )
    .bind(partner)
    .fetch_one(&mut *command.tx)
    .await?;
    if !active {
        return Err(AppError::forbidden("合作方尚未登记或已暂停"));
    }
    sqlx::query("INSERT INTO platform_auth_applications(application_id,partner_id,display_name,created_by) VALUES($1,$2,$3,$4)").bind(command.operation).bind(partner).bind(&name).bind(user.user_id).execute(&mut *command.tx).await?;
    let result = json!({"applicationId":command.operation,"partnerId":partner,"displayName":name,"createdBy":user.user_id,"requests":[]});
    command
        .finish(result, "APPLICATION_CREATE", None, false)
        .await
}
pub(super) async fn applications(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let mut tx = store::management_tx(state, user, policy::VIEW, false).await?;
    let global = global_operator(&mut tx, user).await?;
    let rows:Vec<(Uuid,Uuid,String,Uuid)>=sqlx::query_as("SELECT application_id,partner_id,display_name,created_by FROM platform_auth_applications app WHERE $1 OR EXISTS(SELECT 1 FROM platform_auth_application_operators operator WHERE operator.application_id=app.application_id AND operator.user_id=$2) ORDER BY created_at DESC,application_id DESC LIMIT 100")
        .bind(global).bind(user.user_id).fetch_all(&mut *tx).await?;
    let mut records = Vec::with_capacity(rows.len());
    for (application, partner, name, creator) in rows {
        let requests: Vec<Request> = sqlx::query_as(&format!(
            "{REQUEST} WHERE application_id=$1 ORDER BY environment"
        ))
        .bind(application)
        .fetch_all(&mut *tx)
        .await?;
        records.push(json!({"applicationId":application,"partnerId":partner,"displayName":name,"createdBy":creator,"requests":requests}));
    }
    let ready = production_ready(&mut tx).await?;
    let menus = crate::authorization::context_menu_rows_in_tx(&mut tx, user, None).await?;
    let permissions = crate::authorization::permissions_of(&menus);
    tx.commit().await?;
    Ok(json!({"applications":records,"productionReady":ready,"permissions":permissions}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Submission {
    environment: String,
    expected_version: Option<i64>,
    manifest: policy::Manifest,
}
pub(super) async fn submit(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    application: Uuid,
    body: Value,
) -> Result<Value, AppError> {
    let path = format!("/api/identity/review/applications/{application}/requests");
    let mut command = Command::begin(
        state,
        user,
        headers,
        &path,
        &body,
        policy::REGISTER,
        Some(application),
    )
    .await?;
    if let Some(result) = command.replay().await? {
        return Ok(result);
    }
    let parsed: Submission = input(&body)?;
    let environment = policy::environment(&parsed.environment)?;
    let manifest = manifest_value(parsed.manifest)?;
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_auth_applications app JOIN platform_auth_partners partner ON partner.id=app.partner_id WHERE app.application_id=$1 AND partner.status='ACTIVE')").bind(application).fetch_one(&mut *command.tx).await?;
    if !exists {
        return Err(AppError::not_found("受控合作方应用不存在或已暂停"));
    }
    let previous:Option<(Uuid,i64)>=sqlx::query_as("SELECT id,version FROM platform_auth_application_requests WHERE application_id=$1 AND environment=$2 FOR UPDATE").bind(application).bind(environment).fetch_optional(&mut *command.tx).await?;
    let request = if let Some((id, version)) = previous {
        if parsed.expected_version != Some(version) {
            return Err(AppError::conflict(
                "申请版本已变化，请载入最新资料后重新提交",
            ));
        }
        sqlx::query("UPDATE platform_auth_application_requests SET manifest=$2,submitted_by=$3,status='SUBMITTED',reviewed_by=NULL,published_by=NULL,reviewed_contact_reference=NULL,reviewed_domain_reference=NULL,test_evidence_reference=NULL,reason='',version=version+1,remote_operation_id=NULL,updated_at=clock_timestamp() WHERE id=$1")
            .bind(id).bind(&manifest).bind(user.user_id).execute(&mut *command.tx).await?;
        id
    } else {
        if parsed.expected_version.is_some() {
            return Err(AppError::conflict("首次申请不能携带旧版本"));
        }
        sqlx::query("INSERT INTO platform_auth_application_requests(id,application_id,environment,manifest,requested_by,submitted_by) VALUES($1,$2,$3,$4,$5,$5)")
            .bind(command.operation).bind(application).bind(environment).bind(&manifest).bind(user.user_id).execute(&mut *command.tx).await?;
        command.operation
    };
    let row: Request = sqlx::query_as(&format!("{REQUEST} WHERE id=$1"))
        .bind(request)
        .fetch_one(&mut *command.tx)
        .await?;
    command
        .finish(value(row)?, "APPLICATION_SUBMIT", Some(request), false)
        .await
}

pub(super) async fn production_ready(tx: &mut Transaction<'_, Postgres>) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_auth_release_bindings binding WHERE singleton AND enabled AND verified_at<=clock_timestamp() AND cardinality(reviewer_user_ids)>0 AND cardinality(publisher_user_ids)>0 AND NOT EXISTS(SELECT 1 FROM unnest(reviewer_user_ids||publisher_user_ids) AS responsible(user_id) LEFT JOIN users subject ON subject.id=responsible.user_id WHERE subject.id IS NULL OR subject.status<>'ENABLED'))").fetch_one(&mut **tx).await?)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Decision {
    expected_version: i64,
    status: String,
    reason: String,
    verified_contact_reference: Option<String>,
    verified_domain_reference: Option<String>,
    test_evidence_reference: Option<String>,
}
pub(super) async fn decide(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    request: Uuid,
    body: Value,
) -> Result<(Value, bool), AppError> {
    let parsed: Decision = input(&body)?;
    let snapshot:Option<(Uuid,String,String)>=sqlx::query_as("SELECT application_id,status,environment FROM platform_auth_application_requests WHERE id=$1").bind(request).fetch_optional(&state.pool).await?;
    let (application, from, environment) =
        snapshot.ok_or_else(|| AppError::not_found("申请不存在"))?;
    let permission = match parsed.status.as_str() {
        "ENABLED" => policy::PUBLISH,
        "SUSPENDED" => policy::DISABLE,
        "RETURNED" | "TEST_APPROVED" | "TEST_ACCEPTED" | "PRODUCTION_APPROVED" => policy::REVIEW,
        _ => return Err(AppError::bad_request("审核目标状态无效")),
    };
    let path = format!("/api/identity/review/requests/{request}/decisions");
    let mut command = Command::begin(
        state,
        user,
        headers,
        &path,
        &body,
        permission,
        Some(application),
    )
    .await?;
    if let Some(result) = command.replay().await? {
        return Ok((result, false));
    }
    policy::transition(&from, &parsed.status, &environment)?;
    let row: Request = sqlx::query_as(&format!("{REQUEST} WHERE id=$1 FOR UPDATE"))
        .bind(request)
        .fetch_one(&mut *command.tx)
        .await?;
    if row.version != parsed.expected_version || row.status != from {
        return Err(AppError::conflict("审核申请版本或状态已变化"));
    }
    let creator: Uuid = sqlx::query_scalar(
        "SELECT created_by FROM platform_auth_applications WHERE application_id=$1",
    )
    .bind(application)
    .fetch_one(&mut *command.tx)
    .await?;
    if parsed.status != "SUSPENDED"
        && (user.user_id == row.requested_by
            || user.user_id == row.submitted_by
            || user.user_id == creator)
    {
        return Err(AppError::forbidden(
            "申请、最近提交或应用登记人员不能审核或启用自己的申请",
        ));
    }
    let reason = policy::bounded_text(&parsed.reason, 1000)?;
    let mut contact = row.reviewed_contact_reference;
    let mut domain = row.reviewed_domain_reference;
    if matches!(
        parsed.status.as_str(),
        "TEST_APPROVED" | "PRODUCTION_APPROVED"
    ) {
        contact = Some(policy::evidence(
            parsed
                .verified_contact_reference
                .as_deref()
                .ok_or_else(|| AppError::bad_request("审核必须核对联系人证据引用"))?,
        )?);
        domain = Some(policy::evidence(
            parsed
                .verified_domain_reference
                .as_deref()
                .ok_or_else(|| AppError::bad_request("审核必须核对域名控制证据引用"))?,
        )?);
        if contact.map(|id| id.to_string()).as_deref() != row.manifest["contactReference"].as_str()
            || domain.map(|id| id.to_string()).as_deref()
                != row.manifest["domainEvidenceReference"].as_str()
        {
            return Err(AppError::conflict(
                "审核证据必须对应当前申请；证据变化应重新提交资料",
            ));
        }
    }
    let evidence = if parsed.status == "TEST_ACCEPTED" {
        Some(policy::evidence(
            parsed
                .test_evidence_reference
                .as_deref()
                .ok_or_else(|| AppError::bad_request("测试验收需要当前证据引用"))?,
        )?)
    } else {
        row.test_evidence_reference
    };
    if parsed.status == "TEST_ACCEPTED" && row.synced_version != row.version {
        return Err(AppError::conflict(
            "当前测试登记版本尚未由身份权威确认，不能验收",
        ));
    }
    if matches!(parsed.status.as_str(), "PRODUCTION_APPROVED" | "ENABLED") {
        if !production_ready(&mut command.tx).await? {
            return Err(AppError::forbidden(
                "实际审核/发布责任及运营数据政策尚未绑定，生产入口保持关闭",
            ));
        }
        let bound:bool=sqlx::query_scalar("SELECT CASE WHEN $2 THEN $1=ANY(publisher_user_ids) ELSE $1=ANY(reviewer_user_ids) END FROM platform_auth_release_bindings WHERE singleton")
            .bind(user.user_id).bind(parsed.status=="ENABLED").fetch_one(&mut *command.tx).await?;
        if !bound {
            return Err(AppError::forbidden(
                "当前账号不是已核实绑定的生产审核或发布人员",
            ));
        }
        let tested:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_auth_application_requests WHERE application_id=$1 AND environment='test' AND status='TEST_ACCEPTED' AND synced_version IN(version-1,version) AND identity_projection_version IS NOT NULL AND identity_admission_version IS NOT NULL AND test_evidence_reference IS NOT NULL)").bind(application).fetch_one(&mut *command.tx).await?;
        if !tested {
            return Err(AppError::conflict(
                "同一应用须先完成当前测试登记与有证据的测试验收",
            ));
        }
    }
    let remote = matches!(
        parsed.status.as_str(),
        "TEST_APPROVED" | "ENABLED" | "SUSPENDED"
    );
    sqlx::query("UPDATE platform_auth_application_requests SET status=$2,reason=$3,reviewed_by=CASE WHEN $2 IN('TEST_APPROVED','PRODUCTION_APPROVED') THEN $4 ELSE reviewed_by END,published_by=CASE WHEN $2='ENABLED' THEN $4 ELSE published_by END,reviewed_contact_reference=$5,reviewed_domain_reference=$6,test_evidence_reference=$7,version=version+1,remote_operation_id=CASE WHEN $8 THEN $9 ELSE remote_operation_id END,updated_at=clock_timestamp() WHERE id=$1")
        .bind(request).bind(&parsed.status).bind(reason).bind(user.user_id).bind(contact).bind(domain).bind(evidence).bind(remote).bind(command.operation).execute(&mut *command.tx).await?;
    let row: Request = sqlx::query_as(&format!("{REQUEST} WHERE id=$1"))
        .bind(request)
        .fetch_one(&mut *command.tx)
        .await?;
    Ok((
        command
            .finish(value(row)?, "APPLICATION_DECISION", Some(request), remote)
            .await?,
        remote,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Assignment {
    user_id: String,
}
pub(super) async fn assign(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    application: Uuid,
    body: Value,
) -> Result<Value, AppError> {
    let path = format!("/api/identity/review/applications/{application}/operators");
    let mut command = Command::begin(
        state,
        user,
        headers,
        &path,
        &body,
        "system:oauth:assign",
        Some(application),
    )
    .await?;
    command.global().await?;
    if let Some(result) = command.replay().await? {
        return Ok(result);
    }
    let input: Assignment = input(&body)?;
    let target = policy::id(&input.user_id)?;
    let eligible:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memberships member JOIN users subject ON subject.id=member.user_id WHERE member.user_id=$1 AND member.organization_id=$2 AND member.member_type='EMPLOYEE' AND member.status='ENABLED' AND subject.status='ENABLED')").bind(target).bind(user.org_id).fetch_one(&mut *command.tx).await?;
    if !eligible {
        return Err(AppError::forbidden(
            "委派目标必须已有当前平台根组织的有效员工身份；合作方关系不会创建租户管理员",
        ));
    }
    sqlx::query("INSERT INTO platform_auth_application_operators(application_id,user_id,assigned_by) VALUES($1,$2,$3) ON CONFLICT(application_id,user_id) DO NOTHING").bind(application).bind(target).bind(user.user_id).execute(&mut *command.tx).await?;
    command
        .finish(
            json!({"applicationId":application,"userId":target}),
            "APPLICATION_ASSIGN",
            None,
            false,
        )
        .await
}

pub(super) async fn security(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let mut tx = state.pool.begin().await?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    // The own-security bootstrap view must remain available before enrollment;
    // it never reads application data or authorizes a management mutation.
    let allowed: bool = sqlx::query_scalar(
        "SELECT platform_identity_management_actor_active($1,$2,$3,false,false)",
    )
    .bind(user.user_id)
    .bind(user.session_id)
    .bind(policy::VIEW)
    .fetch_one(&mut *tx)
    .await?;
    if !allowed {
        return Err(AppError::forbidden("当前员工没有统一接入管理查看权限"));
    }
    let assurance:Option<(bool,Option<String>,bool)>=sqlx::query_as("SELECT proof.totp_enabled,proof.mfa_method,COALESCE(proof.mfa_verified_at BETWEEN clock_timestamp()-interval '5 minutes' AND clock_timestamp(),false) FROM platform_sessions s JOIN bff_sessions b ON b.id=s.bff_session_id CROSS JOIN LATERAL identity_admin_assurance(b.user_id,b.global_session_id,b.oidc_grant_id,b.account_version,b.oidc_client_id,b.oidc_client_policy_version,b.issuer) proof WHERE s.id=$1")
        .bind(user.session_id).fetch_optional(&mut *tx).await?;
    let (enabled, method, recent) = assurance.unwrap_or((false, None, false));
    let ready = production_ready(&mut tx).await?;
    let menus = crate::authorization::context_menu_rows_in_tx(&mut tx, user, None).await?;
    let permissions = crate::authorization::permissions_of(&menus);
    tx.commit().await?;
    let url = state
        .config
        .bff_oidc_issuer
        .as_ref()
        .map(|issuer| format!("{issuer}/sso/security"))
        .unwrap_or_default();
    Ok(
        json!({"totpEnabled":enabled,"recent":enabled&&method.as_deref()==Some("totp")&&recent,"securityUrl":url,"reauthenticateUrl":"/auth/step-up?return=/system/oauth/clients","productionReady":ready,"permissions":permissions}),
    )
}

fn sync_permission(row: &Request) -> Result<&'static str, AppError> {
    match (row.status.as_str(), row.environment.as_str()) {
        ("SUSPENDED", _) => Ok(policy::DISABLE),
        ("TEST_APPROVED" | "TEST_ACCEPTED", "test") => Ok(policy::REVIEW),
        ("ENABLED", "production") => Ok(policy::PUBLISH),
        _ => Err(AppError::conflict(
            "该申请尚不允许同步为有效接入；生产须先明确批准并由发布人员启用",
        )),
    }
}
pub(super) async fn sync_context(
    state: &AppState,
    user: &CurrentUser,
    request: Uuid,
) -> Result<(Uuid, String, &'static str), AppError> {
    let row: Request = sqlx::query_as(&format!("{REQUEST} WHERE id=$1"))
        .bind(request)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("申请不存在"))?;
    let permission = sync_permission(&row)?;
    let mut tx = store::management_tx(state, user, permission, true).await?;
    require_application(&mut tx, user, row.application_id).await?;
    tx.commit().await?;
    Ok((row.application_id, row.environment, permission))
}

pub(super) struct SyncPlan {
    pub operation: Uuid,
    pub payload: Option<Value>,
    pub result: Value,
}

pub(super) async fn prepare_sync(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    request: Uuid,
    body: Option<Value>,
    remote: &Value,
) -> Result<SyncPlan, AppError> {
    let row: Request = sqlx::query_as(&format!("{REQUEST} WHERE id=$1"))
        .bind(request)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("申请不存在"))?;
    let permission = sync_permission(&row)?;
    let operation = store::operation(headers)?;
    let path = format!("/api/identity/review/requests/{request}/sync");
    let mut tx = store::management_tx(state, user, permission, true).await?;
    require_application(&mut tx, user, row.application_id).await?;
    let current: Request = sqlx::query_as(&format!("{REQUEST} WHERE id=$1 FOR UPDATE"))
        .bind(request)
        .fetch_one(&mut *tx)
        .await?;
    if current.version != row.version || current.status != row.status {
        return Err(AppError::conflict("同步期间申请版本发生变化"));
    }
    let mut local_fact = None;
    if let Some(body) = body {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Expected {
            expected_version: i64,
        }
        let expected: Expected = input(&body)?;
        let digest = store::digest(&path, &body)?;
        if let Some(result) = store::replay(&mut tx, user, operation, &digest).await? {
            tx.commit().await?;
            return Ok(SyncPlan {
                operation,
                payload: None,
                result,
            });
        }
        if expected.expected_version != current.version {
            return Err(AppError::conflict("同步目标不是当前审核版本"));
        }
        let previous:Option<(Uuid,Option<Value>)>=sqlx::query_as("SELECT op.operation_id,op.remote_result FROM platform_auth_application_requests q JOIN platform_auth_review_operations op ON op.operation_id=q.remote_operation_id WHERE q.id=$1 AND op.remote_required")
            .bind(request).fetch_optional(&mut *tx).await?;
        if previous.is_some_and(|(previous, result)| previous != operation && result.is_none()) {
            return Err(AppError::conflict(
                "原同步结果仍未知；请由发起会话先查询原操作，不能更换编号重放",
            ));
        }
        if current.synced_version == current.version {
            let mut result = value(current)?;
            result["operationId"] = json!(operation);
            store::record(
                &mut tx,
                user,
                headers,
                store::Record {
                    operation,
                    digest: &digest,
                    action: "APPLICATION_SYNC_CONFIRMED",
                    permission,
                    application: Some(row.application_id),
                    request: Some(request),
                    remote: false,
                    result: &result,
                },
            )
            .await?;
            tx.commit().await?;
            return Ok(SyncPlan {
                operation,
                payload: None,
                result,
            });
        }
        sqlx::query(
            "UPDATE platform_auth_application_requests SET remote_operation_id=$2 WHERE id=$1",
        )
        .bind(request)
        .bind(operation)
        .execute(&mut *tx)
        .await?;
        local_fact = Some(digest);
    } else {
        let own:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_auth_review_operations op JOIN platform_auth_application_requests q ON q.id=op.request_id WHERE op.operation_id=$1 AND op.actor_user_id=$2 AND op.actor_session_id=$3 AND op.remote_required AND q.remote_operation_id=op.operation_id AND q.id=$4)")
            .bind(operation).bind(user.user_id).bind(user.session_id).bind(request).fetch_one(&mut *tx).await?;
        if !own {
            return Err(AppError::conflict("原审核事实与当前发起会话不一致"));
        }
    }
    let partner: Uuid = sqlx::query_scalar(
        "SELECT partner_id FROM platform_auth_applications WHERE application_id=$1",
    )
    .bind(row.application_id)
    .fetch_one(&mut *tx)
    .await?;
    let reviewed = current
        .reviewed_by
        .ok_or_else(|| AppError::conflict("当前版本没有独立审核人员"))?;
    let contact = current
        .reviewed_contact_reference
        .ok_or_else(|| AppError::conflict("当前版本没有联系人核验证据"))?;
    let domain = current
        .reviewed_domain_reference
        .ok_or_else(|| AppError::conflict("当前版本没有域名核验证据"))?;
    let mut result = value(&current)?;
    result["operationId"] = json!(operation);
    result["syncState"] = json!("PENDING");
    let payload = json!({"applicationId":current.application_id,"environment":current.environment,"requestId":request,"requestVersion":current.version,
        "actorUserId":user.user_id,"actorSessionId":user.session_id,"expectedProjectionVersion":remote["projectionVersion"],"expectedAdmissionVersion":remote["admissionVersion"],
        "partnerId":partner,"manifest":current.manifest,"requestedBy":current.requested_by,"reviewedBy":reviewed,"publishedBy":current.published_by,
        "contactReference":contact,"domainReference":domain,"status":if current.status=="SUSPENDED"{"SUSPENDED"}else{"APPROVED"}});
    if let Some(digest) = local_fact {
        store::record(
            &mut tx,
            user,
            headers,
            store::Record {
                operation,
                digest: &digest,
                action: "APPLICATION_SYNC",
                permission,
                application: Some(row.application_id),
                request: Some(request),
                remote: true,
                result: &result,
            },
        )
        .await?;
    }
    assert_actor(&mut tx, state, user, permission, true).await?;
    tx.commit().await?;
    Ok(SyncPlan {
        operation,
        payload: Some(payload),
        result,
    })
}

/// Return the exact approved proposal even if another proposal is submitted
/// while Identity is processing this operation. No fresh read permission is
/// needed after a mutation has committed and its result is already authorized.
pub(super) fn synchronized_result(mut result: Value, remote: &Value) -> Result<Value, AppError> {
    if result["id"] != remote["requestId"] || result["version"] != remote["requestVersion"] {
        return Err(AppError::Unavailable);
    }
    result["syncedVersion"] = remote["requestVersion"].clone();
    result["identityProjectionVersion"] = remote["projectionVersion"].clone();
    result["identityAdmissionVersion"] = remote["admissionVersion"].clone();
    result["syncState"] = json!("COMPLETED");
    Ok(result)
}
