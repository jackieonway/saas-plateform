//! One PostgreSQL statement assembles every authorization projection in one
//! MVCC snapshot. A separate invocation is required for the final fresh check.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{app_token::AppTokenInfo, authorization, error::AppError, state::ORG_ANCESTORS_SQL};

#[derive(Serialize, Deserialize)]
pub(super) struct ContextRows {
    pub identity: (String, String, String, String, String, String, String, bool),
    pub menu_rows: Vec<Value>,
    pub roles: Vec<(Uuid, String, String)>,
    pub grants: Vec<(String, String, String)>,
    pub subtree: Vec<Uuid>,
    pub can_manage_platform: bool,
    pub runtime: (String, String, String, Option<String>, Option<Uuid>),
}

/// All SQL and database row shapes remain in the executable's private adapter.
pub(super) async fn load(
    pool: &PgPool,
    binding: &AppTokenInfo,
    environment: &str,
) -> Result<ContextRows, AppError> {
    let timer = crate::observability::stages().timer(operational_telemetry::Stage::BusinessSql);
    let result = load_inner(pool, binding, environment).await;
    timer.finish(crate::observability::outcome(&result));
    result
}

async fn load_inner(
    pool: &PgPool,
    binding: &AppTokenInfo,
    environment: &str,
) -> Result<ContextRows, AppError> {
    // The menu query factory is shared with the existing platform menu facade.
    // Its positional contract is organization/user/tenant/application.
    let effective = authorization::effective_menu_sql();
    let granted = authorization::granted_menu_sql()
        .replace("$1", "$TENANT")
        .replace("$2", "$ORG")
        .replace("$3", "$APP")
        .replace("$TENANT", "$3")
        .replace("$ORG", "$1")
        .replace("$APP", "$4");
    let sql = format!(
        r#"
WITH identity_row AS (
    SELECT u.username,COALESCE(u.real_name,'') AS real_name,t.code,t.name AS tenant_name,
      o.name AS org_name,COALESCE(org_path(o.id),'') AS org_path,a.name AS app_name,u.is_super_admin
    FROM users u JOIN tenants t ON t.id=$3
      JOIN organizations o ON o.id=$1 AND o.tenant_id=t.id JOIN applications a ON a.id=$4 WHERE u.id=$2
), menu_rows(id,parent_id,type,title,path,icon,permission,link_url,link_target,sort_order,visible,created_at) AS (
    SELECT * FROM ({effective}) ordinary WHERE NOT (SELECT is_super_admin FROM identity_row)
    UNION ALL
    SELECT * FROM ({granted}) administrator WHERE (SELECT is_super_admin FROM identity_row)
), role_rows(id,name,code) AS (
    {ORG_ANCESTORS_SQL} SELECT DISTINCT r.id,r.name,COALESCE(r.code,'')
    FROM user_roles ur JOIN roles r ON r.id=ur.role_id AND r.status='ENABLED'
      JOIN anc ON anc.id=r.organization_id JOIN role_menus rm ON rm.role_id=r.id
      JOIN menus m ON m.id=rm.menu_id AND m.application_id=$4 AND m.status='ENABLED'
      JOIN tenant_menus tm ON tm.menu_id=m.id AND tm.tenant_id=$3
      JOIN organization_menus om ON om.menu_id=m.id AND om.organization_id=$1
    WHERE ur.user_id=$2 ORDER BY r.id
), grant_rows(resource,action,scope) AS (
    SELECT g.resource_code,g.action_code,g.scope_type FROM membership_data_grants g
      JOIN memberships member ON member.id=g.membership_id
    WHERE member.user_id=$2 AND member.organization_id=$1 AND g.application_id=$4
    ORDER BY g.resource_code,g.action_code
), subtree AS (
    WITH RECURSIVE descendants AS (
      SELECT id,tenant_id FROM organizations WHERE id=$1 AND tenant_id=$3
        AND status='ENABLED' AND (expires_at IS NULL OR expires_at>now())
      UNION SELECT child.id,child.tenant_id FROM organizations child JOIN descendants parent ON child.parent_id=parent.id
      WHERE child.tenant_id=$3 AND child.status='ENABLED' AND (child.expires_at IS NULL OR child.expires_at>now())
    ) SELECT id FROM descendants ORDER BY id
), runtime AS (
    SELECT runtime_mode,entry_type,entry_url,version,id FROM application_effective_entry($4,$5)
)
SELECT jsonb_build_object(
 'identity',(SELECT jsonb_build_array(username,real_name,code,tenant_name,org_name,org_path,app_name,is_super_admin) FROM identity_row),
 'menu_rows',COALESCE((SELECT jsonb_agg(jsonb_build_object('id',id,'parentId',parent_id,'type',type,'title',title,
   'path',path,'icon',icon,'permission',permission,'linkUrl',link_url,'linkTarget',link_target,'sortOrder',sort_order,'visible',visible)
   ORDER BY sort_order,created_at) FROM menu_rows),'[]'::jsonb),
 'roles',COALESCE((SELECT jsonb_agg(jsonb_build_array(id,name,code) ORDER BY id) FROM role_rows),'[]'::jsonb),
 'grants',COALESCE((SELECT jsonb_agg(jsonb_build_array(resource,action,scope) ORDER BY resource,action) FROM grant_rows),'[]'::jsonb),
 'subtree',COALESCE((SELECT jsonb_agg(id ORDER BY id) FROM subtree),'[]'::jsonb),
 'can_manage_platform',EXISTS(SELECT 1 FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE o.id=$1 AND t.id=$3 AND t.is_platform AND o.parent_id IS NULL),
 'runtime',(SELECT jsonb_build_array(runtime_mode,entry_type,entry_url,version,id) FROM runtime))
"#
    );
    let mut connection = crate::observability::acquire(pool).await?;
    let value: Value = sqlx::query_scalar(&sql)
        .bind(binding.organization_id)
        .bind(binding.user_id)
        .bind(binding.tenant_id)
        .bind(binding.application_id)
        .bind(environment)
        .fetch_one(&mut *connection)
        .await?;
    drop(connection);
    serde_json::from_value(value)
        .map_err(|_| AppError::Server(anyhow::anyhow!("AUTHORIZATION_SNAPSHOT_INVALID")))
}
