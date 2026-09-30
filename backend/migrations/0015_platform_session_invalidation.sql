-- 会话上下文沿组织祖先链生效；失效状态不可因后续恢复而复活旧会话。
CREATE FUNCTION org_chain_active(target_org UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    WITH RECURSIVE chain AS (
        SELECT id, parent_id, status, expires_at
        FROM organizations WHERE id = target_org
        UNION ALL
        SELECT parent.id, parent.parent_id, parent.status, parent.expires_at
        FROM chain child JOIN organizations parent ON parent.id = child.parent_id
    )
    SELECT COALESCE(bool_and(status = 'ENABLED'
                             AND (expires_at IS NULL OR expires_at > now())), false)
    FROM chain;
$$;

CREATE FUNCTION org_chain_expires_at(target_org UUID) RETURNS TIMESTAMPTZ
LANGUAGE SQL STABLE AS $$
    WITH RECURSIVE chain AS (
        SELECT id, parent_id, expires_at
        FROM organizations WHERE id = target_org
        UNION ALL
        SELECT parent.id, parent.parent_id, parent.expires_at
        FROM chain child JOIN organizations parent ON parent.id = child.parent_id
    )
    SELECT min(expires_at) FROM chain;
$$;

CREATE FUNCTION revoke_sessions_on_membership_delete() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE platform_sessions SET revoked_at = now()
    WHERE user_id = OLD.user_id AND organization_id = OLD.organization_id
      AND revoked_at IS NULL;
    RETURN OLD;
END;
$$;

CREATE TRIGGER trg_membership_session_revoke
AFTER DELETE ON memberships
FOR EACH ROW EXECUTE FUNCTION revoke_sessions_on_membership_delete();

CREATE FUNCTION sync_sessions_on_organization_state() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.status <> 'ENABLED' OR (NEW.expires_at IS NOT NULL AND NEW.expires_at <= now()) THEN
        WITH RECURSIVE descendants AS (
            SELECT NEW.id AS id
            UNION ALL
            SELECT child.id FROM organizations child
            JOIN descendants parent ON child.parent_id = parent.id
        )
        UPDATE platform_sessions SET revoked_at = now()
        WHERE organization_id IN (SELECT id FROM descendants) AND revoked_at IS NULL;
    ELSIF NEW.expires_at IS NOT NULL THEN
        WITH RECURSIVE descendants AS (
            SELECT NEW.id AS id
            UNION ALL
            SELECT child.id FROM organizations child
            JOIN descendants parent ON child.parent_id = parent.id
        )
        UPDATE platform_sessions SET expires_at = LEAST(expires_at, NEW.expires_at)
        WHERE organization_id IN (SELECT id FROM descendants) AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_organization_session_state
AFTER UPDATE OF status, expires_at ON organizations
FOR EACH ROW EXECUTE FUNCTION sync_sessions_on_organization_state();

CREATE FUNCTION sync_sessions_on_tenant_state() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.status <> 'ENABLED' OR (NEW.expires_at IS NOT NULL AND NEW.expires_at <= now()) THEN
        UPDATE platform_sessions SET revoked_at = now()
        WHERE tenant_id = NEW.id AND revoked_at IS NULL;
    ELSIF NEW.expires_at IS NOT NULL THEN
        UPDATE platform_sessions SET expires_at = LEAST(expires_at, NEW.expires_at)
        WHERE tenant_id = NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_tenant_session_state
AFTER UPDATE OF status, expires_at ON tenants
FOR EACH ROW EXECUTE FUNCTION sync_sessions_on_tenant_state();

CREATE FUNCTION bump_session_version_on_user_disable() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        NEW.session_version := OLD.session_version + 1;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION revoke_sessions_on_user_disable() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        UPDATE platform_sessions SET revoked_at = now()
        WHERE user_id = NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_user_session_version
BEFORE UPDATE OF status ON users
FOR EACH ROW EXECUTE FUNCTION bump_session_version_on_user_disable();

CREATE TRIGGER trg_user_session_state
AFTER UPDATE OF status ON users
FOR EACH ROW EXECUTE FUNCTION revoke_sessions_on_user_disable();
