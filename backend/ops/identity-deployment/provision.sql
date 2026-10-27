\set ON_ERROR_STOP on
\set ECHO none
\set QUIET on
-- A privileged, single migration/provisioning job executes this once after
-- canonical fresh baseline 0001, before either zero-DDL runtime is started.
-- Variables: expected_database, deployment_id (canonical UUIDv7), issuer,
-- role_prefix (fresh roles created by create-roles.sql).
-- Database comment must already equal identity-deployment/1/<deployment_id>.
BEGIN;
CREATE TEMP TABLE identity_deployment_input (
    expected_database text NOT NULL,deployment_id uuid NOT NULL,deployment_raw text NOT NULL,issuer text NOT NULL,prefix text NOT NULL
) ON COMMIT DROP;
INSERT INTO identity_deployment_input VALUES (:'expected_database',:'deployment_id'::uuid,:'deployment_id',:'issuer',:'role_prefix');

DO $provision$
DECLARE
    cfg pg_temp.identity_deployment_input%ROWTYPE;
    migration_owner text; identity_owner text; platform_owner text; bridge_owner text;
    identity_runtime text; platform_runtime text; role_name text;
    item record; target_owner text; auth_tables text[]; column_names text;
    auth_functions text[]; bridge_functions text[]; fence_functions text[];
BEGIN
    SELECT * INTO cfg FROM pg_temp.identity_deployment_input;
    IF cfg.expected_database<>pg_catalog.current_database()
       OR cfg.prefix !~ '^[a-z][a-z0-9_]{5,31}$'
       OR NOT public.is_uuid_v7(cfg.deployment_id)
       OR cfg.deployment_id::text<>cfg.deployment_raw
       OR cfg.issuer !~ '^https://[^/?#[:space:]]+$'
       OR pg_catalog.shobj_description((SELECT oid FROM pg_catalog.pg_database WHERE datname=cfg.expected_database),'pg_database')
            IS DISTINCT FROM 'identity-deployment/1/'||cfg.deployment_id::text
       OR EXISTS(SELECT 1 FROM public.identity_deployment_state)
       OR NOT EXISTS(SELECT 1 FROM public._sqlx_migrations WHERE version=1 AND success)
       OR EXISTS(SELECT 1 FROM public._sqlx_migrations WHERE NOT success)
       OR NOT EXISTS(SELECT 1 FROM public.oidc_provider_settings WHERE singleton AND issuer=cfg.issuer)
       OR EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname<>'public'
            AND nspname<>'information_schema' AND pg_catalog.left(nspname,3)<>'pg_') THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_TARGET_OR_HISTORY_INVALID';
    END IF;
    migration_owner:=cfg.prefix||'_migration_owner'; identity_owner:=cfg.prefix||'_identity_owner'; platform_owner:=cfg.prefix||'_platform_owner';
    bridge_owner:=cfg.prefix||'_bridge_owner'; identity_runtime:=cfg.prefix||'_identity_runtime';
    platform_runtime:=cfg.prefix||'_platform_runtime';
    IF EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members m JOIN pg_catalog.pg_roles r ON r.oid=m.member
        WHERE r.rolname IN(identity_runtime,platform_runtime)) THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_RUNTIME_MEMBERSHIP_INVALID';
    END IF;
    FOREACH role_name IN ARRAY ARRAY[migration_owner,identity_owner,platform_owner,bridge_owner,identity_runtime,platform_runtime] LOOP
        IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_roles WHERE rolname=role_name AND NOT rolsuper
             AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication AND NOT rolbypassrls
             AND NOT rolinherit AND rolcanlogin=(role_name IN (identity_runtime,platform_runtime)))
           OR role_name=current_user
           OR EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members m JOIN pg_catalog.pg_roles r ON r.oid IN(m.roleid,m.member)
             WHERE r.rolname=role_name) THEN
            RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_ROLE_CLOSURE_INVALID';
        END IF;
    END LOOP;

    -- The migration job remains schema/database owner. NOLOGIN object owners
    -- get CREATE only during reassignment, then lose it before readiness.
    EXECUTE pg_catalog.format('ALTER DATABASE %I OWNER TO %I',cfg.expected_database,migration_owner);
    EXECUTE pg_catalog.format('ALTER SCHEMA public OWNER TO %I',migration_owner);
    EXECUTE pg_catalog.format('REVOKE ALL ON DATABASE %I FROM PUBLIC',cfg.expected_database);
    EXECUTE pg_catalog.format('GRANT CONNECT ON DATABASE %I TO %I,%I',cfg.expected_database,identity_runtime,platform_runtime);
    REVOKE ALL ON SCHEMA public FROM PUBLIC;
    EXECUTE pg_catalog.format('GRANT USAGE ON SCHEMA public TO %I,%I,%I,%I,%I',identity_owner,platform_owner,bridge_owner,identity_runtime,platform_runtime);
    EXECUTE pg_catalog.format('GRANT CREATE ON SCHEMA public TO %I,%I,%I',identity_owner,platform_owner,bridge_owner);
    REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC;
    REVOKE ALL ON ALL SEQUENCES IN SCHEMA public FROM PUBLIC;
    REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM PUBLIC;
    ALTER DEFAULT PRIVILEGES IN SCHEMA public REVOKE ALL ON TABLES FROM PUBLIC;
    ALTER DEFAULT PRIVILEGES IN SCHEMA public REVOKE ALL ON SEQUENCES FROM PUBLIC;
    ALTER DEFAULT PRIVILEGES IN SCHEMA public REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;
    FOREACH role_name IN ARRAY ARRAY[migration_owner,identity_owner,platform_owner,bridge_owner] LOOP
        EXECUTE pg_catalog.format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA public REVOKE ALL ON TABLES FROM PUBLIC',role_name);
        EXECUTE pg_catalog.format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA public REVOKE ALL ON SEQUENCES FROM PUBLIC',role_name);
        EXECUTE pg_catalog.format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA public REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC',role_name);
    END LOOP;

    auth_tables:=ARRAY['users','sso_sessions','sso_login_challenges','oidc_provider_settings','oidc_clients',
        'oidc_authorization_requests','oidc_grants','oidc_authorization_codes','oidc_access_tokens',
        'oidc_refresh_tokens','oidc_signing_keys','oidc_account_logout_receipts','identity_service_clients',
        'identity_audit_logs','identity_account_tombstones','identity_account_command_receipts','identity_oidc_admin_receipts','identity_account_revocation_facts',
        'oidc_context_clients','oidc_context_interactions','oidc_consents'];
    FOR item IN SELECT c.relname FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='public' AND c.relkind IN ('r','p') LOOP
        target_owner:=CASE WHEN item.relname=ANY(auth_tables) THEN identity_owner ELSE platform_owner END;
        IF item.relname IN ('_sqlx_migrations','identity_deployment_state') THEN
            target_owner:=migration_owner;
        END IF;
        EXECUTE pg_catalog.format('ALTER TABLE public.%I OWNER TO %I',item.relname,target_owner);
        EXECUTE pg_catalog.format('REVOKE ALL ON public.%I FROM %I,%I,%I,%I,%I',item.relname,
            identity_owner,platform_owner,bridge_owner,identity_runtime,platform_runtime);
        -- Table-level REVOKE does not remove pre-existing column ACLs.
        SELECT pg_catalog.string_agg(pg_catalog.quote_ident(a.attname),',' ORDER BY a.attnum) INTO column_names
        FROM pg_catalog.pg_attribute a WHERE a.attrelid=pg_catalog.to_regclass('public.'||pg_catalog.quote_ident(item.relname))
          AND a.attnum>0 AND NOT a.attisdropped;
        EXECUTE pg_catalog.format('REVOKE SELECT(%s),INSERT(%s),UPDATE(%s),REFERENCES(%s) ON public.%I FROM PUBLIC,%I,%I,%I,%I,%I',
            column_names,column_names,column_names,column_names,item.relname,
            identity_owner,platform_owner,bridge_owner,identity_runtime,platform_runtime);
        -- Ownership preserves DDL/grant authority, but ordinary owner SELECT
        -- and DML can themselves be revoked. Restore only the actual NOLOGIN
        -- object owner, so its reviewed SECURITY DEFINER functions can work.
        -- Runtime and non-owner grants remain the narrow matrix below.
        EXECUTE pg_catalog.format('GRANT ALL PRIVILEGES ON public.%I TO %I',item.relname,target_owner);
        IF EXISTS(SELECT 1 FROM pg_catalog.unnest(ARRAY['SELECT','INSERT','UPDATE','DELETE']) AS owner_permission(privilege_name)
            WHERE NOT pg_catalog.has_table_privilege(target_owner,'public.'||pg_catalog.quote_ident(item.relname),owner_permission.privilege_name)) THEN
            RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_OWNER_ORDINARY_PRIVILEGE_INVALID';
        END IF;
        IF item.relname IN ('_sqlx_migrations','identity_deployment_state') THEN
            EXECUTE pg_catalog.format('GRANT SELECT ON public.%I TO %I,%I',item.relname,identity_runtime,platform_runtime);
        ELSIF item.relname='users' THEN
            EXECUTE pg_catalog.format('GRANT SELECT(id,username,password_hash,real_name,email,phone,avatar,gender,status,last_login_at,created_at,updated_at,session_version), INSERT(id,username,password_hash,real_name,email,phone,avatar,gender,status,last_login_at,created_at,updated_at,session_version), UPDATE(username,password_hash,real_name,email,phone,avatar,gender,status,last_login_at,created_at,updated_at,session_version) ON public.users TO %I',identity_runtime);
            EXECUTE pg_catalog.format('GRANT SELECT(id,username,real_name,email,phone,avatar,gender,status,is_super_admin,last_login_at,created_at,updated_at,session_version), UPDATE(is_super_admin) ON public.users TO %I,%I',platform_runtime,platform_owner);
        ELSIF item.relname IN ('identity_service_clients','identity_account_revocation_facts') THEN
            EXECUTE pg_catalog.format('GRANT SELECT ON public.%I TO %I',item.relname,identity_runtime);
        ELSIF item.relname IN ('identity_audit_logs','identity_account_command_receipts','identity_account_tombstones','identity_oidc_admin_receipts') THEN
            EXECUTE pg_catalog.format('GRANT SELECT,INSERT ON public.%I TO %I',item.relname,identity_runtime);
            IF item.relname='identity_account_tombstones' THEN
                EXECUTE pg_catalog.format('GRANT SELECT ON public.%I TO %I,%I',item.relname,platform_runtime,platform_owner);
            END IF;
        ELSIF item.relname='platform_oidc_command_intents' THEN
            EXECUTE pg_catalog.format('GRANT SELECT,INSERT ON public.%I TO %I',item.relname,platform_runtime);
        ELSIF item.relname=ANY(auth_tables) THEN
            EXECUTE pg_catalog.format('GRANT SELECT,INSERT,UPDATE,DELETE ON public.%I TO %I',item.relname,identity_runtime);
        ELSE
            EXECUTE pg_catalog.format('GRANT SELECT,INSERT,UPDATE,DELETE ON public.%I TO %I',item.relname,platform_runtime);
        END IF;
    END LOOP;
    FOR item IN SELECT c.relname FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='public' AND c.relkind='S' LOOP
        EXECUTE pg_catalog.format('ALTER SEQUENCE public.%I OWNER TO %I',item.relname,platform_owner);
        EXECUTE pg_catalog.format('GRANT USAGE,SELECT ON SEQUENCE public.%I TO %I',item.relname,platform_runtime);
    END LOOP;

    auth_functions:=ARRAY['bump_session_version_on_user_disable','revoke_sso_on_account_generation',
        'revoke_oidc_on_client_policy','revoke_oidc_on_sso_revocation','oidc_key_material_immutable',
        'bff_oidc_proof_active','customer_portal_account_source_active','oidc_console_source_active','identity_lock_oidc_source',
        'oidc_context_capability_generation','oidc_context_binding_immutable'];
    bridge_functions:=ARRAY['revoke_sessions_on_user_disable','revoke_credentials_on_account_generation'];
    fence_functions:=ARRAY['platform_password_context_active','platform_account_command_intent_active','identity_oidc_command_intent_active'];
    FOR item IN SELECT p.oid,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid) AS args
        FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' LOOP
        IF EXISTS(SELECT 1 FROM pg_catalog.pg_depend d WHERE d.classid='pg_catalog.pg_proc'::regclass
            AND d.objid=item.oid AND d.refclassid='pg_catalog.pg_extension'::regclass AND d.deptype='e') THEN
            -- Extension members keep their extension owner's lifecycle. Their
            -- safe platform search/index operations are not auth capabilities.
            EXECUTE pg_catalog.format('GRANT EXECUTE ON FUNCTION public.%I(%s) TO %I,%I',item.proname,item.args,platform_runtime,platform_owner);
            CONTINUE;
        END IF;
        target_owner:=CASE WHEN item.proname=ANY(auth_functions) THEN identity_owner
            WHEN item.proname=ANY(bridge_functions) THEN bridge_owner ELSE platform_owner END;
        IF item.proname='is_uuid_v7' THEN target_owner:=migration_owner; END IF;
        EXECUTE pg_catalog.format('ALTER FUNCTION public.%I(%s) OWNER TO %I',item.proname,item.args,target_owner);
        EXECUTE pg_catalog.format('ALTER FUNCTION public.%I(%s) SET search_path TO pg_catalog,public',item.proname,item.args);
        EXECUTE pg_catalog.format('REVOKE ALL ON FUNCTION public.%I(%s) FROM PUBLIC,%I,%I',item.proname,item.args,identity_runtime,platform_runtime);
        IF item.proname='is_uuid_v7' THEN
            EXECUTE pg_catalog.format('GRANT EXECUTE ON FUNCTION public.%I(%s) TO %I,%I,%I,%I,%I',item.proname,item.args,identity_owner,platform_owner,bridge_owner,identity_runtime,platform_runtime);
        ELSIF item.proname=ANY(fence_functions) THEN
            EXECUTE pg_catalog.format('GRANT EXECUTE ON FUNCTION public.%I(%s) TO %I',item.proname,item.args,identity_runtime);
        ELSIF item.proname=ANY(bridge_functions) THEN
            NULL; -- Trigger invocation, never a callable arbitrary-target API.
        ELSIF item.proname=ANY(auth_functions) THEN
            IF item.proname<>'identity_lock_oidc_source' THEN
                EXECUTE pg_catalog.format('GRANT EXECUTE ON FUNCTION public.%I(%s) TO %I',item.proname,item.args,identity_runtime);
            END IF;
            IF item.proname IN ('bff_oidc_proof_active','customer_portal_account_source_active','oidc_console_source_active','identity_lock_oidc_source') THEN
                EXECUTE pg_catalog.format('GRANT EXECUTE ON FUNCTION public.%I(%s) TO %I,%I',item.proname,item.args,platform_runtime,platform_owner);
            END IF;
        ELSE
            EXECUTE pg_catalog.format('GRANT EXECUTE ON FUNCTION public.%I(%s) TO %I',item.proname,item.args,platform_runtime);
        END IF;
    END LOOP;

    -- The NOLOGIN invalidation bridge has no business ownership or DML beyond
    -- these exact invalidation columns. All reads are required by its predicates.
    EXECUTE pg_catalog.format('GRANT SELECT(id,user_id,revoked_at),UPDATE(revoked_at) ON public.platform_sessions TO %I',bridge_owner);
    EXECUTE pg_catalog.format('GRANT SELECT(session_id,consumed_at),UPDATE(consumed_at) ON public.platform_session_refresh_tokens TO %I',bridge_owner);
    EXECUTE pg_catalog.format('GRANT SELECT(user_id,revoked),UPDATE(revoked) ON public.oauth_tokens TO %I',bridge_owner);
    EXECUTE pg_catalog.format('GRANT SELECT(user_id,consumed),UPDATE(consumed) ON public.oauth_authorization_codes TO %I',bridge_owner);
    EXECUTE pg_catalog.format('GRANT SELECT(user_id,status),UPDATE(status) ON public.oauth_device_codes TO %I',bridge_owner);
    EXECUTE pg_catalog.format('GRANT SELECT(user_id,account_version,platform_session_count,oauth_token_count),INSERT(user_id,account_version,platform_session_count,oauth_token_count,created_at),UPDATE(platform_session_count,oauth_token_count) ON public.identity_account_revocation_facts TO %I',bridge_owner);
    FOREACH role_name IN ARRAY ARRAY[identity_owner,platform_owner,bridge_owner,identity_runtime,platform_runtime] LOOP
        EXECUTE pg_catalog.format('REVOKE CREATE ON SCHEMA public FROM %I',role_name);
        EXECUTE pg_catalog.format('REVOKE CREATE,TEMPORARY ON DATABASE %I FROM %I',cfg.expected_database,role_name);
    END LOOP;

    -- Final readiness is written only after catalog privilege assertions.
    IF pg_catalog.has_column_privilege(platform_runtime,'public.users','password_hash','SELECT')
       OR pg_catalog.has_column_privilege(platform_runtime,'public.users','password_hash','UPDATE')
       OR pg_catalog.has_column_privilege(platform_runtime,'public.users','session_version','UPDATE')
       OR pg_catalog.has_column_privilege(identity_runtime,'public.users','is_super_admin','SELECT')
       OR pg_catalog.has_column_privilege(identity_runtime,'public.users','is_super_admin','UPDATE')
       OR pg_catalog.has_table_privilege(identity_runtime,'public.memberships','SELECT')
       OR pg_catalog.has_table_privilege(platform_runtime,'public.oidc_clients','SELECT')
       OR pg_catalog.has_table_privilege(identity_runtime,'public.users','DELETE')
       OR pg_catalog.has_table_privilege(identity_runtime,'public.users','TRUNCATE')
       OR pg_catalog.has_column_privilege(identity_runtime,'public.users','id','UPDATE') THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_COLUMN_OR_OWNER_ISOLATION_FAILED';
    END IF;
    FOREACH role_name IN ARRAY ARRAY[identity_runtime,platform_runtime] LOOP
        IF EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members m JOIN pg_catalog.pg_roles r ON r.oid=m.member WHERE r.rolname=role_name)
           OR pg_catalog.has_database_privilege(role_name,cfg.expected_database,'CREATE')
           OR pg_catalog.has_database_privilege(role_name,cfg.expected_database,'TEMPORARY')
           OR pg_catalog.has_schema_privilege(role_name,'public','CREATE')
           OR EXISTS(SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_roles r ON r.oid=c.relowner WHERE r.rolname=role_name)
           OR EXISTS(SELECT 1 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_roles r ON r.oid=p.proowner WHERE r.rolname=role_name) THEN
            RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_RUNTIME_DDL_OR_OWNERSHIP_INVALID';
        END IF;
    END LOOP;
    IF EXISTS(SELECT 1 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
         CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) a
         WHERE n.nspname='public' AND a.grantee=0 AND a.privilege_type='EXECUTE')
       OR EXISTS(SELECT 1 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
         WHERE n.nspname='public' AND p.prosecdef AND
           (p.proconfig IS NULL OR NOT ('search_path=pg_catalog, public'=ANY(p.proconfig)))) THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_PUBLIC_EXECUTE_OR_DEFINER_PATH_INVALID';
    END IF;
    INSERT INTO public.identity_deployment_state(singleton,deployment_id,issuer,schema_version,migration_owner,identity_owner,platform_owner,
        revocation_bridge_owner,projection_owner,identity_runtime,platform_runtime,ready_at)
    VALUES(true,cfg.deployment_id,cfg.issuer,1,migration_owner,identity_owner,platform_owner,bridge_owner,identity_owner,identity_runtime,platform_runtime,pg_catalog.clock_timestamp());
END $provision$;
COMMIT;
