use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::authorization;
use crate::error::AppError;
use crate::extractors::CurrentUser;

/// OAuth 端点 IP 限流参数: 滑动窗口 60 秒内最多 30 次
const OAUTH_RATE_WINDOW_SECS: u64 = 60;
const OAUTH_RATE_MAX_REQUESTS: usize = 30;
/// 限流表超过该 IP 数量时触发一次全量过期清理 (防内存膨胀, 均摊开销)
const OAUTH_RATE_MAP_GC_THRESHOLD: usize = 4096;

type RateMap = Arc<Mutex<HashMap<String, Vec<Instant>>>>;

/// All lanes are bounded and use monotonic time. No active identity is evicted to
/// make room for an attacker; a saturated table or poisoned lock fails closed.
fn record_rate(map: &RateMap, key: &str, limit: usize) -> Option<i64> {
    let now = Instant::now();
    let period = Duration::from_secs(OAUTH_RATE_WINDOW_SECS);
    let Ok(mut map) = map.lock() else {
        return Some(60);
    };
    if map.len() >= OAUTH_RATE_MAP_GC_THRESHOLD && !map.contains_key(key) {
        map.retain(|_, entries| {
            entries
                .iter()
                .any(|instant| now.duration_since(*instant) < period)
        });
        if map.len() >= OAUTH_RATE_MAP_GC_THRESHOLD {
            return Some(60);
        }
    }
    let entries = map.entry(key.to_owned()).or_default();
    entries.retain(|instant| now.duration_since(*instant) < period);
    if entries.len() >= limit {
        let elapsed = entries
            .first()
            .map(|oldest| now.duration_since(*oldest).as_secs())
            .unwrap_or(0);
        return Some(OAUTH_RATE_WINDOW_SECS.saturating_sub(elapsed).max(1) as i64);
    }
    entries.push(now);
    None
}

/// 组织祖先链 (含自身) 的 WITH RECURSIVE 片段, 以 $1 为当前组织 (含 parent_id 列供递归)
pub const ORG_ANCESTORS_SQL: &str = "WITH RECURSIVE anc AS ( \
SELECT o.id, o.parent_id FROM organizations o WHERE o.id = $1 \
UNION ALL SELECT o.id, o.parent_id FROM organizations o JOIN anc ON o.id = anc.parent_id )";

/// 应用共享状态: 连接池 + 配置 + OAuth 端点限流窗口 + 进程启动时刻
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub config: Arc<crate::config::Config>,
    /// 进程启动时刻 (GET /api/monitor/system 的 startedAt/uptimeSeconds 数据源)
    pub started_at: chrono::DateTime<chrono::Utc>,
    oauth_rate: RateMap,
    introspection_clients: RateMap,
    introspection_failures: RateMap,
    introspection_attempts: RateMap,
    pub introspection_auth_slots: Arc<tokio::sync::Semaphore>,
}

impl AppState {
    pub fn new(pool: sqlx::PgPool, config: Arc<crate::config::Config>) -> Self {
        Self {
            pool,
            config,
            started_at: chrono::Utc::now(),
            oauth_rate: Arc::new(Mutex::new(HashMap::new())),
            introspection_clients: Arc::new(Mutex::new(HashMap::new())),
            introspection_failures: Arc::new(Mutex::new(HashMap::new())),
            introspection_attempts: Arc::new(Mutex::new(HashMap::new())),
            introspection_auth_slots: Arc::new(tokio::sync::Semaphore::new(32)),
        }
    }

    /// 从与菜单展示相同的授权结果计算权限码，逐请求读取以保证多实例下一请求一致。
    ///
    /// 权限口径与 GET /api/auth/menus 的非超管分支完全一致:
    /// 角色 ∩ 组织授权 ∩ 租户授权 —— 菜单(含 BUTTON 型操作权限码)必须同时被
    /// 当前组织与所属租户授权, 角色菜单才生效, 保证「组织只能看到/操作被授权的菜单与按钮」。
    pub async fn permissions_of(
        &self,
        user_id: Uuid,
        org_id: Uuid,
    ) -> Result<HashSet<String>, AppError> {
        let tenant_id: Uuid =
            sqlx::query_scalar("SELECT tenant_id FROM organizations WHERE id = $1")
                .bind(org_id)
                .fetch_one(&self.pool)
                .await?;
        let rows = authorization::effective_menu_rows(&self.pool, user_id, tenant_id, org_id, None)
            .await?;
        Ok(authorization::permissions_of(&rows))
    }

    /// 权限校验 (超管直通)
    pub async fn require(&self, user: &CurrentUser, perm: &'static str) -> Result<(), AppError> {
        // OAuth clients and token administration are global platform resources;
        // a tenant menu/role grant cannot make them tenant-owned resources.
        if perm == "system:oauth" { self.require_platform(user).await?; }
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
        let platform: Option<bool> =
            sqlx::query_scalar("SELECT is_platform FROM tenants WHERE id = $1")
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

    /// 管理接口的资源归属检查。跨租户资源与不存在资源统一表现为 404。
    pub async fn ensure_tenant_scope(
        &self,
        user: &CurrentUser,
        tenant_id: Uuid,
        resource: &'static str,
    ) -> Result<(), AppError> {
        if self.is_platform_context(user).await? || tenant_id == user.tenant_id {
            Ok(())
        } else {
            Err(AppError::not_found(format!("{resource}不存在")))
        }
    }

    /// 用户在指定组织上下文中的生效角色 (角色所属组织 ∈ 组织祖先链)
    pub async fn effective_roles(
        &self,
        user_id: Uuid,
        org_id: Uuid,
    ) -> Result<Vec<RoleBrief>, AppError> {
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
        Ok(rows
            .into_iter()
            .map(|(id, name, code)| RoleBrief { id, name, code })
            .collect())
    }

    /// 兼容现有写入调用；权限现在逐请求读取，不再有进程内正向缓存。
    pub fn invalidate_perm_cache(&self) {}

    /// OAuth 端点滑动窗口限流: 清理该 IP 过期时间戳后检查长度,
    /// 未超限则记录本次请求返回 None; 超限不记录, 返回建议的 Retry-After 秒数。
    /// 锁中毒或达到身份数量上限时拒绝请求。
    pub fn record_oauth_request(&self, ip: &str) -> Option<i64> {
        record_rate(&self.oauth_rate, ip, OAUTH_RATE_MAX_REQUESTS)
    }

    pub fn record_introspection_attempt(&self, ip: &str) -> Option<i64> {
        record_rate(
            &self.introspection_attempts,
            ip,
            self.config.oauth_introspection_attempts_per_minute,
        )
    }

    pub fn record_introspection_failure(&self, ip: &str) -> Option<i64> {
        record_rate(
            &self.introspection_failures,
            ip,
            self.config.oauth_auth_failure_requests_per_minute,
        )
    }

    pub fn record_introspection_client(&self, client_id: &str) -> Option<i64> {
        record_rate(
            &self.introspection_clients,
            client_id,
            self.config.oauth_introspection_client_requests_per_minute,
        )
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RoleBrief {
    pub id: Uuid,
    pub name: String,
    pub code: String,
}
