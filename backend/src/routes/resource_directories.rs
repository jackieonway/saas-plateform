//! Private, service-authenticated directory requests. Access tokens stay in the
//! bounded POST form; public Bearer routes never gain an external-token fallback.
use axum::{
    extract::{rejection::FormRejection, DefaultBodyLimit, Form, Path, Request, State},
    http::{header, HeaderMap, HeaderValue, Uri},
    middleware::{self, Next},
    response::Response,
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    app_organizations::{self, LookupQuery},
    directory_authorization::{Access, PrivateAccess, Purpose},
    error::AppError,
    routes::app_members::{self, MemberQuery},
    state::AppState,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/platform/apps/{appCode}/members", post(members))
        .route(
            "/internal/platform/apps/{appCode}/members/{userId}",
            post(member),
        )
        .route(
            "/internal/platform/apps/{appCode}/organizations",
            post(organizations),
        )
        .route(
            "/internal/platform/apps/{appCode}/organizations/{organizationId}",
            post(organization),
        )
        .layer(DefaultBodyLimit::max(32 * 1024))
        .layer(middleware::from_fn(private_response))
}

async fn private_response(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemberForm {
    token: String,
    resource: String,
    action: String,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    organization_id: Uuid,
    page: Option<i64>,
    size: Option<i64>,
    q: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OrganizationForm {
    token: String,
    resource: String,
    action: String,
    page: Option<i64>,
    size: Option<i64>,
    q: Option<String>,
}

fn private_uri(uri: &Uri) -> Result<(), AppError> {
    if uri.query().is_some() {
        return Err(AppError::bad_request("私有目录参数只能出现在请求表单"));
    }
    Ok(())
}

fn reply(directory: &str, data: Value, deadline: DateTime<Utc>) -> Result<Json<Value>, AppError> {
    if deadline <= Utc::now() {
        return Err(AppError::Unavailable);
    }
    Ok(Json(
        json!({"contractVersion":"resource-directory/1","directory":directory,
        "expiresAt":deadline.to_rfc3339_opts(chrono::SecondsFormat::Millis,true),"data":data}),
    ))
}

async fn member_lookup(
    state: &AppState,
    code: &str,
    headers: HeaderMap,
    uri: Uri,
    form: Result<Form<MemberForm>, FormRejection>,
    target: Option<Uuid>,
) -> Result<Json<Value>, AppError> {
    private_uri(&uri)?;
    let Form(input) = form.map_err(|_| AppError::bad_request("私有成员目录表单无效"))?;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let access = PrivateAccess::authenticate(
            state,
            code,
            headers,
            input.token,
            &input.resource,
            &input.action,
            Purpose::Members,
        )
        .await?;
        let result = app_members::lookup(
            state,
            code,
            Access::Private(&access),
            MemberQuery {
                resource: input.resource,
                action: input.action,
                organization_id: input.organization_id,
                page: input.page,
                size: input.size,
                q: input.q,
            },
            target,
        )
        .await?;
        let deadline = access.expires_at()?;
        let deadline = result
            .expires_at
            .map_or(deadline, |expiry| expiry.min(deadline));
        reply("members", result.data, deadline)
    })
    .await
    .map_err(|_| AppError::Unavailable)?
}

async fn organization_lookup(
    state: &AppState,
    code: &str,
    headers: HeaderMap,
    uri: Uri,
    form: Result<Form<OrganizationForm>, FormRejection>,
    target: Option<Uuid>,
) -> Result<Json<Value>, AppError> {
    private_uri(&uri)?;
    let Form(input) = form.map_err(|_| AppError::bad_request("私有组织目录表单无效"))?;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let access = PrivateAccess::authenticate(
            state,
            code,
            headers,
            input.token,
            &input.resource,
            &input.action,
            Purpose::Organizations,
        )
        .await?;
        let data = app_organizations::lookup_with_access(
            state,
            code,
            Access::Private(&access),
            LookupQuery {
                resource: input.resource,
                action: input.action,
                page: input.page,
                size: input.size,
                q: input.q,
            },
            target,
        )
        .await?;
        let data_deadline = data["expiresAt"]
            .as_str()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
            .ok_or(AppError::Unavailable)?;
        reply(
            "organizations",
            data,
            access.expires_at()?.min(data_deadline),
        )
    })
    .await
    .map_err(|_| AppError::Unavailable)?
}

async fn members(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    form: Result<Form<MemberForm>, FormRejection>,
) -> Result<Json<Value>, AppError> {
    member_lookup(&state, &code, headers, uri, form, None).await
}
async fn member(
    State(state): State<AppState>,
    Path((code, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    uri: Uri,
    form: Result<Form<MemberForm>, FormRejection>,
) -> Result<Json<Value>, AppError> {
    member_lookup(&state, &code, headers, uri, form, Some(id)).await
}
async fn organizations(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    form: Result<Form<OrganizationForm>, FormRejection>,
) -> Result<Json<Value>, AppError> {
    organization_lookup(&state, &code, headers, uri, form, None).await
}
async fn organization(
    State(state): State<AppState>,
    Path((code, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    uri: Uri,
    form: Result<Form<OrganizationForm>, FormRejection>,
) -> Result<Json<Value>, AppError> {
    organization_lookup(&state, &code, headers, uri, form, Some(id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_directory_disallows_query_credentials() {
        assert!(private_uri(&"/internal/platform/apps/stores/members".parse().unwrap()).is_ok());
        assert!(private_uri(
            &"/internal/platform/apps/stores/members?token=opaque"
                .parse()
                .unwrap()
        )
        .is_err());
    }
}
