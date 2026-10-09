-- The original first-party report-center seed is an ESM module loaded into the
-- host runtime. Migration 0021 introduced an IFRAME default for all existing
-- applications, which made this exact seed impossible to mount.
-- Keep administrator-customized entries and other ESM applications untouched.
UPDATE applications
SET runtime_mode = 'TRUSTED', updated_at = now()
WHERE code = 'report-center'
  AND entry_type = 'ESM'
  AND entry_url = '/src/micro-apps/report-center.tsx'
  AND base_path = '/micro/report-center'
  AND runtime_mode = 'IFRAME';
