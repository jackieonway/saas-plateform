//! Minimal direct-member view for business assignment, protected by the current
//! application's operation permission and independent resource data grant.
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::bearer_token;
use crate::routes::app_context;
use crate::state::AppState;
use crate::util::jwt;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/apps/{appCode}/members", get(list))
        .route("/api/apps/{appCode}/members/{userId}", get(one))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemberQuery {
    resource: String,
    action: String,
    organization_id: Uuid,
    page: Option<i64>,
    size: Option<i64>,
    q: Option<String>,
}

fn validate(query: &MemberQuery, single: bool) -> Result<(i64, i64, String), AppError> {
    if query.resource != "ticket" || !matches!(query.action.as_str(), "create" | "update") {
        return Err(AppError::bad_request(
            "成员辅助仅支持 ticket/create 或 ticket/update",
        ));
    }
    if single && (query.page.is_some() || query.size.is_some() || query.q.is_some()) {
        return Err(AppError::bad_request("单成员校验不接受分页或搜索参数"));
    }
    let page = query.page.unwrap_or(1);
    let size = query.size.unwrap_or(20);
    let q = query.q.as_deref().unwrap_or("").trim();
    if !(1..=1_000_000).contains(&page) || !(1..=50).contains(&size) || q.chars().count() > 100 {
        return Err(AppError::bad_request("分页或搜索参数超出范围"));
    }
    let pattern = if q.is_empty() {
        String::new()
    } else {
        format!(
            "%{}%",
            q.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    };
    Ok((page, size, pattern))
}

fn uuid_of(ctx: &Value, parent: &str) -> Result<Uuid, AppError> {
    ctx[parent]["id"]
        .as_str()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| AppError::unauthorized("应用上下文无效"))
}

async fn authorized(
    state: &AppState,
    app_code: &str,
    headers: &HeaderMap,
    query: &MemberQuery,
) -> Result<(Uuid, Uuid, Uuid, Uuid), AppError> {
    let token = bearer_token(headers)?;
    let claims = jwt::verify(&token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("应用令牌无效"))?;
    if claims.exp <= chrono::Utc::now().timestamp() {
        return Err(AppError::unauthorized("应用令牌已到期"));
    }
    let ctx = app_context::current_context(state, app_code, headers).await?;
    let user = uuid_of(&ctx, "user")?;
    let tenant = uuid_of(&ctx, "tenant")?;
    let org = uuid_of(&ctx, "organization")?;
    let app = uuid_of(&ctx, "application")?;
    let permission = format!("ticket:{}", query.action);
    if !ctx["permissions"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|value| value.as_str() == Some(&permission))
    }) {
        return Err(AppError::forbidden("缺少当前应用的工单操作权限"));
    }
    let grant = ctx["dataGrants"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["resource"] == "ticket"
                    && item["action"] == query.action
                    && item["subjectUserId"].as_str() == Some(user.to_string().as_str())
                    && item["tenantId"].as_str() == Some(tenant.to_string().as_str())
            })
        })
        .ok_or_else(|| AppError::forbidden("缺少 ticket 操作的独立数据授权"))?;
    let allowed = match grant["scope"].as_str() {
        Some("SELF" | "ORG") => query.organization_id == org,
        Some("ORG_SUBTREE") => grant["organizationIds"].as_array().is_some_and(|ids| {
            ids.iter().any(|id| {
                id.as_str().and_then(|id| id.parse::<Uuid>().ok()) == Some(query.organization_id)
            })
        }),
        Some("TENANT") => true,
        _ => false,
    };
    if !allowed {
        return Err(AppError::not_found("组织不存在或不在授权范围内"));
    }
    let valid_org: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM organizations o \
         JOIN tenants t ON t.id=o.tenant_id AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>now()) \
         JOIN applications a ON a.id=$3 AND a.status='ENABLED' \
         JOIN tenant_applications ta ON ta.tenant_id=t.id AND ta.application_id=a.id \
         JOIN organization_applications oa ON oa.organization_id=o.id AND oa.application_id=a.id \
         WHERE o.id=$1 AND o.tenant_id=$2 AND org_chain_active(o.id))"
    ).bind(query.organization_id).bind(tenant).bind(app).fetch_one(&state.pool).await?;
    if !valid_org {
        return Err(AppError::not_found("组织不存在或不能访问当前应用"));
    }
    Ok((user, tenant, org, app))
}

async fn list(
    State(state): State<AppState>,
    Path(app_code): Path<String>,
    headers: HeaderMap,
    Query(query): Query<MemberQuery>,
) -> Result<Json<Value>, AppError> {
    let (page, size, pattern) = validate(&query, false)?;
    let identity = authorized(&state, &app_code, &headers, &query).await?;
    let predicate = "m.organization_id=$1 AND u.status='ENABLED' \
        AND ($2='' OR COALESCE(NULLIF(trim(u.real_name),''),u.username) ILIKE $2 ESCAPE CHR(92))";
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM memberships m JOIN users u ON u.id=m.user_id WHERE {predicate}"
    ))
    .bind(query.organization_id)
    .bind(&pattern)
    .fetch_one(&state.pool)
    .await?;
    let rows: Vec<(Uuid, String)> = sqlx::query_as(&format!(
        "SELECT u.id,COALESCE(NULLIF(trim(u.real_name),''),u.username) FROM memberships m JOIN users u ON u.id=m.user_id \
         WHERE {predicate} ORDER BY u.id LIMIT $3 OFFSET $4"))
        .bind(query.organization_id).bind(&pattern).bind(size).bind((page - 1) * size).fetch_all(&state.pool).await?;
    if authorized(&state, &app_code, &headers, &query).await? != identity {
        return Err(AppError::unauthorized("应用上下文已改变"));
    }
    Ok(Json(
        json!({ "records": rows.into_iter().map(|(id, display)|
        json!({"id":id,"displayName":display,"organizationId":query.organization_id})).collect::<Vec<_>>(),
        "total":total,"page":page,"size":size }),
    ))
}

async fn one(
    State(state): State<AppState>,
    Path((app_code, user_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Query(query): Query<MemberQuery>,
) -> Result<Json<Value>, AppError> {
    validate(&query, true)?;
    let identity = authorized(&state, &app_code, &headers, &query).await?;
    let display: String = sqlx::query_scalar(
        "SELECT COALESCE(NULLIF(trim(u.real_name),''),u.username) FROM memberships m JOIN users u ON u.id=m.user_id \
         WHERE m.organization_id=$1 AND m.user_id=$2 AND u.status='ENABLED'"
    ).bind(query.organization_id).bind(user_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::not_found("有效直接成员不存在"))?;
    if authorized(&state, &app_code, &headers, &query).await? != identity {
        return Err(AppError::unauthorized("应用上下文已改变"));
    }
    Ok(Json(
        json!({"id":user_id,"displayName":display,"organizationId":query.organization_id}),
    ))
}
