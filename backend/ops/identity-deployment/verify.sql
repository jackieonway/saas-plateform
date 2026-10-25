\set ON_ERROR_STOP on
\set ECHO none
\set QUIET on
-- Read-only catalog check. Variables match provision.sql. No passwords or token
-- material are read or returned; a single safe receipt is emitted on success.
BEGIN READ ONLY;
SELECT pg_catalog.set_config('identity_deployment.verify_database',:'expected_database',true) AS verified_database,
    pg_catalog.set_config('identity_deployment.verify_id',:'deployment_id',true) AS verified_id,
    pg_catalog.set_config('identity_deployment.verify_issuer',:'issuer',true) AS verified_issuer,
    pg_catalog.set_config('identity_deployment.verify_prefix',:'role_prefix',true) AS verified_prefix
\gset
DO $verify$
DECLARE cfg record; s public.identity_deployment_state%ROWTYPE; r record; owner_ref name;
BEGIN
    SELECT current_setting('identity_deployment.verify_database') AS expected_database,
        current_setting('identity_deployment.verify_id')::uuid AS deployment_id,
        current_setting('identity_deployment.verify_issuer') AS issuer,
        current_setting('identity_deployment.verify_prefix') AS prefix INTO cfg;
    SELECT * INTO s FROM public.identity_deployment_state WHERE singleton;
    IF NOT FOUND OR current_database()<>cfg.expected_database OR s.deployment_id<>cfg.deployment_id
       OR cfg.deployment_id::text<>current_setting('identity_deployment.verify_id')
       OR s.issuer<>cfg.issuer OR s.schema_version<>20
       OR s.migration_owner<>cfg.prefix||'_migration_owner'
       OR s.identity_owner<>cfg.prefix||'_identity_owner'
       OR s.platform_owner<>cfg.prefix||'_platform_owner'
       OR s.revocation_bridge_owner<>cfg.prefix||'_bridge_owner'
       OR s.projection_owner<>s.identity_owner
       OR s.identity_runtime<>cfg.prefix||'_identity_runtime'
       OR s.platform_runtime<>cfg.prefix||'_platform_runtime' THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_MARKER_INVALID';
    END IF;
    IF EXISTS(SELECT 1 FROM pg_roles WHERE rolname IN(s.migration_owner,s.identity_owner,s.platform_owner,s.revocation_bridge_owner)
         AND (rolcanlogin OR rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls OR rolinherit))
       OR (SELECT count(*) FROM pg_roles WHERE rolname IN(s.migration_owner,s.identity_owner,s.platform_owner,s.revocation_bridge_owner)
         AND NOT rolcanlogin)<>4 THEN RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_OWNER_ROLE_INVALID'; END IF;
    FOR r IN SELECT oid,rolname,rolcanlogin,rolsuper,rolcreatedb,rolcreaterole,rolreplication,rolbypassrls,rolinherit
        FROM pg_roles WHERE rolname IN(s.identity_runtime,s.platform_runtime) LOOP
        -- NOINHERIT suppresses current inherited privileges, but membership
        -- can still permit SET ROLE to an unrelated permission-bearing role.
        IF EXISTS(SELECT 1 FROM pg_auth_members WHERE member=r.oid) THEN
            RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_RUNTIME_MEMBERSHIP_INVALID';
        END IF;
        IF NOT r.rolcanlogin OR r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls OR r.rolinherit
           OR EXISTS(SELECT 1 FROM pg_auth_members WHERE roleid=r.oid)
           OR has_database_privilege(r.oid,current_database(),'CREATE')
           OR has_database_privilege(r.oid,current_database(),'TEMPORARY')
           OR EXISTS(SELECT 1 FROM pg_namespace n WHERE left(n.nspname,3)<>'pg_' AND has_schema_privilege(r.oid,n.oid,'CREATE')) THEN
            RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_RUNTIME_ROLE_INVALID';
        END IF;
        FOREACH owner_ref IN ARRAY ARRAY[s.migration_owner,s.identity_owner,s.platform_owner,s.revocation_bridge_owner] LOOP
            IF pg_has_role(r.oid,owner_ref,'MEMBER') THEN RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_OWNER_MEMBERSHIP_INVALID'; END IF;
        END LOOP;
    END LOOP;
    IF (SELECT count(*) FROM pg_roles WHERE rolname IN(s.identity_runtime,s.platform_runtime))<>2 THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_RUNTIME_MISSING';
    END IF;
    IF has_column_privilege(s.platform_runtime,'public.users','password_hash','SELECT')
       OR has_column_privilege(s.platform_runtime,'public.users','password_hash','UPDATE')
       OR has_column_privilege(s.platform_runtime,'public.users','session_version','UPDATE')
       OR has_column_privilege(s.identity_runtime,'public.users','is_super_admin','SELECT')
       OR has_column_privilege(s.identity_runtime,'public.users','is_super_admin','UPDATE')
       OR has_column_privilege(s.identity_runtime,'public.users','id','UPDATE')
       OR has_table_privilege(s.identity_runtime,'public.users','DELETE')
       OR has_table_privilege(s.identity_runtime,'public.users','TRUNCATE')
       OR has_table_privilege(s.identity_runtime,'public.memberships','SELECT')
       OR has_table_privilege(s.identity_runtime,'public.roles','SELECT')
       OR has_table_privilege(s.platform_runtime,'public.oidc_clients','SELECT')
       OR has_table_privilege(s.platform_runtime,'public.oidc_access_tokens','SELECT') THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_PRIVILEGE_BOUNDARY_INVALID';
    END IF;
    IF EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
        CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a
        WHERE n.nspname='public' AND a.grantee=0 AND a.privilege_type='EXECUTE') THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_PUBLIC_FUNCTION_EXECUTE';
    END IF;
    IF EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
        WHERE n.nspname='public' AND p.prosecdef AND
          (p.proconfig IS NULL OR NOT ('search_path=pg_catalog, public'=ANY(p.proconfig)))) THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_DEFINER_SEARCH_PATH_INVALID';
    END IF;
    IF EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
        JOIN pg_roles owner_role ON owner_role.oid=c.relowner
        CROSS JOIN unnest(ARRAY['SELECT','INSERT','UPDATE','DELETE']) AS owner_permission(privilege_name)
        WHERE n.nspname='public' AND c.relkind IN ('r','p')
          AND owner_role.rolname IN(s.migration_owner,s.identity_owner,s.platform_owner)
          AND NOT has_table_privilege(owner_role.oid,c.oid,owner_permission.privilege_name)) THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_OWNER_ORDINARY_PRIVILEGE_INVALID';
    END IF;
    IF (SELECT relowner FROM pg_class WHERE oid='public.users'::regclass)<>(SELECT oid FROM pg_roles WHERE rolname=s.identity_owner)
       OR (SELECT relowner FROM pg_class WHERE oid='public.memberships'::regclass)<>(SELECT oid FROM pg_roles WHERE rolname=s.platform_owner)
       OR (SELECT datdba FROM pg_database WHERE datname=current_database())<>(SELECT oid FROM pg_roles WHERE rolname=s.migration_owner)
       OR (SELECT nspowner FROM pg_namespace WHERE nspname='public')<>(SELECT oid FROM pg_roles WHERE rolname=s.migration_owner) THEN
        RAISE EXCEPTION 'IDENTITY_DEPLOYMENT_OBJECT_OWNER_INVALID';
    END IF;
END $verify$;
SELECT 'identity-deployment/1 verified' AS receipt;
COMMIT;
