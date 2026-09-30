-- 应用 code 或客户端绑定/令牌配置变更时，旧受众声明不可复活。
CREATE FUNCTION revoke_app_tokens_on_app_code_change() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.code IS DISTINCT FROM NEW.code THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND application_id = NEW.id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_app_token_code_revoke
AFTER UPDATE OF code ON applications
FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_app_code_change();

CREATE FUNCTION revoke_app_tokens_on_client_config_change() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.application_id IS DISTINCT FROM NEW.application_id
       OR OLD.client_type IS DISTINCT FROM NEW.client_type
       OR OLD.scopes IS DISTINCT FROM NEW.scopes
       OR OLD.access_token_validity IS DISTINCT FROM NEW.access_token_validity THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND client_id = NEW.client_id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_app_token_client_config_revoke
AFTER UPDATE OF application_id, client_type, scopes, access_token_validity ON oauth_clients
FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_client_config_change();
