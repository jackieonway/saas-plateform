-- Phase 2：核心组织关系的数据库级租户/组织一致性。
-- 只增加结构不变量；发现存量异常时中止迁移，不静默重写或删除数据。

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM organizations child
        JOIN organizations parent ON parent.id = child.parent_id
        WHERE child.tenant_id <> parent.tenant_id
    ) THEN
        RAISE EXCEPTION '0013 preflight: organizations parent crosses tenant';
    END IF;
    IF EXISTS (
        SELECT 1 FROM departments child
        JOIN departments parent ON parent.id = child.parent_id
        WHERE child.organization_id <> parent.organization_id
    ) THEN
        RAISE EXCEPTION '0013 preflight: departments parent crosses organization';
    END IF;
    IF EXISTS (
        SELECT 1 FROM memberships m
        JOIN departments d ON d.id = m.department_id
        WHERE m.organization_id <> d.organization_id
    ) THEN
        RAISE EXCEPTION '0013 preflight: membership department crosses organization';
    END IF;
    IF EXISTS (
        SELECT 1 FROM roles r
        JOIN organizations o ON o.id = r.organization_id
        WHERE r.tenant_id <> o.tenant_id
    ) THEN
        RAISE EXCEPTION '0013 preflight: role tenant differs from organization tenant';
    END IF;
    IF EXISTS (
        WITH RECURSIVE walk AS (
            SELECT id AS origin_id, parent_id, ARRAY[id] AS path, false AS cycle
            FROM organizations
            UNION ALL
            SELECT w.origin_id, parent.parent_id, w.path || parent.id,
                   parent.id = ANY(w.path)
            FROM walk w JOIN organizations parent ON parent.id = w.parent_id
            WHERE NOT w.cycle
        )
        SELECT 1 FROM walk WHERE cycle
    ) THEN
        RAISE EXCEPTION '0013 preflight: organization hierarchy contains a cycle';
    END IF;
    IF EXISTS (
        WITH RECURSIVE walk AS (
            SELECT id AS origin_id, parent_id, ARRAY[id] AS path, false AS cycle
            FROM departments
            UNION ALL
            SELECT w.origin_id, parent.parent_id, w.path || parent.id,
                   parent.id = ANY(w.path)
            FROM walk w JOIN departments parent ON parent.id = w.parent_id
            WHERE NOT w.cycle
        )
        SELECT 1 FROM walk WHERE cycle
    ) THEN
        RAISE EXCEPTION '0013 preflight: department hierarchy contains a cycle';
    END IF;
END
$$;

-- 复合外键目标列需要唯一约束；id 本身已是主键，下列索引只用于表达复合归属。
CREATE UNIQUE INDEX IF NOT EXISTS uq_organizations_id_tenant
    ON organizations (id, tenant_id);
CREATE UNIQUE INDEX IF NOT EXISTS uq_departments_id_organization
    ON departments (id, organization_id);

ALTER TABLE organizations
    ADD CONSTRAINT fk_organizations_parent_same_tenant
    FOREIGN KEY (parent_id, tenant_id)
    REFERENCES organizations (id, tenant_id)
    DEFERRABLE INITIALLY IMMEDIATE;

ALTER TABLE departments
    ADD CONSTRAINT fk_departments_parent_same_organization
    FOREIGN KEY (parent_id, organization_id)
    REFERENCES departments (id, organization_id)
    DEFERRABLE INITIALLY IMMEDIATE;

ALTER TABLE memberships
    ADD CONSTRAINT fk_memberships_department_same_organization
    FOREIGN KEY (department_id, organization_id)
    REFERENCES departments (id, organization_id)
    DEFERRABLE INITIALLY IMMEDIATE;

ALTER TABLE roles
    ADD CONSTRAINT fk_roles_organization_same_tenant
    FOREIGN KEY (organization_id, tenant_id)
    REFERENCES organizations (id, tenant_id)
    DEFERRABLE INITIALLY IMMEDIATE;

CREATE OR REPLACE FUNCTION reject_organization_cycle() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.parent_id IS NULL THEN
        RETURN NEW;
    END IF;
    -- 同租户层级变更串行化，防并发事务各自通过检查后共同形成环路。
    PERFORM pg_advisory_xact_lock(hashtextextended(NEW.tenant_id::text, 0));
    IF NEW.parent_id = NEW.id THEN
        RAISE EXCEPTION 'organization cannot be its own parent';
    END IF;
    IF EXISTS (
        WITH RECURSIVE ancestors AS (
            SELECT o.id, o.parent_id, ARRAY[o.id] AS path
            FROM organizations o WHERE o.id = NEW.parent_id
            UNION ALL
            SELECT parent.id, parent.parent_id, ancestors.path || parent.id
            FROM ancestors JOIN organizations parent ON parent.id = ancestors.parent_id
            WHERE NOT parent.id = ANY(ancestors.path)
        )
        SELECT 1 FROM ancestors WHERE id = NEW.id
    ) THEN
        RAISE EXCEPTION 'organization hierarchy cannot contain a cycle';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER trg_organizations_no_cycle
    BEFORE INSERT OR UPDATE OF parent_id ON organizations
    FOR EACH ROW EXECUTE FUNCTION reject_organization_cycle();

CREATE OR REPLACE FUNCTION reject_department_cycle() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.parent_id IS NULL THEN
        RETURN NEW;
    END IF;
    -- 同组织部门层级变更串行化。
    PERFORM pg_advisory_xact_lock(hashtextextended(NEW.organization_id::text, 1));
    IF NEW.parent_id = NEW.id THEN
        RAISE EXCEPTION 'department cannot be its own parent';
    END IF;
    IF EXISTS (
        WITH RECURSIVE ancestors AS (
            SELECT d.id, d.parent_id, ARRAY[d.id] AS path
            FROM departments d WHERE d.id = NEW.parent_id
            UNION ALL
            SELECT parent.id, parent.parent_id, ancestors.path || parent.id
            FROM ancestors JOIN departments parent ON parent.id = ancestors.parent_id
            WHERE NOT parent.id = ANY(ancestors.path)
        )
        SELECT 1 FROM ancestors WHERE id = NEW.id
    ) THEN
        RAISE EXCEPTION 'department hierarchy cannot contain a cycle';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER trg_departments_no_cycle
    BEFORE INSERT OR UPDATE OF parent_id ON departments
    FOR EACH ROW EXECUTE FUNCTION reject_department_cycle();
