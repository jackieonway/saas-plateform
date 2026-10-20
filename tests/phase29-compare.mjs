/** Freeze raw comparable samples and module evidence; fail rather than invent gains. */
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
const read = file => JSON.parse(readFileSync(file))
const budget = read('docs/validation/phase-29-budget.json')
const before = read('target/phase-29/before/performance.json')
const after = read('target/phase-29/after/performance-final.json')
const builds = Object.fromEntries(['before', 'after'].map(label => [label, read(`target/phase-29/${label}/build.json`)]))
assert.deepEqual(before.method, after.method); assert.equal(before.browser, after.browser)
const median = values => [...values].sort((a,b) => a-b)[Math.floor(values.length / 2)]
const summarize = (samples, build) => {
  const loaded = new Set(samples[0].resources.map(r => new URL(r.name).pathname.slice(1)))
  const js = build.inventory.filter(item => loaded.has(item.file) && item.file.endsWith('.js'))
  return { transfer: samples.map(s => s.initialTransferBytes), encoded: samples.map(s => s.initialEncodedBodyBytes),
    lcp: samples.map(s => s.lcpMs), lcpMedian: median(samples.map(s => s.lcpMs)),
    interaction: samples.map(s => s.labInteractionMs),
    initialJsGzip: js.reduce((sum, chunk) => sum + chunk.gzip, 0),
    initialJsChunks: js.map(({ file, bytes, gzip, modules }) => ({ file, bytes, gzip, modules })),
    allJsGzip: build.inventory.filter(item => item.file.endsWith('.js')).reduce((sum,item) => sum+item.gzip,0) }
}
const summary = { before: summarize(before.samples, builds.before), after: summarize(after.samples, builds.after) }
assert(after.samples.every(s => s.initialTransferBytes <= budget.limits.initialTransferBytesMax))
assert(after.samples.every(s => s.initialEncodedBodyBytes <= budget.limits.initialEncodedBodyBytesMax))
assert(summary.after.lcpMedian <= budget.limits.lcpMedianMsMax)
assert(after.samples.every(s => s.labInteractionMs === null || s.labInteractionMs <= budget.limits.labInteractionMaxMsMax))
const forbidden = /components\/layout\/(AppLayout|CommandPalette|AppRail|AppSidebar|TopBar)|use-notification-socket|node_modules\/socket.io-client/
assert(!summary.after.initialJsChunks.some(chunk => chunk.modules.some(id => forbidden.test(id))))
const reductions = { transferPercent: 100 * (1 - median(summary.after.transfer) / median(summary.before.transfer)),
  encodedPercent: 100 * (1 - median(summary.after.encoded) / median(summary.before.encoded)),
  initialJsGzipPercent: 100 * (1 - summary.after.initialJsGzip / summary.before.initialJsGzip),
  lcpMedianPercent: 100 * (1 - summary.after.lcpMedian / summary.before.lcpMedian) }
writeFileSync('docs/validation/phase-29-performance.json', JSON.stringify({ budget, reductions, summary, raw: { before, after } }, null, 2) + '\n')
console.log(JSON.stringify({ reductions, before: { ...summary.before, initialJsChunks: undefined }, after: { ...summary.after, initialJsChunks: undefined } }, null, 2))
