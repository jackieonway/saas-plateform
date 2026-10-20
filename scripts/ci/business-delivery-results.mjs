/** Closed, fail-closed contract for actual business image delivery evidence. */
export const businessBrowserProducts = Object.freeze(['chromium', 'chrome', 'msedge'])
export const businessBrowserChecks = Object.freeze([
  'url-ui-draft-preflight-activate',
  'preflight-http200-failed-rejected',
  'activate-rechecks-unavailable-entry',
  'versioned-chunks-rollback-retained',
  'ordinary-user-sdk-rust-authorization',
  'external-origin-api-cors-csp',
  'artifact-inventory-conflicts',
  'image-deployment-does-not-grant',
  'independent-databases-and-services',
])
export const businessDeliveryChecks = Object.freeze([
  'independent-static-image-builds',
  'isolated-registry-push-pull',
  'fixed-digest-compose-start',
  'unhealthy-blocks-promotion',
  'normal-tls',
  ...businessBrowserChecks,
])
const fail = message => { throw new Error('Business delivery evidence: ' + message) }
const hex = (value, length) => typeof value === 'string' && new RegExp('^[a-f0-9]{' + length + '}$').test(value)
function exactChecks(checks, expected, label) {
  if (!Array.isArray(checks) || checks.length !== expected.length) fail(label + ' checks are incomplete')
  const seen = new Set()
  for (const check of checks) {
    if (!check || !expected.includes(check.id) || seen.has(check.id) || check.status !== 'PASS') fail(label + ' has a missing, duplicate, failed or skipped check')
    seen.add(check.id)
  }
}
export function parseBusinessBrowserOutput(output, { product, sourceRevision, inputsSha256, sourceSha256 } = {}) {
  const lines = String(output).split(/\r?\n/)
  const markers = lines.filter(line => line.startsWith('PHASE28_BROWSER '))
  if (markers.length !== 1) fail('exactly one original browser marker required')
  let result
  try { result = JSON.parse(markers[0].slice('PHASE28_BROWSER '.length)) } catch { fail('invalid original browser JSON') }
  if (result.schemaVersion !== 1 || result.kind !== 'phase28-real-image-delivery-browser' || result.status !== 'PASS' || result.product !== product || result.sourceRevision !== sourceRevision || result.inputsSha256 !== inputsSha256 || sourceSha256 && result.sourceSha256 !== sourceSha256) fail('original browser identity differs')
  exactChecks(result.checks, businessBrowserChecks, product)
  const rawChecks = lines.filter(line => line.startsWith('PASS phase28 browser ')).map(line => ({ id: line.slice('PASS phase28 browser '.length), status: 'PASS' }))
  exactChecks(rawChecks, businessBrowserChecks, product + ' raw')
  if (result.passed !== businessBrowserChecks.length || result.failed !== 0 || result.skipped !== 0 || result.normalTLS !== true || typeof result.browserVersion !== 'string' || !result.browserVersion || !result.evidence || businessBrowserChecks.some(id => result.evidence[id] === undefined)) fail('browser work, evidence, version or TLS is missing')
  if (result.cleanup?.databaseConnectionsClosed !== true || result.cleanup?.browserClosed !== true) fail('browser cleanup not confirmed')
  if (!Array.isArray(result.transport?.businessRequests) || !result.transport.businessRequests.length || !Array.isArray(result.transport?.records) || !result.transport.records.length) fail('actual SDK and HTTP observations required')
  return result
}
export function validateBusinessDeliveryReport(document, { sourceRevision, inputsSha256 } = {}) {
  const report = document
  if (!report || report.schemaVersion !== 1 || report.kind !== 'business-delivery' || report.status !== 'PASS' || report.cancelled === true) fail('successful uncancelled typed report required')
  if (!hex(report.sourceRevision, 40) || !hex(report.inputsSha256, 64) || sourceRevision && report.sourceRevision !== sourceRevision || inputsSha256 && report.inputsSha256 !== inputsSha256) fail('source identity mismatch')
  if (report.targetPlatform !== 'linux/amd64' || report.sourceUnchanged !== true) fail('platform or source mutation')
  exactChecks(report.checks, businessDeliveryChecks, 'delivery')
  if (!Array.isArray(report.browserProducts) || report.browserProducts.length !== businessBrowserProducts.length) fail('all three browser products are required')
  const seen = new Set()
  for (const product of report.browserProducts) {
    if (!businessBrowserProducts.includes(product.product) || seen.has(product.product) || product.status !== 'PASS') fail('failed, duplicate or unexpected browser')
    seen.add(product.product)
    if (product.sourceRevision !== report.sourceRevision || product.inputsSha256 !== report.inputsSha256 || product.normalTLS !== true) fail('browser source or TLS mismatch')
    exactChecks(product.checks, businessBrowserChecks, product.product)
    if (product.cleanup?.databaseConnectionsClosed !== true || product.cleanup?.browserClosed !== true) fail('browser cleanup not confirmed')
  }
  const registry = report.registry
  if (!registry || registry.scope !== 'isolated' || registry.push !== 'PASS' || registry.pull !== 'PASS' || registry.candidateAbsentBeforePull !== true || registry.registryRequestsObserved !== true || registry.productionPublication !== false) fail('actual isolated registry distribution proof missing')
  const imageKeys = ['work-orders-v1', 'work-orders-v2', 'stores-v1', 'stores-v2', 'work-orders-api', 'stores-api']
  if (!Array.isArray(report.images) || report.images.length !== 6 || new Set(report.images.map(image => image.key)).size !== 6 || imageKeys.some(key => !report.images.some(image => image.key === key)) || report.images.some(image => !/^sha256:[a-f0-9]{64}$/.test(image.imageId || '') || !/^.+@sha256:[a-f0-9]{64}$/.test(image.reference || '') || image.sourceRevision !== report.sourceRevision || image.inputsSha256 !== report.inputsSha256 || !image.buildId || image.reference.split('@')[1] !== image.digest)) fail('six immutable source-bound business image identities required')
  if (report.unhealthy?.status !== 'BLOCKED' || ['routes', 'register', 'activate'].some(key => report.unhealthy.calls?.[key] !== 0)) fail('unhealthy promotion was not blocked')
  const cleanup = report.cleanup
  if (!cleanup || cleanup.ownedContainersRemoved !== true || cleanup.ownedNetworksRemoved !== true || cleanup.ownedVolumesRemoved !== true || cleanup.privateWorkRemoved !== true || !Array.isArray(cleanup.errors) || cleanup.errors.length) fail('owned cleanup failed or was omitted')
  return report
}
