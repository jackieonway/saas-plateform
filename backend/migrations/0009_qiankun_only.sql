-- New independent micro applications use qiankun. Historical enum values and
-- release rows remain available for inventory; migration never grants trust.
ALTER TABLE applications ALTER COLUMN entry_type SET DEFAULT 'HTML';
ALTER TABLE applications ALTER COLUMN runtime_mode SET DEFAULT 'TRUSTED';

-- Disable only incompatible /micro/{code} apps. Keep business configuration,
-- menus, grants, source fields and release history for explicit reconfiguration.
-- Built-in /system, /hermes and other platform routes are unaffected.
UPDATE applications a SET status='DISABLED',updated_at=now()
WHERE rtrim(a.base_path,'/')='/micro/' || a.code
  AND (a.entry_type<>'HTML' OR a.runtime_mode<>'TRUSTED' OR EXISTS (
      SELECT 1 FROM application_deployments d
        JOIN application_releases r ON r.application_id=a.id
          AND r.environment=d.environment AND r.id=d.active_release_id
      WHERE d.application_id=a.id AND r.entry_type<>'HTML'
  ));

-- The active schema admits no legacy micro loader. Existing release rows are
-- read-only archives: NOT VALID retains them while enforcing HTML on new writes.
ALTER TABLE applications DROP CONSTRAINT applications_runtime_mode_check;
ALTER TABLE applications ADD CONSTRAINT applications_runtime_mode_check
  CHECK (rtrim(base_path,'/') <> '/micro/' || code OR status <> 'ENABLED'
    OR (entry_type='HTML' AND runtime_mode='TRUSTED'));
ALTER TABLE application_releases DROP CONSTRAINT application_releases_entry_type_check;
ALTER TABLE application_releases ADD CONSTRAINT application_releases_entry_type_check
  CHECK (entry_type='HTML') NOT VALID;

CREATE OR REPLACE FUNCTION application_effective_entry(app_id UUID, env TEXT)
RETURNS TABLE(runtime_mode TEXT, entry_type TEXT, entry_url TEXT, version TEXT,
              id UUID, source TEXT, proxy_enabled BOOLEAN)
LANGUAGE SQL STABLE AS $$
    WITH effective AS (
      SELECT a.code,a.base_path,a.runtime_mode::TEXT,
        CASE WHEN u.application_id IS NOT NULL THEN 'HTML' ELSE COALESCE(r.entry_type,a.entry_type)::TEXT END AS entry_type,
        CASE WHEN u.application_id IS NOT NULL AND u.proxy_enabled THEN
          '/micro-proxy/' || a.code || '/' || u.revision::TEXT || '/' ||
          CASE WHEN strpos(u.entry_url,'?') > 0 THEN '?' || substring(u.entry_url from strpos(u.entry_url,'?') + 1) ELSE '' END
        WHEN u.application_id IS NOT NULL THEN u.entry_url ELSE COALESCE(r.entry_url,a.entry_url,'')::TEXT END AS entry_url,
        CASE WHEN u.application_id IS NULL THEN r.version::TEXT END AS version,
        CASE WHEN u.application_id IS NOT NULL THEN u.revision ELSE r.id END AS id,
        CASE WHEN u.application_id IS NOT NULL THEN 'URL' WHEN r.id IS NOT NULL THEN 'RELEASE' ELSE 'LEGACY' END AS source,
        COALESCE(u.proxy_enabled,FALSE) AS proxy_enabled
      FROM applications a
        LEFT JOIN application_url_entries u ON u.application_id=a.id AND u.environment=env
          AND a.entry_type='HTML' AND a.runtime_mode='TRUSTED'
          AND rtrim(a.base_path,'/')='/micro/' || a.code
        LEFT JOIN application_deployments d ON d.application_id=a.id AND d.environment=env
        LEFT JOIN application_releases r ON r.application_id=a.id AND r.environment=d.environment AND r.id=d.active_release_id
      WHERE a.id=app_id
    )
    SELECT e.runtime_mode,e.entry_type,
      CASE WHEN rtrim(e.base_path,'/')='/micro/' || e.code AND (e.runtime_mode<>'TRUSTED' OR e.entry_type<>'HTML') THEN '' ELSE e.entry_url END,
      e.version,e.id,e.source,e.proxy_enabled
    FROM effective e
$$;
