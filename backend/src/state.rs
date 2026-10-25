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
    pub passwords: crate::util::password::PasswordService,
    pub(crate) authentication: Arc<dyn crate::platform_accounts::AuthenticationPort>,
    pub(crate) accounts: Arc<dyn crate::platform_accounts::AccountAdministrationPort>,
    pub(crate) remote_accounts: Option<Arc<crate::authentication::remote::RemoteAccounts>>,
    pub(crate) identity_admin_slots: Arc<tokio::sync::Semaphore>,
    pub login_limits: Arc<crate::login_limits::LoginLimits>,
    pub map_client: reqwest::Client,
    /// 进程启动时刻 (GET /api/monitor/system 的 startedAt/uptimeSeconds 数据源)
    pub started_at: chrono::DateTime<chrono::Utc>,
    oauth_rate: RateMap,
    map_rate: RateMap,
    introspection_clients: RateMap,
    introspection_failures: RateMap,
    introspection_attempts: RateMap,
    pub introspection_auth_slots: Arc<tokio::sync::Semaphore>,
    pub app_authorization_slots: Arc<tokio::sync::Semaphore>,
    pub(crate) bff_transport: Arc<dyn crate::bff::OidcTransport>,
    pub(crate) bff_authority_slots: Arc<tokio::sync::Semaphore>,
    pub(crate) customer_portal_authority:
        Arc<dyn crate::customer_portal::authority::AccountProofPort>,
    pub(crate) customer_portal_slots: Arc<tokio::sync::Semaphore>,
}

impl AppState {
    #[cfg(test)]
    pub fn new(pool: sqlx::PgPool, config: Arc<crate::config::Config>) -> Self {
        Self::try_new(pool, config).expect("test runtime configuration")
    }

    pub fn try_new(pool: sqlx::PgPool, config: Arc<crate::config::Config>) -> anyhow::Result<Self> {
        let app_authorization_slots = Arc::new(tokio::sync::Semaphore::new(
            config.app_authorization_max_concurrent,
        ));
        let passwords = crate::util::password::PasswordService::new(
            config.password_concurrency,
            config.password_queue_capacity,
            Duration::from_millis(config.password_queue_timeout_ms),
            Duration::from_millis(config.password_execution_timeout_ms),
        );
        let (authentication, accounts, remote_accounts) =
            crate::authentication::new(pool.clone(), passwords.clone(), config.clone());
        let bff_transport = crate::bff::configured_transport(&config);
        let bff_authority_slots = Arc::new(tokio::sync::Semaphore::new(config.bff_max_concurrent));
        let customer_portal_authority =
            if config.identity_remote_enabled && config.customer_portal_port_enabled {
                crate::customer_portal::remote_authority::configured(&config)?
            } else {
                crate::oidc::customer_portal_authority(pool.clone(), config.clone())
            };
        Ok(Self {
            customer_portal_authority,
            customer_portal_slots: Arc::new(tokio::sync::Semaphore::new(16)),
            bff_transport,
            bff_authority_slots,
            app_authorization_slots,
            pool,
            passwords,
            authentication,
            accounts,
            remote_accounts,
            identity_admin_slots: Arc::new(tokio::sync::Semaphore::new(16)),
            login_limits: Arc::new(crate::login_limits::LoginLimits::new(
                crate::login_limits::LoginLimitOptions {
                    max_entries: config.login_max_entries,
                    ttl: Duration::from_secs(config.login_window_seconds),
                    source_limit: config.login_source_requests,
                    user_failure_limit: config.login_user_failures,
                },
            )),
            config,
            map_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("SaaSPlatform/1.0 map-resource-proxy")
                .build()
                .expect("static map client configuration"),
            started_at: chrono::Utc::now(),
            oauth_rate: Arc::new(Mutex::new(HashMap::new())),
            map_rate: Arc::new(Mutex::new(HashMap::new())),
            introspection_clients: Arc::new(Mutex::new(HashMap::new())),
            introspection_failures: Arc::new(Mutex::new(HashMap::new())),
            introspection_attempts: Arc::new(Mutex::new(HashMap::new())),
            introspection_auth_slots: Arc::new(tokio::sync::Semaphore::new(32)),
        })
    }

    pub fn check_map_rate(&self, identity: &str, limit: usize) -> Result<(), AppError> {
        match record_rate(&self.map_rate, identity, limit) {
            Some(retry) => Err(AppError::RateLimited(retry as u64)),
            None => Ok(()),
        }
    }

    /// 从与菜单展示相同的授权结果计算权限码，逐请求读取以保证多实例下一请求一致。
    ///
    /// 权限口径与 GET /api/auth/menus 的非超管分支完全一致:
    /// 角色 ∩ 组织授权 ∩ 租户授权 —— 菜单(含 BUTTON 型操作权限码)必须同时被
    /// 当前组织与所属租户授权, 角色菜单才生效, 保证「组织只能看到/操作被授权的菜单与按钮」。
    #[cfg(test)]
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

    /// 权限校验；任何上下文都必须满足当前组织与租户的应用、菜单授权。
    pub async fn require(&self, user: &CurrentUser, perm: &'static str) -> Result<(), AppError> {
        // These catalogs are global platform resources; a tenant or child
        // organization menu grant cannot turn them into scoped resources.
        if matches!(
            perm,
            "system:oauth"
                | "system:settings"
                | "system:application"
                | "system:menu"
                | "system:deployment:read"
                | "system:deployment:write"
                | "system:distribution:read"
                | "system:distribution:write"
        ) {
            self.require_platform(user).await?;
        }
        let perms = self.context_permissions(user).await?;
        if perms.contains(perm) {
            Ok(())
        } else {
            Err(AppError::forbidden(format!("无操作权限: {}", perm)))
        }
    }

    /// Menus sharing a permission code still require an explicit grant for the
    /// selected page. Application and tenant/organization caps are unchanged.
    pub(crate) async fn require_menu_path(
        &self,
        user: &CurrentUser,
        path: &'static str,
    ) -> Result<(), AppError> {
        let rows = authorization::context_menu_rows(&self.pool, user, None).await?;
        if rows
            .iter()
            .any(|menu| menu["type"] == "MENU" && menu["path"] == path)
        {
            Ok(())
        } else {
            Err(AppError::forbidden(format!("当前组织未授权菜单: {path}")))
        }
    }

    /// 跨租户管理只在实际平台租户的根组织上下文生效。
    /// 切换到平台子组织时，也必须遵守当前组织子树的数据边界。
    pub async fn is_platform_context(&self, user: &CurrentUser) -> Result<bool, AppError> {
        authorization::require_employee_context(&self.pool, user).await?;
        let platform: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM tenants t JOIN organizations o ON o.tenant_id=t.id \
             WHERE t.id=$1 AND o.id=$2 AND t.is_platform AND o.parent_id IS NULL)",
        )
        .bind(user.tenant_id)
        .bind(user.org_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(platform)
    }

    pub(crate) async fn can_manage_platform(&self, user: &CurrentUser) -> Result<bool, AppError> {
        self.is_platform_context(user).await
    }

    /// 当前上下文的组织数据范围。仅平台根组织返回无组织限制的 None。
    /// UNION 在异常循环关系下也能终止，递归每一步都限制在当前租户。
    pub(crate) async fn scoped_org_ids(
        &self,
        user: &CurrentUser,
    ) -> Result<Option<Vec<Uuid>>, AppError> {
        if self.can_manage_platform(user).await? {
            return Ok(None);
        }
        let ids = sqlx::query_scalar(
            "WITH RECURSIVE scoped_orgs AS ( \
             SELECT id FROM organizations WHERE id=$1 AND tenant_id=$2 \
             UNION SELECT o.id FROM organizations o JOIN scoped_orgs parent ON o.parent_id=parent.id \
             WHERE o.tenant_id=$2 ) SELECT id FROM scoped_orgs",
        )
        .bind(user.org_id)
        .bind(user.tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(ids))
    }

    /// 目标组织必须属于当前组织及下级组织；越界与不存在统一返回 404。
    pub(crate) async fn ensure_org_scope(
        &self,
        user: &CurrentUser,
        org_id: Uuid,
        resource: &'static str,
    ) -> Result<(), AppError> {
        let scope = self.scoped_org_ids(user).await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM organizations WHERE id=$1 \
             AND ($2::uuid[] IS NULL OR id=ANY($2)))",
        )
        .bind(org_id)
        .bind(scope.as_deref())
        .fetch_one(&self.pool)
        .await?;
        if exists {
            Ok(())
        } else {
            Err(AppError::not_found(format!("{resource}不存在")))
        }
    }

    /// 跨租户写操作防线: 仅当前平台租户上下文可继续, 其余返回 403
    pub async fn require_platform(&self, user: &CurrentUser) -> Result<(), AppError> {
        if self.is_platform_context(user).await? {
            Ok(())
        } else {
            Err(AppError::forbidden("仅平台租户根组织可执行平台管理操作"))
        }
    }

    /// The selected context bounds global administrator permissions as well as menus.
    pub async fn context_permissions(
        &self,
        user: &CurrentUser,
    ) -> Result<HashSet<String>, AppError> {
        let rows = authorization::context_menu_rows(&self.pool, user, None).await?;
        Ok(authorization::permissions_of(&rows))
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
             AND EXISTS(SELECT 1 FROM memberships employee JOIN users subject ON subject.id=employee.user_id \
                 JOIN organizations selected ON selected.id=employee.organization_id \
                 JOIN tenants tenant ON tenant.id=selected.tenant_id \
                 WHERE employee.user_id=$2 AND employee.organization_id=$1 \
                 AND employee.member_type='EMPLOYEE' AND employee.status='ENABLED' \
                 AND subject.status='ENABLED' AND tenant.status='ENABLED' AND r.tenant_id=tenant.id \
                 AND (tenant.expires_at IS NULL OR tenant.expires_at>clock_timestamp()) \
                 AND org_chain_active(selected.id)) \
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

    /// Current-session role projection must prove its own employee identity.
    pub(crate) async fn context_roles(
        &self,
        user: &CurrentUser,
    ) -> Result<Vec<RoleBrief>, AppError> {
        let before = authorization::require_employee_context(&self.pool, user).await?;
        let roles = self.effective_roles(user.user_id, user.org_id).await?;
        if authorization::require_employee_context(&self.pool, user).await? != before {
            return Err(AppError::unauthorized("员工身份已在角色计算期间变更"));
        }
        Ok(roles)
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
