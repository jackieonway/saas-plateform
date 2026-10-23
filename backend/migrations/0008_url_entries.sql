-- A mutable environment URL is an explicit override of the immutable release
-- pointer. Removing it (or activating/rolling back a release) restores releases.
CREATE TABLE application_url_entries (
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    environment VARCHAR(32) NOT NULL CHECK (environment ~ '^[a-z0-9][a-z0-9_-]{0,31}$'),
    entry_url TEXT NOT NULL CHECK (length(entry_url) BETWEEN 1 AND 2000),
    proxy_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    revision UUID NOT NULL,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (application_id, environment)
);

-- Both profile discovery and the authorization MVCC snapshot use this exact
-- resolver. The revision is an entry identity, not an immutable release record.
CREATE FUNCTION application_effective_entry(app_id UUID, env TEXT)
RETURNS TABLE(runtime_mode TEXT, entry_type TEXT, entry_url TEXT, version TEXT,
              id UUID, source TEXT, proxy_enabled BOOLEAN)
LANGUAGE SQL STABLE AS $$
    SELECT a.runtime_mode::TEXT,
      CASE WHEN u.application_id IS NOT NULL THEN 'HTML' ELSE COALESCE(r.entry_type,a.entry_type)::TEXT END,
      CASE WHEN u.application_id IS NOT NULL AND u.proxy_enabled THEN
        '/micro-proxy/' || a.code || '/' || u.revision::TEXT || '/' ||
        CASE WHEN strpos(u.entry_url,'?') > 0 THEN '?' || substring(u.entry_url from strpos(u.entry_url,'?') + 1) ELSE '' END
      WHEN u.application_id IS NOT NULL THEN u.entry_url ELSE COALESCE(r.entry_url,a.entry_url,'')::TEXT END,
      CASE WHEN u.application_id IS NULL THEN r.version::TEXT END,
      CASE WHEN u.application_id IS NOT NULL THEN u.revision ELSE r.id END,
      CASE WHEN u.application_id IS NOT NULL THEN 'URL' WHEN r.id IS NOT NULL THEN 'RELEASE' ELSE 'LEGACY' END,
      COALESCE(u.proxy_enabled,FALSE)
    FROM applications a
      LEFT JOIN application_url_entries u ON u.application_id=a.id AND u.environment=env
        AND a.entry_type='HTML' AND a.runtime_mode IN ('TRUSTED','IFRAME')
        AND rtrim(a.base_path,'/')='/micro/' || a.code
      LEFT JOIN application_deployments d ON d.application_id=a.id AND d.environment=env
      LEFT JOIN application_releases r ON r.application_id=a.id AND r.environment=d.environment AND r.id=d.active_release_id
    WHERE a.id=app_id
$$;
