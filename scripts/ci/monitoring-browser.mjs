/** Reuse the owned HTTPS/NSS/three-Rust-service harness, select real Phase32 measurements. */
import { runStoresBrowser, parseStoresBrowserArgs } from './stores-browser.mjs'
const result = await runStoresBrowser({ ...parseStoresBrowserArgs(process.argv.slice(2)), monitoring: true })
console.log(`MONITORING_BROWSER ${result.product} ${result.status}`)
if (result.status !== 'PASS') process.exitCode = 1
