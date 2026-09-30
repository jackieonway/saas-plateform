-- Phase 2 数据库约束测试。须在已应用 0013 的专用测试数据库执行。
-- psql -X -v ON_ERROR_STOP=1 "$TEST_DATABASE_URL" -f tests/tenant-relationship-constraints.sql
-- 仅在事务中插入随机 UUID 测试数据，末尾 ROLLBACK。

BEGIN;
SET LOCAL statement_timeout = '30s';

DO $$
DECLARE
    tenant_a uuid;
    tenant_b uuid;
    root_a uuid;
    child_a uuid;
    root_b uuid;
    dept_a uuid;
    dept_child_a uuid;
    dept_b uuid;
    user_a uuid;
    failed boolean;
BEGIN
    INSERT INTO tenants (name, code)
    VALUES ('Phase 2 tenant A', 'p2-a-' || substr(gen_random_uuid()::text, 1, 8))
    RETURNING id INTO tenant_a;
    INSERT INTO tenants (name, code)
    VALUES ('Phase 2 tenant B', 'p2-b-' || substr(gen_random_uuid()::text, 1, 8))
    RETURNING id INTO tenant_b;

    INSERT INTO organizations (tenant_id, name) VALUES (tenant_a, 'Root A') RETURNING id INTO root_a;
    INSERT INTO organizations (tenant_id, parent_id, name)
    VALUES (tenant_a, root_a, 'Child A') RETURNING id INTO child_a;
    INSERT INTO organizations (tenant_id, name) VALUES (tenant_b, 'Root B') RETURNING id INTO root_b;

    INSERT INTO departments (organization_id, name)
    VALUES (child_a, 'Dept A') RETURNING id INTO dept_a;
    INSERT INTO departments (organization_id, parent_id, name)
    VALUES (child_a, dept_a, 'Dept A child') RETURNING id INTO dept_child_a;
    INSERT INTO departments (organization_id, name)
    VALUES (root_b, 'Dept B') RETURNING id INTO dept_b;
    INSERT INTO users (username, password_hash)
    VALUES ('p2-' || substr(gen_random_uuid()::text, 1, 12), 'test-only')
    RETURNING id INTO user_a;
    INSERT INTO memberships (user_id, organization_id, department_id)
    VALUES (user_a, child_a, dept_a);
    INSERT INTO roles (tenant_id, organization_id, name)
    VALUES (tenant_a, child_a, 'Valid role');

    failed := false;
    BEGIN
        INSERT INTO organizations (tenant_id, parent_id, name)
        VALUES (tenant_b, root_a, 'Cross-tenant child');
    EXCEPTION WHEN foreign_key_violation THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'cross-tenant organization parent was accepted'; END IF;

    failed := false;
    BEGIN
        INSERT INTO departments (organization_id, parent_id, name)
        VALUES (child_a, dept_b, 'Cross-organization child');
    EXCEPTION WHEN foreign_key_violation THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'cross-organization department parent was accepted'; END IF;

    failed := false;
    BEGIN
        INSERT INTO memberships (user_id, organization_id, department_id)
        VALUES (user_a, root_b, dept_a);
    EXCEPTION WHEN foreign_key_violation THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'cross-organization membership department was accepted'; END IF;

    failed := false;
    BEGIN
        INSERT INTO roles (tenant_id, organization_id, name)
        VALUES (tenant_b, child_a, 'Cross-tenant role');
    EXCEPTION WHEN foreign_key_violation THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'role tenant mismatch was accepted'; END IF;

    failed := false;
    BEGIN
        UPDATE organizations SET parent_id = child_a WHERE id = root_a;
    EXCEPTION WHEN raise_exception THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'organization cycle was accepted'; END IF;

    failed := false;
    BEGIN
        UPDATE departments SET parent_id = dept_child_a WHERE id = dept_a;
    EXCEPTION WHEN raise_exception THEN failed := true;
    END;
    IF NOT failed THEN RAISE EXCEPTION 'department cycle was accepted'; END IF;

    RAISE NOTICE 'Phase 2 relationship constraints: valid paths accepted; six invalid paths rejected';
END
$$;

ROLLBACK;
