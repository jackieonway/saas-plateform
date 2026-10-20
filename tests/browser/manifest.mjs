// Existing real Vite/React browser tests. HTTP/Socket.IO fixtures isolate the UI;
// backend authorization, databases and protocol migration have independent gates.
export const mandatorySuites = [
  { id: 'phase29-host', script: 'tests/phase29-browser-gate.mjs', expectedCheckGroups: 9 },
  { id: 'micro-host', script: 'tests/micro-host-browser.mjs', expectedCheckGroups: 47 },
  { id: 'notifications', script: 'tests/notification-frontend-browser.mjs', expectedCheckGroups: 10 },
  { id: 'platform-settings', script: 'tests/platform-settings-browser.mjs', expectedCheckGroups: 11 },
  { id: 'regions', script: 'tests/region-location-browser.mjs', expectedCheckGroups: 12 },
  { id: 'entities', script: 'tests/entity-types-browser.mjs', expectedCheckGroups: 13 },
  { id: 'route-permissions', script: 'tests/route-permissions-browser.mjs', expectedCheckGroups: 64 },
  { id: 'phase9-navigation', script: 'tests/phase9-navigation-browser.mjs', expectedCheckGroups: 14 },
  { id: 'phase10-session', script: 'tests/phase10-session-browser.mjs', expectedCheckGroups: 17 },
  { id: 'departments', script: 'tests/department-context-browser.mjs', expectedCheckGroups: 11 },
  { id: 'chat', script: 'examples/websocket/tests/browser-smoke.mjs', cwd: 'examples/websocket', expectedCheckGroups: 1 },
];
