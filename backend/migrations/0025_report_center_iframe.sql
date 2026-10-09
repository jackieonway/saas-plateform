-- Report center now ships as a first-party HTML page inside the host build.
-- Only migrate the original ESM demo entry; preserve customized applications.
UPDATE applications
SET entry_type = 'HTML',
    entry_url = '/sub-apps/report-center/index.html',
    runtime_mode = 'IFRAME',
    description = '第一方报表中心；随宿主发布，在 iframe 中隔离运行',
    updated_at = now()
WHERE code = 'report-center'
  AND entry_type = 'ESM'
  AND entry_url = '/src/micro-apps/report-center.tsx'
  AND base_path = '/micro/report-center';

UPDATE menus
SET title = '隔离通信'
WHERE application_id = (SELECT id FROM applications WHERE code = 'report-center')
  AND path = '/micro/report-center/shared'
  AND title = '共享运行时';
