/** Reproducible production build and module/gzip inventory, outside tracked dist. */
import { createRequire } from 'node:module'
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs'
import { gzipSync } from 'node:zlib'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
const root = process.cwd()
const label = process.argv[2]
if (!['before', 'after'].includes(label) && !/^gate-(chromium|chrome|msedge|firefox|webkit)$/.test(label)) throw new Error('Expected before, after or gate-browser')
const out = path.join(root, 'target/phase-29', label)
if (label === 'before' && existsSync(path.join(out, 'build.json'))) throw new Error('Baseline already exists; preserve the original evidence')
mkdirSync(out, { recursive: true })
const require = createRequire(path.join(root, 'frontend/package.json'))
const { build } = await import(pathToFileURL(require.resolve('vite')).href)
const inventory = []
await build({ root: path.join(root, 'frontend'), configFile: path.join(root, 'frontend/vite.config.ts'),
  build: { outDir: path.join(out, 'dist'), emptyOutDir: true, manifest: true },
  plugins: [{ name: 'phase29-module-evidence', writeBundle(_, bundle) {
    for (const [file, item] of Object.entries(bundle)) {
      // Vite may still rewrite preload imports after generateBundle; hash disk bytes.
      const bytes = readFileSync(path.join(out, 'dist', file))
      inventory.push({ file, bytes: bytes.length, gzip: gzipSync(bytes).length,
        sha256: createHash('sha256').update(bytes).digest('hex'),
        ...(item.type === 'chunk' ? { isEntry: item.isEntry, imports: item.imports,
          dynamicImports: item.dynamicImports, modules: Object.keys(item.modules).map(p => p.replaceAll('\\', '/').replace(root.replaceAll('\\', '/') + '/', '')) } : {}) })
    }
  } }] })
writeFileSync(path.join(out, 'build.json'), JSON.stringify({ node: process.version, vite: require('vite/package.json').version,
  sourceAppSha256: createHash('sha256').update(readFileSync('frontend/src/App.tsx')).digest('hex'), inventory }, null, 2) + '\n')
