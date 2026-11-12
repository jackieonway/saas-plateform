-- 在当前结构基线之后、角色权限交接之前，由初始化作业在事务中执行。
-- 调用方通过事务级 saas.bootstrap.username / password_hash 传入账号信息。
-- 不在建表迁移中固化共享密码，也不重置已初始化账号。
SELECT pg_advisory_xact_lock(764351204988213);

CREATE OR REPLACE FUNCTION pg_temp.platform_bootstrap_uuid_v7() RETURNS uuid
LANGUAGE sql VOLATILE AS $$
    SELECT (lpad(to_hex(floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint),12,'0')
        || '7' || substr(replace(pg_catalog.gen_random_uuid()::text,'-',''),14))::uuid
$$;

CREATE TEMP TABLE platform_bootstrap_result (
    created boolean NOT NULL, tenant_id uuid NOT NULL,
    organization_id uuid NOT NULL, admin_id uuid NOT NULL
) ON COMMIT DROP;

DO $bootstrap$
DECLARE
    v_username text := current_setting('saas.bootstrap.username');
    v_hash text := current_setting('saas.bootstrap.password_hash');
    v_tenant uuid; v_root uuid; v_admin uuid; v_role uuid;
    v_system uuid; v_hermes uuid; v_menu uuid; v_parent uuid;
    item record;
BEGIN
    IF NOT EXISTS(SELECT 1 FROM public._sqlx_migrations WHERE version=1 AND success) THEN
        RAISE EXCEPTION 'PLATFORM_BOOTSTRAP_BASELINE_REQUIRED';
    END IF;
    IF v_username !~ '^[A-Za-z0-9_.-]{3,64}$' OR v_hash NOT LIKE '$argon2id$%' THEN
        RAISE EXCEPTION 'PLATFORM_BOOTSTRAP_CREDENTIALS_INVALID';
    END IF;
    SELECT id INTO v_tenant FROM tenants WHERE is_platform;
    IF FOUND THEN
        SELECT o.id,u.id,r.id INTO v_root,v_admin,v_role
        FROM organizations o
        JOIN memberships m ON m.organization_id=o.id AND m.member_type='EMPLOYEE' AND m.status='ENABLED'
        JOIN users u ON u.id=m.user_id AND u.username=v_username AND u.is_super_admin AND u.status='ENABLED'
        JOIN user_roles ur ON ur.user_id=u.id AND ur.organization_id=o.id
        JOIN roles r ON r.id=ur.role_id AND r.organization_id=o.id AND r.code='super_admin' AND r.built_in
        WHERE o.tenant_id=v_tenant AND o.parent_id IS NULL AND o.code='HQ';
        IF v_admin IS NULL THEN
            RAISE EXCEPTION 'PLATFORM_BOOTSTRAP_EXISTING_DATA_CONFLICT';
        END IF;
        INSERT INTO platform_bootstrap_result VALUES(false,v_tenant,v_root,v_admin);
        RETURN;
    END IF;
    IF EXISTS(SELECT 1 FROM tenants) OR EXISTS(SELECT 1 FROM organizations)
       OR EXISTS(SELECT 1 FROM users) OR EXISTS(SELECT 1 FROM applications) THEN
        RAISE EXCEPTION 'PLATFORM_BOOTSTRAP_EMPTY_DATABASE_REQUIRED';
    END IF;
    v_tenant := pg_temp.platform_bootstrap_uuid_v7();
    v_root := pg_temp.platform_bootstrap_uuid_v7();
    v_admin := pg_temp.platform_bootstrap_uuid_v7();
    v_role := pg_temp.platform_bootstrap_uuid_v7();
    v_system := pg_temp.platform_bootstrap_uuid_v7();
    v_hermes := pg_temp.platform_bootstrap_uuid_v7();
    INSERT INTO tenants(id,name,code,description,is_platform)
        VALUES(v_tenant,'平台管理租户','admin-tenant','平台控制层',true);
    INSERT INTO organizations(id,tenant_id,name,code,description)
        VALUES(v_root,v_tenant,'平台总部','HQ','平台管理组织');
    INSERT INTO users(id,username,password_hash,real_name,is_super_admin)
        VALUES(v_admin,v_username,v_hash,'平台超级管理员',true);
    INSERT INTO memberships(id,user_id,organization_id,member_type,position)
        VALUES(pg_temp.platform_bootstrap_uuid_v7(),v_admin,v_root,'EMPLOYEE','平台超级管理员');
    INSERT INTO roles(id,tenant_id,organization_id,name,code,description,built_in)
        VALUES(v_role,v_tenant,v_root,'平台管理员','super_admin','平台内置管理角色',true);
    INSERT INTO user_roles(user_id,role_id,organization_id) VALUES(v_admin,v_role,v_root);

    INSERT INTO applications(id,code,name,icon,base_path,sort_order,allowed_member_types) VALUES
        (v_system,'system','系统管理','LayoutGrid','/system',1,ARRAY['EMPLOYEE']),
        (v_hermes,'hermes','消息中心','MessageSquare','/hermes',2,ARRAY['EMPLOYEE']);
    INSERT INTO tenant_applications(tenant_id,application_id) VALUES(v_tenant,v_system),(v_tenant,v_hermes);
    INSERT INTO organization_applications(organization_id,application_id) VALUES(v_root,v_system),(v_root,v_hermes);

    FOR item IN SELECT * FROM (VALUES
        ('组织机构','Building2',2),('用户与角色','Users',3),
        ('平台配置','Settings',4),('开放平台','KeyRound',5)
    ) AS dirs(title,icon,sort_order) LOOP
        INSERT INTO menus(id,application_id,type,title,icon,sort_order)
            VALUES(pg_temp.platform_bootstrap_uuid_v7(),v_system,'DIR',item.title,item.icon,item.sort_order);
    END LOOP;

    FOR item IN SELECT * FROM (VALUES
        ('system','仪表盘','/dashboard','Gauge','dashboard:view',1,NULL),
        ('system','租户管理','/system/tenants','Building','system:tenant',2,'组织机构'),
        ('system','组织管理','/system/organizations','Network','system:organization',3,'组织机构'),
        ('system','部门管理','/system/departments','FolderTree','system:department',4,'组织机构'),
        ('system','授权管理','/system/authorizations','ShieldCheck','system:authorization',5,'组织机构'),
        ('system','用户管理','/system/users','User','system:user',6,'用户与角色'),
        ('system','客户管理','/system/customers','UsersRound','system:user',7,'用户与角色'),
        ('system','角色管理','/system/roles','Shield','system:role',8,'用户与角色'),
        ('system','应用管理','/system/applications','Grid3x3','system:application',9,'平台配置'),
        ('system','菜单管理','/system/menus','ListTree','system:menu',10,'平台配置'),
        ('system','平台设置','/system/settings','Settings','system:settings',11,'平台配置'),
        ('system','客户端管理','/system/oauth/clients','Key','system:oauth',12,'开放平台'),
        ('system','审计日志','/system/audit-logs','ScrollText','system:audit',13,'开放平台'),
        ('system','平台监控','/system/monitor','Activity','system:monitor',14,'开放平台'),
        ('hermes','收件箱','/hermes/inbox','Inbox','hermes:inbox',15,NULL),
        ('hermes','通知设置','/hermes/notifications','Bell','hermes:notification',16,NULL)
    ) AS entries(app,title,path,icon,permission,sort_order,directory) LOOP
        SELECT id INTO v_parent FROM menus WHERE application_id=v_system AND type='DIR' AND title=item.directory;
        INSERT INTO menus(id,application_id,parent_id,type,title,path,icon,permission,sort_order)
        VALUES(pg_temp.platform_bootstrap_uuid_v7(),CASE WHEN item.app='system' THEN v_system ELSE v_hermes END,
            v_parent,'MENU',item.title,item.path,item.icon,item.permission,item.sort_order);
    END LOOP;

    FOR item IN SELECT * FROM (VALUES
        ('/system/tenants','授权','system:tenant:auth',1),
        ('/system/applications','查看镜像部署','system:deployment:read',20),
        ('/system/applications','管理镜像部署','system:deployment:write',21),
        ('/system/applications','查看ZIP分发','system:distribution:read',22),
        ('/system/applications','管理ZIP分发','system:distribution:write',23),
        ('/system/authorizations','查询数据授权','system:data-grant:read',1),
        ('/system/authorizations','维护数据授权','system:data-grant:write',2),
        ('/system/oauth/clients','查看客户端','system:oauth:view',1),
        ('/system/oauth/clients','新增客户端','system:oauth:register',2),
        ('/system/oauth/clients','轮换客户端凭据','system:oauth:rotate',3),
        ('/system/oauth/clients','停用客户端','system:oauth:disable',4),
        ('/system/oauth/clients','管理授权范围','system:oauth:scopes',5)
    ) AS buttons(path,title,permission,sort_order) LOOP
        SELECT id INTO STRICT v_parent FROM menus WHERE application_id=v_system AND path=item.path;
        INSERT INTO menus(id,application_id,parent_id,type,title,permission,sort_order,visible)
        VALUES(pg_temp.platform_bootstrap_uuid_v7(),v_system,v_parent,'BUTTON',item.title,item.permission,item.sort_order,false);
    END LOOP;
    INSERT INTO role_menus(role_id,menu_id) SELECT v_role,id FROM menus WHERE application_id IN(v_system,v_hermes);
    INSERT INTO tenant_menus(tenant_id,menu_id) SELECT v_tenant,id FROM menus WHERE application_id IN(v_system,v_hermes);
    INSERT INTO organization_menus(organization_id,menu_id) SELECT v_root,id FROM menus WHERE application_id IN(v_system,v_hermes);
    INSERT INTO platform_bootstrap_result VALUES(true,v_tenant,v_root,v_admin);
END
$bootstrap$;
