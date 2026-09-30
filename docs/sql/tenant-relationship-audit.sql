-- Phase 1 只读审计：检测平台核心关系是否跨租户/组织或形成组织环路。
-- 运行：psql -X -v ON_ERROR_STOP=1 "$DATABASE_URL" -f docs/sql/tenant-relationship-audit.sql
-- 结果为空表示本文件列出的关系异常未发现；这不是全部 API 安全性的证明。
-- 请在数据库备份和访问控制允许的环境中执行，保存输出供 Phase 2 评审。

BEGIN READ ONLY;
SET LOCAL statement_timeout = '60s';

WITH RECURSIVE org_walk AS (
    SELECT o.id AS origin_id, o.id AS current_id, o.parent_id,
           ARRAY[o.id] AS path, false AS cycle
    FROM organizations o
    UNION ALL
    SELECT w.origin_id, parent.id, parent.parent_id,
           w.path || parent.id, parent.id = ANY(w.path)
    FROM org_walk w
    JOIN organizations parent ON parent.id = w.parent_id
    WHERE NOT w.cycle
),
dept_walk AS (
    SELECT d.id AS origin_id, d.id AS current_id, d.parent_id,
           ARRAY[d.id] AS path, false AS cycle
    FROM departments d
    UNION ALL
    SELECT w.origin_id, parent.id, parent.parent_id,
           w.path || parent.id, parent.id = ANY(w.path)
    FROM dept_walk w
    JOIN departments parent ON parent.id = w.parent_id
    WHERE NOT w.cycle
),
issues AS (
    SELECT 'ORG_PARENT_CROSS_TENANT'::text AS issue,
           child.id::text AS entity_id,
           jsonb_build_object('tenantId', child.tenant_id, 'parentId', parent.id,
                              'parentTenantId', parent.tenant_id) AS detail
    FROM organizations child
    JOIN organizations parent ON parent.id = child.parent_id
    WHERE child.tenant_id <> parent.tenant_id

    UNION ALL
    SELECT 'ORG_CYCLE', w.origin_id::text,
           jsonb_build_object('path', w.path)
    FROM org_walk w
    WHERE w.cycle

    UNION ALL
    SELECT 'DEPARTMENT_PARENT_CROSS_ORG', child.id::text,
           jsonb_build_object('organizationId', child.organization_id,
                              'parentId', parent.id,
                              'parentOrganizationId', parent.organization_id)
    FROM departments child
    JOIN departments parent ON parent.id = child.parent_id
    WHERE child.organization_id <> parent.organization_id

    UNION ALL
    SELECT 'DEPARTMENT_CYCLE', w.origin_id::text,
           jsonb_build_object('path', w.path)
    FROM dept_walk w
    WHERE w.cycle

    UNION ALL
    SELECT 'MEMBERSHIP_DEPARTMENT_CROSS_ORG', m.id::text,
           jsonb_build_object('userId', m.user_id,
                              'organizationId', m.organization_id,
                              'departmentId', d.id,
                              'departmentOrganizationId', d.organization_id)
    FROM memberships m
    JOIN departments d ON d.id = m.department_id
    WHERE m.organization_id <> d.organization_id

    UNION ALL
    SELECT 'ROLE_TENANT_MISMATCH', r.id::text,
           jsonb_build_object('roleTenantId', r.tenant_id,
                              'organizationId', o.id,
                              'organizationTenantId', o.tenant_id)
    FROM roles r
    JOIN organizations o ON o.id = r.organization_id
    WHERE r.tenant_id <> o.tenant_id

    UNION ALL
    SELECT 'USER_ROLE_CONTEXT_MISMATCH',
           concat(ur.user_id, '/', ur.role_id),
           jsonb_build_object('roleOrganizationId', r.organization_id,
                              'assignmentOrganizationId', ur.organization_id)
    FROM user_roles ur
    JOIN roles r ON r.id = ur.role_id
    WHERE ur.organization_id <> r.organization_id

    UNION ALL
    SELECT 'ORG_APP_WITHOUT_TENANT_GRANT',
           concat(oa.organization_id, '/', oa.application_id),
           jsonb_build_object('tenantId', o.tenant_id,
                              'organizationId', oa.organization_id,
                              'applicationId', oa.application_id)
    FROM organization_applications oa
    JOIN organizations o ON o.id = oa.organization_id
    WHERE NOT EXISTS (
        SELECT 1 FROM tenant_applications ta
        WHERE ta.tenant_id = o.tenant_id
          AND ta.application_id = oa.application_id
    )

    UNION ALL
    SELECT 'ORG_MENU_WITHOUT_TENANT_GRANT',
           concat(om.organization_id, '/', om.menu_id),
           jsonb_build_object('tenantId', o.tenant_id,
                              'organizationId', om.organization_id,
                              'menuId', om.menu_id)
    FROM organization_menus om
    JOIN organizations o ON o.id = om.organization_id
    WHERE NOT EXISTS (
        SELECT 1 FROM tenant_menus tm
        WHERE tm.tenant_id = o.tenant_id
          AND tm.menu_id = om.menu_id
    )

    UNION ALL
    SELECT 'ROLE_MENU_WITHOUT_ORG_GRANT',
           concat(rm.role_id, '/', rm.menu_id),
           jsonb_build_object('roleOrganizationId', r.organization_id,
                              'roleTenantId', r.tenant_id,
                              'menuId', rm.menu_id)
    FROM role_menus rm
    JOIN roles r ON r.id = rm.role_id
    WHERE NOT EXISTS (
        SELECT 1 FROM organization_menus om
        WHERE om.organization_id = r.organization_id
          AND om.menu_id = rm.menu_id
    )
)
SELECT issue, entity_id, detail
FROM issues
ORDER BY issue, entity_id;

ROLLBACK;
