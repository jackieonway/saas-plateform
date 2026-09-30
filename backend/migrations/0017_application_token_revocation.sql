-- 应用授权和客户端停用后，旧应用令牌不可因再次授权而复活。
CREATE FUNCTION revoke_app_tokens_on_tenant_grant_delete() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE oauth_tokens SET revoked = true
    WHERE token_use = 'app' AND tenant_id = OLD.tenant_id
      AND application_id = OLD.application_id AND revoked = false;
    RETURN OLD;
END;
$$;

CREATE TRIGGER trg_app_token_tenant_grant_revoke
AFTER DELETE ON tenant_applications
FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_tenant_grant_delete();

CREATE FUNCTION revoke_app_tokens_on_org_grant_delete() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE oauth_tokens SET revoked = true
    WHERE token_use = 'app' AND organization_id = OLD.organization_id
      AND application_id = OLD.application_id AND revoked = false;
    RETURN OLD;
END;
$$;

CREATE TRIGGER trg_app_token_org_grant_revoke
AFTER DELETE ON organization_applications
FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_org_grant_delete();

CREATE FUNCTION revoke_app_tokens_on_application_disable() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND application_id = NEW.id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_app_token_application_revoke
AFTER UPDATE OF status ON applications
FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_application_disable();

CREATE FUNCTION revoke_app_tokens_on_client_disable() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND client_id = NEW.client_id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_app_token_client_revoke
AFTER UPDATE OF status ON oauth_clients
FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_client_disable();
