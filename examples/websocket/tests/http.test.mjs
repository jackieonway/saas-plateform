import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { build, createServer, preview } from 'vite';

async function checkHttp(url) {
  const response = await fetch(`${url}/`);
  assert.equal(response.status, 200);
  const html = await response.text();
  assert.match(html, /WebSocket Demo/);
  const scriptPaths = [...html.matchAll(/<script[^>]+src="([^"]+)"/g)].map((match) => match[1]);
  assert.ok(scriptPaths.length > 0);
  for (const scriptPath of scriptPaths) {
    const scriptResponse = await fetch(new URL(scriptPath, `${url}/`));
    assert.equal(scriptResponse.status, 200);
    assert.match(scriptResponse.headers.get('content-type'), /javascript/);
    assert.ok((await scriptResponse.arrayBuffer()).byteLength > 0);
  }
  const stylesheetPaths = [...html.matchAll(/<link[^>]+href="([^"]+\.css)"/g)].map((match) => match[1]);
  for (const stylesheetPath of stylesheetPaths) {
    const stylesheet = await fetch(new URL(stylesheetPath, `${url}/`));
    assert.equal(stylesheet.status, 200);
    assert.match(stylesheet.headers.get('content-type'), /text\/css/);
    assert.ok((await stylesheet.text()).length > 0);
  }
  for (const path of ['/assets/missing.js', '/assets/missing.css', '/missing-route', '/.env', '/server.ts', '/%73erver.ts', '/backend/Cargo.toml', '/backend/src/main.rs']) {
    const missing = await fetch(`${url}${path}`, { headers: { accept: 'text/html' } });
    assert.equal(missing.status, 404, path);
    await missing.text();
  }
}

test('Vite dev serves the standalone entry and returns real 404s', async () => {
  const devServer = await createServer({ server: { port: 0, host: '127.0.0.1' }, logLevel: 'error' });
  try {
    await devServer.listen();
    const address = devServer.httpServer.address();
    await checkHttp(`http://127.0.0.1:${address.port}`);
  } finally {
    await devServer.close();
  }
});

test('preview serves relative version assets and does not expose source or SPA fallbacks', async () => {
  const html = await readFile(new URL('../dist/index.html', import.meta.url), 'utf8');
  assert.match(html, /src="\.\/assets\//);
  assert.doesNotMatch(html, /src="\/assets\//);
  const versionBase = '/sub-apps/chat/versions/test/';
  const previewServer = await preview({ base: versionBase, preview: { port: 0, host: '127.0.0.1' }, logLevel: 'error' });
  try {
    const address = previewServer.httpServer.address();
    await checkHttp(`http://127.0.0.1:${address.port}${versionBase.slice(0, -1)}`);
  } finally {
    await new Promise((resolve, reject) => previewServer.httpServer.close((error) => error ? reject(error) : resolve()));
  }
});

test('only the validated public endpoint reaches browser build output', async () => {
  const values = {
    VITE_CHAT_ENDPOINT: 'https://chat.example.invalid',
    VITE_PRIVATE_SECRET: 'phase2-private-secret-sentinel',
    DATABASE_URL: 'phase2-database-address-sentinel',
    HOST_SESSION: 'phase2-host-session-sentinel',
  };
  const previous = Object.fromEntries(Object.keys(values).map((key) => [key, process.env[key]]));
  Object.assign(process.env, values);
  try {
    const result = await build({ build: { write: false }, logLevel: 'error' });
    const output = (Array.isArray(result) ? result : [result]).flatMap((item) => item.output);
    const browserCode = output.map((item) => item.type === 'chunk' ? item.code : item.source).join('\n');
    assert.ok(browserCode.includes(values.VITE_CHAT_ENDPOINT));
    for (const key of ['VITE_PRIVATE_SECRET', 'DATABASE_URL', 'HOST_SESSION']) {
      assert.equal(browserCode.includes(values[key]), false, key);
    }
    assert.equal(browserCode.includes('XTransformPort'), false);
  } finally {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
});
