-- Phase 1 最小授权矩阵：两个租户、多层组织、跨租户成员、不同菜单授权。
-- 运行：psql -X -v ON_ERROR_STOP=1 "$DATABASE_URL" -f tests/tenant-isolation-policy-fixture.sql
-- 所有表均为 TEMP，事务末尾 ROLLBACK；不读取或修改项目业务表。
-- current_allowed 模拟 backend/src/state.rs::permissions_of 的关键 SQL 条件。
-- target_allowed 增加推荐的直接成员、组织/租户状态和应用授权门禁。
-- 当前行为与目标不一致的用例会以 CURRENT_GAP 显示；它们是后续阶段的回归目标。

BEGIN;
SET LOCAL statement_timeout = '30s';

CREATE TEMP TABLE baseline_results AS
WITH RECURSIVE
tenants(id, status) AS (
    VALUES ('ta', 'ENABLED'), ('tb', 'ENABLED')
),
organizations(id, tenant_id, parent_id, status) AS (
    VALUES ('a-root', 'ta', NULL::text, 'ENABLED'),
           ('a-child', 'ta', 'a-root', 'ENABLED'),
           ('a-disabled', 'ta', 'a-root', 'DISABLED'),
           ('b-root', 'tb', NULL::text, 'ENABLED'),
           ('b-child', 'tb', 'b-root', 'ENABLED')
),
memberships(user_id, organization_id) AS (
    VALUES ('shared', 'a-child'), ('shared', 'a-disabled'),
           ('shared', 'b-child'), ('a-only', 'a-child')
),
applications(id) AS (VALUES ('app-a'), ('app-b')),
menus(id, application_id, permission, status) AS (
    VALUES ('ma', 'app-a', 'app-a:read', 'ENABLED'),
           ('ma-no-org', 'app-a', 'app-a:admin', 'ENABLED'),
           ('mb', 'app-b', 'app-b:read', 'ENABLED')
),
roles(id, organization_id, status) AS (
    VALUES ('ra', 'a-root', 'ENABLED'), ('rb', 'b-root', 'ENABLED')
),
user_roles(user_id, role_id) AS (
    VALUES ('shared', 'ra'), ('shared', 'rb'),
           ('a-only', 'ra'), ('removed', 'ra')
),
role_menus(role_id, menu_id) AS (
    VALUES ('ra', 'ma'), ('ra', 'ma-no-org'), ('rb', 'mb')
),
tenant_applications(tenant_id, application_id) AS (
    VALUES ('ta', 'app-a'), ('tb', 'app-b')
),
organization_applications(organization_id, application_id) AS (
    VALUES ('a-child', 'app-a'), ('a-disabled', 'app-a'),
           ('b-child', 'app-b')
),
tenant_menus(tenant_id, menu_id) AS (
    VALUES ('ta', 'ma'), ('ta', 'ma-no-org'), ('tb', 'mb')
),
organization_menus(organization_id, menu_id) AS (
    VALUES ('a-child', 'ma'), ('a-disabled', 'ma'),
           ('b-child', 'mb')
),
cases(case_id, user_id, organization_id, menu_id, expected) AS (
    VALUES ('A_DIRECT_INHERITED_ROLE', 'shared', 'a-child', 'ma', true),
           ('B_DIRECT_INHERITED_ROLE', 'shared', 'b-child', 'mb', true),
           ('A_USER_CANNOT_ENTER_B', 'a-only', 'b-child', 'mb', false),
           ('A_CONTEXT_CANNOT_USE_B_MENU', 'shared', 'a-child', 'mb', false),
           ('NO_ORG_MENU_GRANT', 'shared', 'a-child', 'ma-no-org', false),
           ('REMOVED_MEMBER_OLD_ROLE', 'removed', 'a-child', 'ma', false),
           ('DISABLED_ORGANIZATION', 'shared', 'a-disabled', 'ma', false)
),
ancestors(case_id, id, parent_id) AS (
    SELECT c.case_id, o.id, o.parent_id
    FROM cases c JOIN organizations o ON o.id = c.organization_id
    UNION ALL
    SELECT a.case_id, parent.id, parent.parent_id
    FROM ancestors a JOIN organizations parent ON parent.id = a.parent_id
),
decisions AS (
    SELECT c.case_id, c.user_id, c.organization_id, c.menu_id, c.expected,
           o.tenant_id, o.status AS organization_status,
           t.status AS tenant_status, m.application_id,
           EXISTS (
               SELECT 1
               FROM user_roles ur
               JOIN roles r ON r.id = ur.role_id AND r.status = 'ENABLED'
               JOIN ancestors a ON a.case_id = c.case_id AND a.id = r.organization_id
               JOIN role_menus rm ON rm.role_id = r.id AND rm.menu_id = c.menu_id
               WHERE ur.user_id = c.user_id
                 AND m.status = 'ENABLED'
                 AND EXISTS (SELECT 1 FROM organization_menus om
                             WHERE om.organization_id = c.organization_id AND om.menu_id = c.menu_id)
                 AND EXISTS (SELECT 1 FROM tenant_menus tm
                             WHERE tm.tenant_id = o.tenant_id AND tm.menu_id = c.menu_id)
           ) AS current_allowed,
           EXISTS (SELECT 1 FROM memberships mb
                   WHERE mb.user_id = c.user_id AND mb.organization_id = c.organization_id)
               AS direct_member,
           EXISTS (SELECT 1 FROM tenant_applications ta
                   WHERE ta.tenant_id = o.tenant_id AND ta.application_id = m.application_id)
               AS tenant_app_grant,
           EXISTS (SELECT 1 FROM organization_applications oa
                   WHERE oa.organization_id = c.organization_id AND oa.application_id = m.application_id)
               AS org_app_grant
    FROM cases c
    JOIN organizations o ON o.id = c.organization_id
    JOIN tenants t ON t.id = o.tenant_id
    JOIN menus m ON m.id = c.menu_id
)
SELECT case_id, user_id, organization_id, menu_id, expected, current_allowed,
       (current_allowed AND direct_member
        AND organization_status = 'ENABLED' AND tenant_status = 'ENABLED'
        AND tenant_app_grant AND org_app_grant) AS target_allowed,
       CASE WHEN current_allowed <> expected THEN 'CURRENT_GAP' ELSE 'CURRENT_MATCH' END AS baseline
FROM decisions;

SELECT case_id, expected, current_allowed, target_allowed, baseline
FROM baseline_results ORDER BY case_id;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM baseline_results WHERE target_allowed <> expected) THEN
        RAISE EXCEPTION 'Phase 1 target policy matrix mismatch';
    END IF;
    IF (SELECT count(*) FROM baseline_results) <> 7 THEN
        RAISE EXCEPTION 'Phase 1 fixture case count changed';
    END IF;
END
$$;

ROLLBACK;
