/** Always rebuild the tested bytes; a prior phase artifact cannot satisfy CI. */
const { browserName } = await import('./browser/runtime.mjs')
process.argv[2] = `gate-${browserName}`
await import('./phase29-build.mjs')
await import('./phase29-host-browser.mjs')
