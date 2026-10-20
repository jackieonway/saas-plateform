import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { readFile, mkdtemp, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { renderEdge } from '../deploy/render-edge.mjs'

const directory = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const valid = { publicOrigin: 'https://oauth.example.test', frontend: 'oauth-web:8080', api: 'oauth-api:3011' }

test('edge keeps private API and callback on the same HTTPS origin and resolves recreated services', () => {
  const config = renderEdge(valid)
  assert.match(config, /return 308 https:\/\/oauth\.example\.test\$request_uri;/)
  assert.match(config, /listen 8443 ssl;/)
  assert.match(config, /resolver 127\.0\.0\.11 valid=5s/)
  assert.match(config, /set \$oauth_api oauth-api:3011;/)
  assert.match(config, /location \^~ \/api\/ \{[\s\S]*proxy_pass http:\/\/\$oauth_api\$request_uri;/)
  assert.match(config, /location \/ \{ proxy_pass http:\/\/\$oauth_frontend\$request_uri;/)
  assert.match(config, /access_log off;/)
  assert.match(config, /proxy_hide_header Referrer-Policy;/)
  assert.match(config, /location \^~ \/api\/ \{\s*proxy_hide_header Cache-Control;/)
  assert.doesNotMatch(config, /\$request\b|log_format|ssl_protocols[^;]*TLSv1(?:\s|;)/)
  assert.match(renderEdge({ ...valid, publicOrigin: 'https://oauth.example.test:9443' }), /return 308 https:\/\/oauth\.example\.test:9443\$request_uri;/)
})

test('edge refuses non-HTTPS origins, credentials, redirects, config injection and public upstream URLs', () => {
  for (const publicOrigin of ['http://oauth.example.test', 'https://user:private@oauth.example.test', 'https://oauth.example.test/callback', 'https://oauth.example.test?token=private', 'https://oauth.example.test#private', 'https://oauth.example.test/', 'https://[::1]']) {
    assert.throws(() => renderEdge({ ...valid, publicOrigin }), /publicOrigin/)
  }
  for (const upstream of ['https://external.example.test:443', 'user:private@oauth-api:3011', 'oauth-api:65536', 'oauth-api:0', 'oauth-api:3011;\n access_log /tmp/leak;']) {
    assert.throws(() => renderEdge({ ...valid, api: upstream }), /private Compose/)
  }
  assert.throws(() => renderEdge({ ...valid, secret: 'private' }), /Unknown edge/)
})

test('renderer never overwrites reviewed config and does not echo accidental credentials', async (t) => {
  const temporaryRoot = await realpath(tmpdir())
  const temporary = await mkdtemp(path.join(temporaryRoot, 'oauth-edge-render-'))
  t.after(async () => {
    const resolved = await realpath(temporary)
    assert.equal(path.dirname(resolved), temporaryRoot)
    assert.ok(path.basename(resolved).startsWith('oauth-edge-render-'))
    await rm(resolved, { recursive: true })
  })
  const source = path.join(temporary, 'edge.json')
  const output = path.join(temporary, 'nginx.conf')
  const renderer = path.join(directory, 'deploy/render-edge.mjs')
  await writeFile(source, JSON.stringify(valid))
  const first = spawnSync(process.execPath, [renderer, source, output], { encoding: 'utf8', windowsHide: true })
  assert.equal(first.status, 0, first.stderr)
  assert.equal(await readFile(output, 'utf8'), renderEdge(valid))
  const second = spawnSync(process.execPath, [renderer, source, output], { encoding: 'utf8', windowsHide: true })
  assert.equal(second.status, 1)
  assert.equal(await readFile(output, 'utf8'), renderEdge(valid))
  await writeFile(source, JSON.stringify({ ...valid, publicOrigin: 'https://user:private-fixture-secret@oauth.example.test' }))
  const invalid = spawnSync(process.execPath, [renderer, source, output], { encoding: 'utf8', windowsHide: true })
  assert.equal(invalid.status, 1)
  assert.doesNotMatch(`${invalid.stdout}${invalid.stderr}`, /user|private-fixture-secret/)
})

test('static delivery serves real callback HTML, returns missing assets as 404 and has no API fallback', async () => {
  const config = await readFile(path.join(directory, 'deploy/static.nginx.conf'), 'utf8')
  assert.match(config, /location = \/callback \{[^\n]*try_files \/callback\.html =404;/)
  assert.match(config, /location \^~ \/assets\/ \{[^\n]*try_files \$uri =404;/)
  assert.match(config, /location \/ \{ return 404; \}/)
  assert.match(config, /pid \/tmp\/nginx.pid;/)
  assert.match(config, /access_log off;/)
  assert.doesNotMatch(config, /proxy_pass|try_files[^;]*\/index.html[^;]*\$|unsafe-inline|unsafe-eval/)
})

test('separate runtime images and Compose never copy private config into frontend or publish the API', async () => {
  const [compose, frontend, api, ignore, packageJson] = await Promise.all([
    'deploy/compose.yml', 'deploy/Dockerfile.frontend', 'deploy/Dockerfile.api', '.dockerignore', 'package.json',
  ].map((file) => readFile(path.join(directory, file), 'utf8')))
  const apiService = compose.split('  oauth-api:')[1].split('  oauth-web:')[0]
  const webService = compose.split('  oauth-web:')[1].split('  oauth-edge:')[0]
  assert.match(apiService, /env_file:/)
  assert.doesNotMatch(apiService, /\n\s+ports:/)
  assert.doesNotMatch(webService, /env_file:|environment:|\n\s+ports:/)
  assert.equal((compose.match(/\n\s+ports:/g) || []).length, 1)
  assert.match(frontend, /USER 101:101/)
  assert.match(frontend, /COPY --from=build[^\n]*\/app\/dist\//)
  assert.doesNotMatch(frontend.split('AS runtime')[1], /node|npm|COPY frontend|COPY backend/)
  assert.match(api, /USER 10001:10001/)
  assert.match(api, /cargo build --release --locked/)
  assert.match(api, /ENTRYPOINT \["oauth-playground-api"\]/)
  for (const dockerfile of [api, frontend]) {
    assert.match(dockerfile, /ARG SOURCE_REVISION/)
    assert.match(dockerfile, /org\.opencontainers\.image\.revision/)
    assert.doesNotMatch(dockerfile, /:latest\b|COPY \. |OAUTH_[A-Z_]*(?:SECRET|PASSWORD)/)
  }
  assert.match(ignore, /\*\*\/\.env\*/)
  assert.match(ignore, /\*\*\/\*\.env/)
  for (const script of Object.values(JSON.parse(packageJson).scripts)) assert.doesNotMatch(script, /node[^\n]*index\.ts|pages\.ts/)
})
