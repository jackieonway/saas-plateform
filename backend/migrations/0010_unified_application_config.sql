-- Built-in platform routes keep their identity, menus and status, but no longer
-- carry legacy loader metadata. Their old release history remains archived.
UPDATE applications SET entry_type='HTML',runtime_mode='TRUSTED',
  entry_url=CASE WHEN entry_type='ESM' THEN '' ELSE entry_url END,updated_at=now()
WHERE rtrim(COALESCE(base_path,''),'/') <> '/micro/' || code
  AND (entry_type<>'HTML' OR runtime_mode<>'TRUSTED');

-- Disabled historical micro apps retain metadata for explicit reconfiguration.
ALTER TABLE applications DROP CONSTRAINT applications_runtime_mode_check;
ALTER TABLE applications ADD CONSTRAINT applications_runtime_mode_check
  CHECK (status <> 'ENABLED' OR (entry_type='HTML' AND runtime_mode='TRUSTED'));

CREATE OR REPLACE FUNCTION application_effective_entry(app_id UUID, env TEXT)
RETURNS TABLE(runtime_mode TEXT, entry_type TEXT, entry_url TEXT, version TEXT,
              id UUID, source TEXT, proxy_enabled BOOLEAN)
LANGUAGE SQL STABLE AS $$
    WITH effective AS (
      SELECT a.code,a.base_path,a.runtime_mode::TEXT,
        CASE WHEN u.application_id IS NOT NULL THEN 'HTML' ELSE COALESCE(r.entry_type,a.entry_type)::TEXT END AS entry_type,
        CASE WHEN rtrim(COALESCE(a.base_path,''),'/') <> '/micro/' || a.code THEN ''
        WHEN u.application_id IS NOT NULL AND u.proxy_enabled THEN
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
          AND rtrim(COALESCE(a.base_path,''),'/')='/micro/' || a.code
        LEFT JOIN application_deployments d ON d.application_id=a.id AND d.environment=env
          AND rtrim(COALESCE(a.base_path,''),'/')='/micro/' || a.code
        LEFT JOIN application_releases r ON r.application_id=a.id AND r.environment=d.environment AND r.id=d.active_release_id
      WHERE a.id=app_id
    )
    SELECT e.runtime_mode,e.entry_type,
      CASE WHEN rtrim(COALESCE(e.base_path,''),'/')='/micro/' || e.code AND (e.runtime_mode<>'TRUSTED' OR e.entry_type<>'HTML') THEN '' ELSE e.entry_url END,
      e.version,e.id,e.source,e.proxy_enabled
    FROM effective e
$$;
