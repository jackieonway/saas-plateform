-- The first-party report module is bundled with the host. Keep historical
-- migrations immutable and restore only the exact built-in HTML demo entry.
UPDATE applications
SET entry_type = 'ESM',
    entry_url = '/src/micro-apps/report-center.tsx',
    runtime_mode = 'TRUSTED',
    description = '第一方 ESM 报表中心；随宿主构建并直挂',
    updated_at = now()
WHERE code = 'report-center'
  AND entry_type = 'HTML'
  AND entry_url = '/sub-apps/report-center/index.html'
  AND runtime_mode = 'IFRAME'
  AND base_path = '/micro/report-center';

UPDATE menus
SET title = '共享运行时'
WHERE application_id = (SELECT id FROM applications WHERE code = 'report-center')
  AND path = '/micro/report-center/shared'
  AND title = '隔离通信';
