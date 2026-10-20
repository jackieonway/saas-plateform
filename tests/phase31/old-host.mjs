// Execute the exact frozen Phase30 adapter, replacing only its browser transport.
import assert from 'node:assert/strict'
import { registerHooks } from 'node:module'
import { readFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
const provenance = JSON.parse(readFileSync(new URL('./old-host-provenance.json', import.meta.url)))
assert.equal(createHash('sha256').update(readFileSync(new URL('./applications.phase30.ts', import.meta.url))).digest('hex'), provenance.sha256)
registerHooks({ resolve(specifier, context, next) {
  if (specifier === '@/lib/api') return { url: new URL('./transport.mjs', import.meta.url).href, shortCircuit: true }
  return next(specifier, context)
} })
const { applicationsApi: api } = await import('./applications.phase30.ts')
const id = process.env.PHASE31_APP
const before = await api.releases(id, 'development')
assert(Array.isArray(before) && before.length >= 9)
const row = await api.registerRelease(id, { environment: 'development', version: 'old-adapter', entryUrl: '/micro/old-adapter/index.html', healthUrl: null, resources: [] })
assert.equal(row.version, 'old-adapter')
assert.match(row.id, /^[0-9a-f-]{36}$/)
assert((await api.releases(id, 'development')).some(item => item.id === row.id))
assert(Array.isArray(await api.deployments(id)))
console.log('PHASE31_OLD_HOST PASS')
