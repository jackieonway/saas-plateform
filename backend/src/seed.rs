use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::util::{hash, password};

const DEMO_USERS: &[(&str, &str)] = &[
    ("admin", "admin123"),
    ("zhangsan", "Zs@12345"),
    ("lisi", "Ls@12345"),
    ("wangwu", "Ww@12345"),
    ("tianqi", "Tq@12345"),
    ("chenjiu", "Cj@12345"),
];
const DEMO_CLIENT_SECRETS: &[&str] = &[
    "platform-web-secret",
    "api-service-secret",
    "resource-portal-secret",
    "micro-vue-shell-secret",
    "micro-jquery-shell-secret",
    "micro-report-shell-secret",
];

pub fn is_demo_password(candidate: &str) -> bool {
    DEMO_USERS
        .iter()
        .any(|(_, password)| *password == candidate)
}

/// A read-only startup gate for upgraded installations. Operators rotate or disable
/// known credentials explicitly; startup never changes existing users or grants.
pub async fn check_production_credentials(pool: &PgPool) -> anyhow::Result<()> {
    let mut after: Option<Uuid> = None;
    loop {
        let users: Vec<(Uuid, String)> = sqlx::query_as("SELECT id,password_hash FROM users WHERE status='ENABLED' AND ($1::uuid IS NULL OR id>$1) ORDER BY id LIMIT 500")
            .bind(after).fetch_all(pool).await?;
        if users.is_empty() {
            break;
        }
        after = users.last().map(|row| row.0);
        for (_, encoded) in users {
            if DEMO_USERS
                .iter()
                .any(|(_, candidate)| password::verify_password(&encoded, candidate))
            {
                anyhow::bail!("PRODUCTION_DEMO_USER_CREDENTIAL: rotate or disable enabled documented demonstration accounts before startup");
            }
        }
    }
    let hashes: Vec<String> = DEMO_CLIENT_SECRETS
        .iter()
        .map(|secret| hash::sha256_hex(secret))
        .collect();
    let unsafe_clients: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM oauth_clients WHERE status='ENABLED' AND client_secret_hash=ANY($1))")
        .bind(&hashes).fetch_one(pool).await?;
    anyhow::ensure!(!unsafe_clients, "PRODUCTION_DEMO_CLIENT_CREDENTIAL: rotate or disable enabled documented demonstration clients before startup");
    Ok(())
}

pub async fn initialize(pool: &PgPool, config: &crate::config::Config) -> anyhow::Result<()> {
    if !config.is_production() {
        return run(pool).await;
    }
    let mut tx = pool.begin().await?;
    // Serialize first bootstrap across platform processes without deleting history.
    sqlx::query("SELECT pg_advisory_xact_lock(764351204988213)")
        .execute(&mut *tx)
        .await?;
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if existing > 0 {
        tx.rollback().await?;
        return check_production_credentials(pool).await;
    }
    let username = config.bootstrap_admin_username.as_deref().ok_or_else(|| {
        anyhow::anyhow!("BOOTSTRAP_ADMIN_USERNAME is required for an empty production database")
    })?;
    let admin_password = config.bootstrap_admin_password.as_deref().ok_or_else(|| {
        anyhow::anyhow!("BOOTSTRAP_ADMIN_PASSWORD is required for an empty production database")
    })?;
    anyhow::ensure!(
        admin_password.chars().count() >= 12 && !is_demo_password(admin_password),
        "production bootstrap password does not satisfy requirements"
    );
    password::validate_for_config(admin_password, config)?;
    // Historical migrations contain examples. Only on the first empty-user
    // bootstrap, quarantine those exact artifacts; never grant them to production.
    sqlx::query("UPDATE applications SET status='DISABLED' WHERE code IN ('vue-workshop','legacy-jquery','report-center')")
        .execute(&mut *tx).await?;
    let hashes: Vec<String> = DEMO_CLIENT_SECRETS
        .iter()
        .map(|secret| hash::sha256_hex(secret))
        .collect();
    sqlx::query("UPDATE oauth_clients SET status='DISABLED' WHERE client_secret_hash=ANY($1)")
        .bind(&hashes)
        .execute(&mut *tx)
        .await?;
    let tenant = scalar_uuid(&mut tx, "INSERT INTO tenants(name,code,description,is_platform) VALUES('平台管理租户','admin-tenant','平台控制层',true) RETURNING id").await?;
    let root = insert_org(&mut tx, tenant, None, "平台总部", "HQ", 1, "平台管理组织").await?;
    let system = insert_app(&mut tx, "system", "系统管理", "LayoutGrid", "/system", 1).await?;
    let hermes = insert_app(&mut tx, "hermes", "消息中心", "MessageSquare", "/hermes", 2).await?;
    let administrator = insert_user(
        &mut tx,
        username,
        admin_password,
        "平台管理员",
        "",
        "",
        "UNKNOWN",
        true,
    )
    .await?;
    insert_membership(&mut tx, administrator, root, None, "平台管理员").await?;
    let role = insert_role(
        &mut tx,
        tenant,
        root,
        "平台管理员",
        "super_admin",
        "平台内置管理角色",
        true,
    )
    .await?;
    grant_user_role(&mut tx, administrator, role, root).await?;
    for app in [system, hermes] {
        grant_tenant_app(&mut tx, tenant, app).await?;
        grant_org_app(&mut tx, root, app).await?;
    }
    let core = [
        (system, "仪表盘", "/dashboard", "Gauge", "dashboard:view"),
        (
            system,
            "租户管理",
            "/system/tenants",
            "Building",
            "system:tenant",
        ),
        (
            system,
            "组织管理",
            "/system/organizations",
            "Network",
            "system:organization",
        ),
        (
            system,
            "部门管理",
            "/system/departments",
            "FolderTree",
            "system:department",
        ),
        (
            system,
            "授权管理",
            "/system/authorizations",
            "ShieldCheck",
            "system:authorization",
        ),
        (system, "用户管理", "/system/users", "User", "system:user"),
        (system, "角色管理", "/system/roles", "Shield", "system:role"),
        (
            system,
            "应用管理",
            "/system/applications",
            "Grid3x3",
            "system:application",
        ),
        (
            system,
            "菜单管理",
            "/system/menus",
            "ListTree",
            "system:menu",
        ),
        (
            system,
            "平台设置",
            "/system/settings",
            "Settings",
            "system:settings",
        ),
        (
            system,
            "OAuth 客户端",
            "/system/oauth/clients",
            "Key",
            "system:oauth",
        ),
        (
            system,
            "设备与会话",
            "/system/oauth/tokens",
            "MonitorSmartphone",
            "system:oauth",
        ),
        (
            system,
            "审计日志",
            "/system/audit-logs",
            "ScrollText",
            "system:audit",
        ),
        (
            system,
            "平台监控",
            "/system/monitor",
            "Activity",
            "system:monitor",
        ),
        (hermes, "收件箱", "/hermes/inbox", "Inbox", "hermes:inbox"),
        (
            hermes,
            "通知设置",
            "/hermes/notifications",
            "Bell",
            "hermes:notification",
        ),
    ];
    for (sort, (app, title, path, icon, permission)) in core.into_iter().enumerate() {
        let menu = insert_menu(
            &mut tx,
            app,
            None,
            "MENU",
            title,
            path,
            icon,
            "",
            permission,
            sort as i32 + 1,
            true,
        )
        .await?;
        grant_role_menu(&mut tx, role, menu).await?;
        grant_tenant_menu(&mut tx, tenant, menu).await?;
        grant_org_menu(&mut tx, root, menu).await?;
        if permission == "system:tenant" {
            let button = insert_menu(
                &mut tx,
                system,
                Some(menu),
                "BUTTON",
                "授权",
                "",
                "",
                "",
                "system:tenant:auth",
                1,
                false,
            )
            .await?;
            grant_role_menu(&mut tx, role, button).await?;
            grant_tenant_menu(&mut tx, tenant, button).await?;
            grant_org_menu(&mut tx, root, button).await?;
        }
    }
    tx.commit().await?;
    tracing::info!(
        event = "PRODUCTION_BOOTSTRAP_COMPLETE",
        "Production core initialized; no demonstration users or active demonstration OAuth clients"
    );
    Ok(())
}

/// 幂等种子: 仅当 users 表为空时执行 (docs/architecture.md#data 种子计划)
pub async fn run(pool: &PgPool) -> anyhow::Result<()> {
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await?;
    if existing > 0 {
        tracing::info!("种子数据已存在 (users={existing}), 跳过初始化");
        return Ok(());
    }
    tracing::info!("数据库为空, 开始写入种子数据 ...");
    let mut tx = pool.begin().await?;

    // ===== 租户 =====
    // 平台租户: 全库唯一 (初始化 SQL 部分唯一索引约束), 可跨租户管理
    let tenant = scalar_uuid(
        &mut tx,
        "INSERT INTO tenants (name, code, description, is_platform) VALUES ('平台管理租户', 'admin-tenant', '多租户多层级组织多应用权限演示平台', true) RETURNING id",
    )
    .await?;

    // ===== 应用 =====
    let sys = insert_app(&mut tx, "system", "系统管理", "LayoutGrid", "/system", 1).await?;
    let athena = insert_app(&mut tx, "athena", "数据分析", "BarChart3", "/athena", 2).await?;
    let hermes = insert_app(&mut tx, "hermes", "消息中心", "MessageSquare", "/hermes", 3).await?;

    // ===== system 菜单树 =====
    let dashboard = insert_menu(&mut tx, sys, None, "MENU", "仪表盘", "/dashboard", "Gauge", "", "dashboard:view", 1, true).await?;
    let org_dir = insert_menu(&mut tx, sys, None, "DIR", "组织机构", "", "Building2", "", "", 2, true).await?;
    let tenant_mgmt = insert_menu(&mut tx, sys, Some(org_dir), "MENU", "租户管理", "/system/tenants", "Building", "", "system:tenant", 1, true).await?;
    let tenant_auth_btn = insert_menu(&mut tx, sys, Some(tenant_mgmt), "BUTTON", "授权", "", "", "", "system:tenant:auth", 1, false).await?;
    let org_mgmt = insert_menu(&mut tx, sys, Some(org_dir), "MENU", "组织管理", "/system/organizations", "Network", "", "system:organization", 2, true).await?;
    let dept_mgmt = insert_menu(&mut tx, sys, Some(org_dir), "MENU", "部门管理", "/system/departments", "FolderTree", "", "system:department", 3, true).await?;
    let authz_mgmt = insert_menu(&mut tx, sys, Some(org_dir), "MENU", "授权管理", "/system/authorizations", "ShieldCheck", "", "system:authorization", 4, true).await?;
    let ur_dir = insert_menu(&mut tx, sys, None, "DIR", "用户与角色", "", "Users", "", "", 3, true).await?;
    let user_mgmt = insert_menu(&mut tx, sys, Some(ur_dir), "MENU", "用户管理", "/system/users", "User", "", "system:user", 1, true).await?;
    let role_mgmt = insert_menu(&mut tx, sys, Some(ur_dir), "MENU", "角色管理", "/system/roles", "Shield", "", "system:role", 2, true).await?;
    let cfg_dir = insert_menu(&mut tx, sys, None, "DIR", "平台配置", "", "Settings", "", "", 4, true).await?;
    let app_mgmt = insert_menu(&mut tx, sys, Some(cfg_dir), "MENU", "应用管理", "/system/applications", "Grid3x3", "", "system:application", 1, true).await?;
    let menu_mgmt = insert_menu(&mut tx, sys, Some(cfg_dir), "MENU", "菜单管理", "/system/menus", "ListTree", "", "system:menu", 2, true).await?;
    let platform_settings = insert_menu(&mut tx, sys, Some(cfg_dir), "MENU", "平台设置", "/system/settings", "Settings", "", "system:settings", 3, true).await?;
    let open_dir = insert_menu(&mut tx, sys, None, "DIR", "开放平台", "", "KeyRound", "", "", 5, true).await?;
    let oauth_clients_menu = insert_menu(&mut tx, sys, Some(open_dir), "MENU", "OAuth 客户端", "/system/oauth/clients", "Key", "", "system:oauth", 1, true).await?;
    let device_session_menu = insert_menu(&mut tx, sys, Some(open_dir), "MENU", "设备与会话", "/system/oauth/tokens", "MonitorSmartphone", "", "system:oauth", 2, true).await?;
    let audit_menu = insert_menu(&mut tx, sys, Some(open_dir), "MENU", "审计日志", "/system/audit-logs", "ScrollText", "", "system:audit", 3, true).await?;
    let monitor_menu = insert_menu(&mut tx, sys, Some(open_dir), "MENU", "平台监控", "/system/monitor", "Activity", "", "system:monitor", 4, true).await?;

    // ===== athena 菜单树 =====
    let a_overview = insert_menu(&mut tx, athena, None, "MENU", "数据概览", "/athena/overview", "PieChart", "", "athena:overview", 1, true).await?;
    let a_reports_dir = insert_menu(&mut tx, athena, None, "DIR", "报表中心", "", "FileBarChart", "", "", 2, true).await?;
    let a_sales = insert_menu(&mut tx, athena, Some(a_reports_dir), "MENU", "销售报表", "/athena/reports/sales", "TrendingUp", "", "athena:report:sales", 1, true).await?;
    let a_ops = insert_menu(&mut tx, athena, Some(a_reports_dir), "MENU", "运营报表", "/athena/reports/operations", "LineChart", "", "athena:report:operations", 2, true).await?;
    let a_sources = insert_menu(&mut tx, athena, None, "MENU", "数据源管理", "/athena/sources", "Database", "", "athena:source", 3, true).await?;

    // ===== hermes 菜单树 =====
    let h_inbox = insert_menu(&mut tx, hermes, None, "MENU", "收件箱", "/hermes/inbox", "Inbox", "", "hermes:inbox", 1, true).await?;
    let h_notify = insert_menu(&mut tx, hermes, None, "MENU", "通知设置", "/hermes/notifications", "Bell", "", "hermes:notification", 2, true).await?;

    // ===== 组织树 =====
    let root = insert_org(&mut tx, tenant, None, "总公司", "HQ", 1, "平台总部组织").await?;
    let east = insert_org(&mut tx, tenant, Some(root), "华东区分公司", "EAST", 1, "").await?;
    let shanghai = insert_org(&mut tx, tenant, Some(east), "上海办事处", "SH", 1, "").await?;
    let hangzhou = insert_org(&mut tx, tenant, Some(east), "杭州办事处", "HZ", 2, "").await?;
    let north = insert_org(&mut tx, tenant, Some(root), "华北区分公司", "NORTH", 2, "").await?;
    let beijing = insert_org(&mut tx, tenant, Some(north), "北京办事处", "BJ", 1, "").await?;

    // ===== 部门 =====
    let _product_dept = insert_dept(&mut tx, root, None, "产品部", "PRODUCT", 1).await?;
    let rd_dept = insert_dept(&mut tx, root, None, "研发部", "RD", 2).await?;
    let _fe_group = insert_dept(&mut tx, root, Some(rd_dept), "前端组", "FE", 1).await?;
    let _be_group = insert_dept(&mut tx, root, Some(rd_dept), "后端组", "BE", 2).await?;
    let _hr_dept = insert_dept(&mut tx, root, None, "人事部", "HR", 3).await?;
    let _delivery1 = insert_dept(&mut tx, east, None, "交付一组", "DLV1", 1).await?;

    // ===== 用户 (密码 Argon2 现算) =====
    let admin = insert_user(&mut tx, "admin", "admin123", "超级管理员", "admin@example.test", "", "UNKNOWN", true).await?;
    let zhangsan = insert_user(&mut tx, "zhangsan", "Zs@12345", "张三", "zhangsan@example.test", "13800000001", "MALE", false).await?;
    let lisi = insert_user(&mut tx, "lisi", "Ls@12345", "李四", "lisi@example.test", "13800000002", "FEMALE", false).await?;
    let wangwu = insert_user(&mut tx, "wangwu", "Ww@12345", "王五", "wangwu@example.test", "13800000003", "MALE", false).await?;
    let tianqi = insert_user(&mut tx, "tianqi", "Tq@12345", "田七", "tianqi@example.test", "13800000004", "MALE", false).await?;

    // ===== 成员关系 =====
    insert_membership(&mut tx, admin, root, Some(rd_dept), "超级管理员").await?;
    insert_membership(&mut tx, zhangsan, east, None, "超级管理员").await?;
    insert_membership(&mut tx, lisi, root, None, "产品专员").await?;
    insert_membership(&mut tx, wangwu, root, None, "总部职员").await?;
    insert_membership(&mut tx, wangwu, north, None, "驻京代表").await?;
    insert_membership(&mut tx, tianqi, shanghai, None, "职员").await?;

    // ===== 角色 =====
    let super_role = insert_role(&mut tx, tenant, root, "超级管理员", "super_admin", "平台内置超管角色, 拥有全部菜单", true).await?;
    let org_admin_role = insert_role(&mut tx, tenant, east, "超级管理员", "org_admin", "华东区分公司超级管理员", false).await?;
    let common_role = insert_role(&mut tx, tenant, root, "普通用户", "common_user", "普通用户基础菜单", false).await?;

    // ===== 角色菜单 =====
    for m in [
        dashboard, org_dir, tenant_mgmt, tenant_auth_btn, org_mgmt, dept_mgmt, authz_mgmt,
        ur_dir, user_mgmt, role_mgmt, cfg_dir, app_mgmt, menu_mgmt, platform_settings, open_dir,
        oauth_clients_menu, device_session_menu, audit_menu, monitor_menu,
        a_overview, a_reports_dir, a_sales, a_ops, a_sources, h_inbox, h_notify,
    ] {
        grant_role_menu(&mut tx, super_role, m).await?;
    }
    for m in [dashboard, org_dir, org_mgmt, dept_mgmt, authz_mgmt, ur_dir, user_mgmt, role_mgmt] {
        grant_role_menu(&mut tx, org_admin_role, m).await?;
    }
    for m in [dashboard, a_overview, a_reports_dir, a_sales, a_ops, a_sources, h_inbox, h_notify] {
        grant_role_menu(&mut tx, common_role, m).await?;
    }

    // ===== 用户角色 =====
    grant_user_role(&mut tx, admin, super_role, root).await?;
    grant_user_role(&mut tx, zhangsan, org_admin_role, east).await?;
    grant_user_role(&mut tx, lisi, common_role, root).await?;
    grant_user_role(&mut tx, wangwu, common_role, root).await?;
    grant_user_role(&mut tx, tianqi, common_role, root).await?;

    // ===== 租户授权: 全部应用 + 全部菜单 =====
    for a in [sys, athena, hermes] {
        grant_tenant_app(&mut tx, tenant, a).await?;
    }
    for m in [
        dashboard, org_dir, tenant_mgmt, tenant_auth_btn, org_mgmt, dept_mgmt, authz_mgmt,
        ur_dir, user_mgmt, role_mgmt, cfg_dir, app_mgmt, menu_mgmt, platform_settings, open_dir,
        oauth_clients_menu, device_session_menu, audit_menu, monitor_menu,
        a_overview, a_reports_dir, a_sales, a_ops, a_sources, h_inbox, h_notify,
    ] {
        grant_tenant_menu(&mut tx, tenant, m).await?;
    }

    // ===== 组织授权链 =====
    // 总公司: 全部应用全部菜单
    for a in [sys, athena, hermes] {
        grant_org_app(&mut tx, root, a).await?;
    }
    for m in [
        dashboard, org_dir, tenant_mgmt, tenant_auth_btn, org_mgmt, dept_mgmt, authz_mgmt,
        ur_dir, user_mgmt, role_mgmt, cfg_dir, app_mgmt, menu_mgmt, platform_settings, open_dir,
        oauth_clients_menu, device_session_menu, audit_menu, monitor_menu,
        a_overview, a_reports_dir, a_sales, a_ops, a_sources, h_inbox, h_notify,
    ] {
        grant_org_menu(&mut tx, root, m).await?;
    }
    // 华东区分公司: system 子集 (组织机构/用户与角色/仪表盘, 供组织管理员使用) + athena + hermes
    for a in [sys, athena, hermes] {
        grant_org_app(&mut tx, east, a).await?;
    }
    for m in [dashboard, org_dir, org_mgmt, dept_mgmt, authz_mgmt, ur_dir, user_mgmt, role_mgmt] {
        grant_org_menu(&mut tx, east, m).await?;
    }
    for m in [a_overview, a_reports_dir, a_sales, a_ops, a_sources] {
        grant_org_menu(&mut tx, east, m).await?;
    }
    for m in [h_inbox, h_notify] {
        grant_org_menu(&mut tx, east, m).await?;
    }
    // 华北区分公司: 仅 hermes
    grant_org_app(&mut tx, north, hermes).await?;
    for m in [h_inbox, h_notify] {
        grant_org_menu(&mut tx, north, m).await?;
    }
    // 上海/杭州办事处: athena + hermes
    for org in [shanghai, hangzhou] {
        grant_org_app(&mut tx, org, athena).await?;
        grant_org_app(&mut tx, org, hermes).await?;
        for m in [a_overview, a_reports_dir, a_sales, a_ops, a_sources] {
            grant_org_menu(&mut tx, org, m).await?;
        }
        for m in [h_inbox, h_notify] {
            grant_org_menu(&mut tx, org, m).await?;
        }
    }
    // 北京办事处: 仅 hermes
    grant_org_app(&mut tx, beijing, hermes).await?;
    for m in [h_inbox, h_notify] {
        grant_org_menu(&mut tx, beijing, m).await?;
    }

    // ===== 第二租户: 星辰制造 (普通租户, 演示平台租户 vs 普通租户数据隔离) =====
    // 平台租户可管理全部租户; 星辰制造的管理员只能管理本租户内的组织/用户/角色
    let star = scalar_uuid(
        &mut tx,
        "INSERT INTO tenants (name, code, description) VALUES ('星辰制造', 'star-mfg', '演示普通租户: 仅能管理自己租户内的数据') RETURNING id",
    )
    .await?;
    let star_root = insert_org(&mut tx, star, None, "星辰集团", "STAR", 1, "星辰制造总部").await?;
    let chenjiu = insert_user(&mut tx, "chenjiu", "Cj@12345", "陈九", "chenjiu@star-mfg.example", "13900000009", "MALE", false).await?;
    insert_membership(&mut tx, chenjiu, star_root, None, "超级管理员").await?;
    let star_admin = insert_role(
        &mut tx, star, star_root, "超级管理员", "star_admin",
        "星辰制造超级管理员: 本租户内组织/用户/角色管理", false,
    ).await?;
    // 角色菜单/租户授权/组织授权三网同口径: system 管理子集 + 消息中心
    let star_menus = [dashboard, org_dir, org_mgmt, dept_mgmt, authz_mgmt, ur_dir, user_mgmt, role_mgmt, h_inbox, h_notify];
    for m in star_menus {
        grant_role_menu(&mut tx, star_admin, m).await?;
    }
    grant_user_role(&mut tx, chenjiu, star_admin, star_root).await?;
    for a in [sys, hermes] {
        grant_tenant_app(&mut tx, star, a).await?;
        grant_org_app(&mut tx, star_root, a).await?;
    }
    for m in star_menus {
        grant_tenant_menu(&mut tx, star, m).await?;
        grant_org_menu(&mut tx, star_root, m).await?;
    }

    // ===== OAuth 客户端 (secret 存 SHA-256) =====
    insert_client(&mut tx, "platform-web", Some("platform-web-secret"), "Web 控制台",
        "Web 应用演示客户端 (授权码 + PKCE)", "CONFIDENTIAL",
        "https://oauthdebugger.com/callback,http://localhost:9000/callback",
        "authorization_code,refresh_token", "profile,email", 7200, 604800).await?;
    insert_client(&mut tx, "smart-tv", None, "智能电视",
        "设备码流程演示客户端 (RFC 8628)", "PUBLIC",
        "", "urn:ietf:params:oauth:grant-type:device_code,refresh_token", "profile", 7200, 604800).await?;
    insert_client(&mut tx, "api-service", Some("api-service-secret"), "API 服务",
        "服务间调用客户端 (client_credentials)", "CONFIDENTIAL",
        "", "client_credentials", "profile", 7200, 604800).await?;
    insert_client(&mut tx, "resource-portal", Some("resource-portal-secret"), "资源门户",
        "资源密码模式演示客户端 (password)", "CONFIDENTIAL",
        "http://localhost:9000/callback", "authorization_code,password,refresh_token", "profile,email", 7200, 604800).await?;

    tx.commit().await?;
    tracing::info!(
        "种子数据写入完成: 2 租户(含 1 平台租户) / 3 应用 / 26 菜单 / 7 组织 / 6 部门 / 6 用户 / 4 角色 / 4 OAuth 客户端"
    );
    Ok(())
}

async fn scalar_uuid(tx: &mut Transaction<'_, Postgres>, sql: &str) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(sql).fetch_one(&mut **tx).await?)
}

async fn insert_app(
    tx: &mut Transaction<'_, Postgres>, code: &str, name: &str, icon: &str, base_path: &str, sort: i32,
) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO applications (code, name, icon, base_path, sort_order) VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(code)
    .bind(name)
    .bind(icon)
    .bind(base_path)
    .bind(sort)
    .fetch_one(&mut **tx)
    .await?)
}

#[allow(clippy::too_many_arguments)]
async fn insert_menu(
    tx: &mut Transaction<'_, Postgres>, app: Uuid, parent: Option<Uuid>, mtype: &str, title: &str,
    path: &str, icon: &str, component: &str, permission: &str, sort: i32, visible: bool,
) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO menus (application_id, parent_id, type, title, path, component, icon, permission, sort_order, visible) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(app)
    .bind(parent)
    .bind(mtype)
    .bind(title)
    .bind(path)
    .bind(component)
    .bind(icon)
    .bind(permission)
    .bind(sort)
    .bind(visible)
    .fetch_one(&mut **tx)
    .await?)
}

async fn insert_org(
    tx: &mut Transaction<'_, Postgres>, tenant: Uuid, parent: Option<Uuid>, name: &str, code: &str, sort: i32, desc: &str,
) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO organizations (tenant_id, parent_id, name, code, sort_order, description) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(tenant)
    .bind(parent)
    .bind(name)
    .bind(code)
    .bind(sort)
    .bind(desc)
    .fetch_one(&mut **tx)
    .await?)
}

async fn insert_dept(
    tx: &mut Transaction<'_, Postgres>, org: Uuid, parent: Option<Uuid>, name: &str, code: &str, sort: i32,
) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO departments (organization_id, parent_id, name, code, sort_order) VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(org)
    .bind(parent)
    .bind(name)
    .bind(code)
    .bind(sort)
    .fetch_one(&mut **tx)
    .await?)
}

#[allow(clippy::too_many_arguments)]
async fn insert_user(
    tx: &mut Transaction<'_, Postgres>, username: &str, password: &str, real_name: &str,
    email: &str, phone: &str, gender: &str, is_super: bool,
) -> anyhow::Result<Uuid> {
    let hash = password::hash_password(password)?;
    Ok(sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, real_name, email, phone, gender, is_super_admin) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(username)
    .bind(&hash)
    .bind(real_name)
    .bind(email)
    .bind(phone)
    .bind(gender)
    .bind(is_super)
    .fetch_one(&mut **tx)
    .await?)
}

async fn insert_membership(
    tx: &mut Transaction<'_, Postgres>, user: Uuid, org: Uuid, dept: Option<Uuid>, position: &str,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO memberships (user_id, organization_id, department_id, position) VALUES ($1, $2, $3, $4)")
        .bind(user)
        .bind(org)
        .bind(dept)
        .bind(position)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn insert_role(
    tx: &mut Transaction<'_, Postgres>, tenant: Uuid, org: Uuid, name: &str, code: &str, desc: &str, built_in: bool,
) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO roles (tenant_id, organization_id, name, code, description, built_in) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(tenant)
    .bind(org)
    .bind(name)
    .bind(code)
    .bind(desc)
    .bind(built_in)
    .fetch_one(&mut **tx)
    .await?)
}

async fn grant_role_menu(tx: &mut Transaction<'_, Postgres>, role: Uuid, menu: Uuid) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO role_menus (role_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(role)
        .bind(menu)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn grant_user_role(tx: &mut Transaction<'_, Postgres>, user: Uuid, role: Uuid, org: Uuid) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO user_roles (user_id, role_id, organization_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
        .bind(user)
        .bind(role)
        .bind(org)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn grant_tenant_app(tx: &mut Transaction<'_, Postgres>, tenant: Uuid, app: Uuid) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(tenant)
        .bind(app)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn grant_tenant_menu(tx: &mut Transaction<'_, Postgres>, tenant: Uuid, menu: Uuid) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tenant_menus (tenant_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(tenant)
        .bind(menu)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn grant_org_app(tx: &mut Transaction<'_, Postgres>, org: Uuid, app: Uuid) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(org)
        .bind(app)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn grant_org_menu(tx: &mut Transaction<'_, Postgres>, org: Uuid, menu: Uuid) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO organization_menus (organization_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(org)
        .bind(menu)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_client(
    tx: &mut Transaction<'_, Postgres>, client_id: &str, secret: Option<&str>, name: &str, desc: &str,
    client_type: &str, redirect_uris: &str, grant_types: &str, scopes: &str,
    access_validity: i32, refresh_validity: i32,
) -> anyhow::Result<()> {
    let secret_hash = secret.map(hash::sha256_hex);
    sqlx::query(
        "INSERT INTO oauth_clients \
         (client_id, client_secret_hash, client_name, description, client_type, redirect_uris, grant_types, scopes, access_token_validity, refresh_token_validity) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(client_id)
    .bind(secret_hash)
    .bind(name)
    .bind(desc)
    .bind(client_type)
    .bind(redirect_uris)
    .bind(grant_types)
    .bind(scopes)
    .bind(access_validity)
    .bind(refresh_validity)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
