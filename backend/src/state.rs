use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::CurrentUser;

/// OAuth 端点 IP 限流参数: 滑动窗口 60 秒内最多 30 次
const OAUTH_RATE_WINDOW_SECS: i64 = 60;
const OAUTH_RATE_MAX_REQUESTS: usize = 30;
/// 限流表超过该 IP 数量时触发一次全量过期清理 (防内存膨胀, 均摊开销)
const OAUTH_RATE_MAP_GC_THRESHOLD: usize = 1024;

/// 组织祖先链 (含自身) 的 WITH RECURSIVE 片段, 以 $1 为当前组织 (含 parent_id 列供递归)
pub const ORG_ANCESTORS_SQL: &str = "WITH RECURSIVE anc AS ( \
SELECT o.id, o.parent_id FROM organizations o WHERE o.id = $1 \
UNION ALL SELECT o.id, o.parent_id FROM organizations o JOIN anc ON o.id = anc.parent_id )";

/// 应用共享状态: 连接池 + 配置 + 权限缓存 (30s TTL) + OAuth 端点限流窗口 + 进程启动时刻
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub config: Arc<crate::config::Config>,
    /// 进程启动时刻 (GET /api/monitor/system 的 startedAt/uptimeSeconds 数据源)
    pub started_at: chrono::DateTime<chrono::Utc>,
    perm_cache: Arc<Mutex<HashMap<(Uuid, Uuid), (HashSet<String>, i64)>>>,
    oauth_rate: Arc<Mutex<HashMap<String, Vec<i64>>>>,
}

impl AppState {
    pub fn new(pool: sqlx::PgPool, config: Arc<crate::config::Config>) -> Self {
        Self {
            pool,
            config,
            started_at: chrono::Utc::now(),
            perm_cache: Arc::new(Mutex::new(HashMap::new())),
            oauth_rate: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 计算用户在指定组织上下文中的权限码集合 (30 秒内存缓存)
    ///
    /// 权限口径与 GET /api/auth/menus 的非超管分支完全一致:
    /// 角色 ∩ 组织授权 ∩ 租户授权 —— 菜单(含 BUTTON 型操作权限码)必须同时被
    /// 当前组织与所属租户授权, 角色菜单才生效, 保证「组织只能看到/操作被授权的菜单与按钮」。
    pub async fn permissions_of(&self, user_id: Uuid, org_id: Uuid) -> Result<HashSet<String>, AppError> {
        let now = chrono::Utc::now().timestamp();
        {
            let cache = self.perm_cache.lock().unwrap();
            if let Some((perms, exp)) = cache.get(&(user_id, org_id)) {
                if *exp > now {
                    return Ok(perms.clone());
                }
            }
        }
        let sql = format!(
            "{} SELECT DISTINCT m.permission FROM user_roles ur \
             JOIN roles r ON r.id = ur.role_id \
             JOIN anc ON anc.id = r.organization_id \
             JOIN role_menus rm ON rm.role_id = r.id \
             JOIN menus m ON m.id = rm.menu_id \
             WHERE ur.user_id = $2 AND r.status = 'ENABLED' AND m.status = 'ENABLED' AND m.permission <> '' \
             AND m.id IN (SELECT menu_id FROM organization_menus WHERE organization_id = $1) \
             AND m.id IN (SELECT menu_id FROM tenant_menus \
                          WHERE tenant_id = (SELECT tenant_id FROM organizations WHERE id = $1))",
            ORG_ANCESTORS_SQL
        );
        let rows: Vec<(String,)> = sqlx::query_as(&sql)
            .bind(org_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        let perms: HashSet<String> = rows.into_iter().map(|(p,)| p).collect();
        let mut cache = self.perm_cache.lock().unwrap();
        cache.insert((user_id, org_id), (perms.clone(), now + 30));
        Ok(perms)
    }

    /// 权限校验 (超管直通)
    pub async fn require(&self, user: &CurrentUser, perm: &'static str) -> Result<(), AppError> {
        if user.is_super_admin {
            return Ok(());
        }
        let perms = self.permissions_of(user.user_id, user.org_id).await?;
        if perms.contains(perm) {
            Ok(())
        } else {
            Err(AppError::forbidden(format!("无操作权限: {}", perm)))
        }
    }

    /// 当前会话是否处于平台租户上下文 (超管视为平台级):
    /// 平台租户成员可跨租户管理, 其余租户只能操作本租户数据。
    pub async fn is_platform_context(&self, user: &CurrentUser) -> Result<bool, AppError> {
        if user.is_super_admin {
            return Ok(true);
        }
        let platform: Option<bool> = sqlx::query_scalar("SELECT is_platform FROM tenants WHERE id = $1")
            .bind(user.tenant_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(platform.unwrap_or(false))
    }

    /// 跨租户写操作防线: 仅平台租户(或超管)可继续, 其余返回 403
    pub async fn require_platform(&self, user: &CurrentUser) -> Result<(), AppError> {
        if self.is_platform_context(user).await? {
            Ok(())
        } else {
            Err(AppError::forbidden("仅平台租户可执行跨租户操作"))
        }
    }

    /// 用户在指定组织上下文中的生效角色 (角色所属组织 ∈ 组织祖先链)
    pub async fn effective_roles(&self, user_id: Uuid, org_id: Uuid) -> Result<Vec<RoleBrief>, AppError> {
        let sql = format!(
            "{} SELECT DISTINCT r.id, r.name, COALESCE(r.code, '') FROM user_roles ur \
             JOIN roles r ON r.id = ur.role_id \
             JOIN anc ON anc.id = r.organization_id \
             WHERE ur.user_id = $2 AND r.status = 'ENABLED' \
             ORDER BY r.name",
            ORG_ANCESTORS_SQL
        );
        let rows: Vec<(Uuid, String, String)> = sqlx::query_as(&sql)
            .bind(org_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|(id, name, code)| RoleBrief { id, name, code }).collect())
    }

    /// 清空权限缓存 (授权/角色变更后调用)
    pub fn invalidate_perm_cache(&self) {
        self.perm_cache.lock().unwrap().clear();
    }

    /// OAuth 端点滑动窗口限流: 清理该 IP 过期时间戳后检查长度,
    /// 未超限则记录本次请求返回 None; 超限不记录, 返回建议的 Retry-After 秒数。
    /// 锁粒度仅为一次内存操作, 持有时间极短; 锁中毒时放行 (fail-open)。
    pub fn record_oauth_request(&self, ip: &str) -> Option<i64> {
        let now = chrono::Utc::now().timestamp();
        let Ok(mut map) = self.oauth_rate.lock() else { return None };
        let window = map.entry(ip.to_string()).or_default();
        window.retain(|t| now - *t < OAUTH_RATE_WINDOW_SECS);
        if window.len() >= OAUTH_RATE_MAX_REQUESTS {
            let oldest = window.first().copied().unwrap_or(now);
            return Some((oldest + OAUTH_RATE_WINDOW_SECS - now).max(1));
        }
        window.push(now);
        if map.len() > OAUTH_RATE_MAP_GC_THRESHOLD {
            map.retain(|_, v| v.iter().any(|t| now - *t < OAUTH_RATE_WINDOW_SECS));
        }
        None
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RoleBrief {
    pub id: Uuid,
    pub name: String,
    pub code: String,
}
