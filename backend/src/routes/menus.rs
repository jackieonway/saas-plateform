use crate::util::uuid7::ParseUuid7;
use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::state::AppState;
use crate::util::{audit, tree};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/menus/tree", get(tree_handler))
        .route("/api/menus", post(create))
        .route("/api/menus/{id}", put(update).delete(delete))
}

/// 列序与 MENU_SELECT / RETURNING 严格一致: …, permission, sort_order, visible, link_url, link_target
type MenuRowTuple = (
    Uuid,
    Uuid,
    Option<Uuid>,
    String,
    String,
    String,
    String,
    String,
    String,
    i32,
    bool,
    String,
    String,
);

const MENU_SELECT: &str = "SELECT m.id, m.application_id, m.parent_id, m.type, m.title, COALESCE(m.path,'') AS path, \
 COALESCE(m.icon,'') AS icon, COALESCE(m.component,'') AS component, COALESCE(m.permission,'') AS permission, \
 m.sort_order, m.visible, m.link_url, m.link_target";

fn menu_json(r: &MenuRowTuple) -> Value {
    let (
        id,
        application_id,
        parent_id,
        mtype,
        title,
        path,
        icon,
        component,
        permission,
        sort_order,
        visible,
        link_url,
        link_target,
    ) = r;
    json!({
        "id": id, "applicationId": application_id, "parentId": parent_id, "type": mtype,
        "title": title, "path": path, "icon": icon, "component": component,
        "permission": permission, "sortOrder": sort_order, "visible": visible,
        "linkUrl": link_url, "linkTarget": link_target,
    })
}

fn menu_summary(row: &MenuRowTuple) -> Value {
    json!({"id": row.0, "applicationId": row.1, "parentId": row.2,
           "type": row.3, "name": row.4, "permission": row.8,
           "sortOrder": row.9, "visible": row.10})
}

async fn lock_menu(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<MenuRowTuple, AppError> {
    // Serialize hierarchy mutations for one application before locking a menu.
    // Locking only the edited rows lets two concurrent moves create a cycle.
    let application_id: Uuid = sqlx::query_scalar("SELECT application_id FROM menus WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::not_found("菜单不存在"))?;
    lock_menu_application(tx, application_id).await?;
    sqlx::query_as(&format!(
        "{MENU_SELECT} FROM menus m WHERE m.id=$1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::not_found("菜单不存在"))
}

async fn lock_menu_application(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    application_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR NO KEY UPDATE")
        .bind(application_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::bad_request("所属应用不存在"))?;
    Ok(())
}

fn parse_parent_id(raw: Option<&str>) -> Result<Option<Uuid>, AppError> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse_uuid7()
                .map_err(|_| AppError::bad_request("parentId 格式错误"))
        })
        .transpose()
}

async fn validate_parent(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    application_id: Uuid,
    menu_id: Option<Uuid>,
    parent_id: Option<Uuid>,
) -> Result<(), AppError> {
    let Some(parent_id) = parent_id else {
        return Ok(());
    };
    let parent_application: Uuid =
        sqlx::query_scalar("SELECT application_id FROM menus WHERE id=$1")
            .bind(parent_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| AppError::bad_request("父菜单不存在"))?;
    if parent_application != application_id {
        return Err(AppError::bad_request("父菜单必须属于同一应用"));
    }
    if let Some(menu_id) = menu_id {
        // UNION also terminates safely for a pre-existing corrupt hierarchy.
        let creates_cycle: bool = sqlx::query_scalar(
            "WITH RECURSIVE ancestors(id,parent_id) AS ( \
             SELECT id,parent_id FROM menus WHERE id=$1 \
             UNION SELECT m.id,m.parent_id FROM menus m JOIN ancestors a ON m.id=a.parent_id) \
             SELECT EXISTS(SELECT 1 FROM ancestors WHERE id=$2)",
        )
        .bind(parent_id)
        .bind(menu_id)
        .fetch_one(&mut **tx)
        .await?;
        if creates_cycle {
            return Err(AppError::bad_request("父菜单不能是自身或下级菜单"));
        }
    }
    Ok(())
}

fn parent_update<'de, D>(de: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

/// 校验并归一外链地址: 空 = 非外链; 非空必须 http(s):// 开头且 ≤ 500 字符(与 VARCHAR(500) 同口径)
fn normalize_link_url(raw: &str) -> Result<String, AppError> {
    let url = raw.trim();
    if url.is_empty() {
        return Ok(String::new());
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::bad_request(
            "外链地址必须以 http:// 或 https:// 开头",
        ));
    }
    if url.chars().count() > 500 {
        return Err(AppError::bad_request("外链地址长度不能超过 500 字符"));
    }
    Ok(url.to_string())
}

/// 归一打开方式: BLANK(新窗口) / SELF(内部嵌入) / MICRO(微前端容器路由, menu.path 填 /micro/{appCode}),
/// 非法或缺省回退 BLANK (迁移 0011 新增 MICRO 语义枚举, DB 无需改列)
fn normalize_link_target(raw: Option<&str>) -> String {
    match raw {
        Some("SELF") => "SELF".to_string(),
        Some("MICRO") => "MICRO".to_string(),
        _ => "BLANK".to_string(),
    }
}

/// GET /api/menus/tree?applicationId= — 管理端全量菜单树 (不受授权过滤)
async fn tree_handler(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:menu")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let app_id: Uuid = params
        .get("applicationId")
        .and_then(|v| crate::util::uuid7::parse_uuid(v).ok())
        .ok_or_else(|| AppError::bad_request("缺少 applicationId 参数"))?;
    let rows: Vec<MenuRowTuple> = sqlx::query_as(&format!(
        "{MENU_SELECT} FROM menus m WHERE m.application_id = $1 ORDER BY m.sort_order, m.created_at"
    ))
    .bind(app_id)
    .fetch_all(&state.pool)
    .await?;
    let items: Vec<Value> = rows.iter().map(menu_json).collect();
    Ok(Json(json!({"menus": tree::build_tree(items)})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MenuBody {
    application_id: String,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default = "default_menu_type")]
    r#type: String,
    title: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    component: String,
    #[serde(default)]
    permission: String,
    #[serde(default)]
    sort_order: i32,
    #[serde(default = "default_visible")]
    visible: bool,
    #[serde(default)]
    link_url: String,
    #[serde(default)]
    link_target: String,
}

fn default_menu_type() -> String {
    "MENU".into()
}

fn default_visible() -> bool {
    true
}

fn parse_menu_type(t: &str) -> Result<String, AppError> {
    match t {
        "DIR" | "MENU" | "BUTTON" => Ok(t.to_string()),
        _ => Err(AppError::bad_request("菜单类型仅支持 DIR / MENU / BUTTON")),
    }
}

/// POST /api/menus
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<MenuBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:menu")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let app_id: Uuid = body
        .application_id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("applicationId 格式错误"))?;
    let parent_id = parse_parent_id(body.parent_id.as_deref())?;
    let title = body.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::bad_request("菜单标题不能为空"));
    }
    let mtype = parse_menu_type(&body.r#type)?;
    let link_url = normalize_link_url(&body.link_url)?;
    let link_target = normalize_link_target(Some(&body.link_target));
    let mut tx = state.pool.begin().await?;
    lock_menu_application(&mut tx, app_id).await?;
    validate_parent(&mut tx, app_id, None, parent_id).await?;
    let row: MenuRowTuple = sqlx::query_as(
        "INSERT INTO menus (application_id, parent_id, type, title, path, icon, component, permission, sort_order, visible, link_url, link_target, id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
         RETURNING id, application_id, parent_id, type, title, COALESCE(path,'') AS path, \
         COALESCE(icon,'') AS icon, COALESCE(component,'') AS component, COALESCE(permission,'') AS permission, \
         sort_order, visible, link_url, link_target",
    )
    .bind(app_id)
    .bind(parent_id)
    .bind(&mtype)
    .bind(&title)
    .bind(&body.path)
    .bind(&body.icon)
    .bind(&body.component)
    .bind(&body.permission)
    .bind(body.sort_order)
    .bind(body.visible)
    .bind(&link_url)
    .bind(&link_target)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(&mut *tx)
    .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("menu", row.0, Value::Null, menu_summary(&row));
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "CREATE_MENU",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(menu_json(&row)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MenuUpdate {
    #[serde(default, deserialize_with = "parent_update")]
    parent_id: Option<Option<String>>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    component: Option<String>,
    #[serde(default)]
    permission: Option<String>,
    #[serde(default)]
    sort_order: Option<i32>,
    #[serde(default)]
    visible: Option<bool>,
    #[serde(default)]
    link_url: Option<String>,
    #[serde(default)]
    link_target: Option<String>,
}

/// PUT /api/menus/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<MenuUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:menu")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    if let Some(t) = &body.r#type {
        parse_menu_type(t)?;
    }
    if let Some(title) = &body.title {
        if title.trim().is_empty() {
            return Err(AppError::bad_request("菜单标题不能为空"));
        }
    }
    let parent_id = body
        .parent_id
        .as_ref()
        .map(|value| parse_parent_id(value.as_deref()))
        .transpose()?;
    let link_url = match &body.link_url {
        Some(u) => Some(normalize_link_url(u)?),
        None => None,
    };
    // 打开方式仅 BLANK/SELF/MICRO, 非法值回退 BLANK(不报错, 与创建口径一致)
    let link_target = body
        .link_target
        .as_deref()
        .map(|t| normalize_link_target(Some(t)));
    let mut tx = state.pool.begin().await?;
    let before = lock_menu(&mut tx, id).await?;
    if let Some(parent_id) = parent_id {
        validate_parent(&mut tx, before.1, Some(id), parent_id).await?;
    }
    let row: MenuRowTuple = sqlx::query_as(
        "UPDATE menus SET parent_id = CASE WHEN $13 THEN $2::uuid ELSE parent_id END, type = COALESCE($3, type), \
         title = COALESCE($4, title), path = COALESCE($5, path), icon = COALESCE($6, icon), \
         component = COALESCE($7, component), permission = COALESCE($8, permission), \
         sort_order = COALESCE($9, sort_order), visible = COALESCE($10, visible), \
         link_url = COALESCE($11, link_url), link_target = COALESCE($12, link_target), updated_at = now() \
         WHERE id = $1 \
         RETURNING id, application_id, parent_id, type, title, COALESCE(path,'') AS path, \
         COALESCE(icon,'') AS icon, COALESCE(component,'') AS component, COALESCE(permission,'') AS permission, \
         sort_order, visible, link_url, link_target",
    )
    .bind(id)
    .bind(parent_id.flatten())
    .bind(&body.r#type)
    .bind(body.title.as_deref().map(str::trim))
    .bind(&body.path)
    .bind(&body.icon)
    .bind(&body.component)
    .bind(&body.permission)
    .bind(body.sort_order)
    .bind(body.visible)
    .bind(&link_url)
    .bind(&link_target)
    .bind(parent_id.is_some())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("菜单不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let changed_fields: Vec<&str> = [
        ("parentId", before.2 != row.2),
        ("type", before.3 != row.3),
        ("title", before.4 != row.4),
        ("path", before.5 != row.5),
        ("icon", before.6 != row.6),
        ("component", before.7 != row.7),
        ("permission", before.8 != row.8),
        ("sortOrder", before.9 != row.9),
        ("visible", before.10 != row.10),
        ("linkUrl", before.11 != row.11),
        ("linkTarget", before.12 != row.12),
    ]
    .into_iter()
    .filter_map(|(field, changed)| changed.then_some(field))
    .collect();
    if !changed_fields.is_empty() {
        let mut after = menu_summary(&row);
        after["changedFields"] = json!(changed_fields);
        let detail = audit::change_detail("menu", id, menu_summary(&before), after);
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(user.tenant_id),
                organization_id: Some(user.org_id),
                action: "UPDATE_MENU",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    }
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(menu_json(&row)))
}

/// DELETE /api/menus/{id} — 级联删除子菜单与角色授权
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:menu")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let mut tx = state.pool.begin().await?;
    let before = lock_menu(&mut tx, id).await?;
    sqlx::query("DELETE FROM menus WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("menu", id, menu_summary(&before), Value::Null);
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "DELETE_MENU",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_parent_update_distinguishes_omission_null_and_empty() {
        let omitted: MenuUpdate = serde_json::from_value(json!({"title": "菜单"})).unwrap();
        assert_eq!(omitted.parent_id, None);
        let cleared: MenuUpdate = serde_json::from_value(json!({"parentId": null})).unwrap();
        assert_eq!(cleared.parent_id, Some(None));
        let empty: MenuUpdate = serde_json::from_value(json!({"parentId": "  "})).unwrap();
        assert_eq!(
            parse_parent_id(empty.parent_id.unwrap().as_deref()).unwrap(),
            None
        );
        let id = crate::util::uuid7::fixture_uuid(1);
        let selected: MenuUpdate =
            serde_json::from_value(json!({"parentId": format!(" {id} ")})).unwrap();
        assert_eq!(
            parse_parent_id(selected.parent_id.unwrap().as_deref()).unwrap(),
            Some(id)
        );
        assert!(parse_parent_id(Some("invalid")).is_err());
    }
}
