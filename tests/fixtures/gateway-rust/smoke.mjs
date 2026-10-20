// Node is only the test driver; the peer itself is the standalone Rust binary.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const fixtureDir = dirname(fileURLToPath(import.meta.url));
const binary = process.env.FIXTURE_BINARY || join(fixtureDir, 'target', 'debug',
  `gateway-rust-transport-fixture${process.platform === 'win32' ? '.exe' : ''}`);
const child = spawn(binary, [], {
  env: { ...process.env, BIND_ADDR: '127.0.0.1:0', INSTANCE_ID: 'smoke-fixture', ALLOW_CONTAINER_BIND: '0' },
  stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
});
let errorOutput = '';
child.stderr.on('data', chunk => { errorOutput += chunk.toString(); });
let socket;
try {
  const port = await new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => reject(new Error(`Rust startup deadline: ${errorOutput}`)), 15_000);
    timer.unref();
    child.once('error', error => { clearTimeout(timer); reject(error); });
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`Rust exited ${code}: ${errorOutput}`)); });
    child.stdout.on('data', chunk => {
      output += chunk.toString();
      const found = output.match(/listening=127\.0\.0\.1:(\d+) instance=smoke-fixture/);
      if (found) { clearTimeout(timer); resolve(Number(found[1])); }
    });
  });
  const base = `http://127.0.0.1:${port}`;
  const health = await (await fetch(`${base}/health`, { signal: AbortSignal.timeout(5_000) })).json();
  assert.equal(health.instanceId, 'smoke-fixture');
  assert.equal(health.marker, 'TRANSPORT_FIXTURE_ONLY');
  const response = await fetch(`${base}/work-orders-api/echo?q=a%2Fb&empty=`, {
    method: 'POST', headers: { Authorization: 'Bearer fixture-token', Origin: 'null' }, body: 'polling-body',
    signal: AbortSignal.timeout(5_000),
  });
  assert.equal(response.headers.get('access-control-allow-origin'), 'null');
  const observed = await response.json();
  assert.equal(observed.uri, '/work-orders-api/echo?q=a%2Fb&empty=');
  assert.equal(observed.body, 'polling-body');
  assert.equal(observed.headers.authorization, 'Bearer fixture-token');
  assert.equal(observed.headers.origin, 'null');
  const polling = await (await fetch(`${base}/socket.io/?EIO=4&transport=polling`, {
    signal: AbortSignal.timeout(5_000),
  })).text();
  assert.equal(polling[0], '0');
  assert.equal(JSON.parse(polling.slice(1)).fixture.query, 'EIO=4&transport=polling');

  socket = new WebSocket(`ws://127.0.0.1:${port}/socket.io/?EIO=4&transport=websocket&t=a%2Bb`);
  socket.binaryType = 'arraybuffer';
  await new Promise((resolve, reject) => {
    let stage = 0;
    const timer = setTimeout(() => reject(new Error(`WebSocket deadline at stage ${stage}`)), 10_000);
    socket.onerror = event => { clearTimeout(timer); reject(event.error || new Error('WebSocket error')); };
    socket.onclose = () => { clearTimeout(timer); reject(new Error(`WebSocket closed at stage ${stage}`)); };
    socket.onmessage = event => {
      try {
        if (stage === 0) {
          const metadata = JSON.parse(event.data);
          assert.equal(metadata.marker, 'TRANSPORT_FIXTURE_ONLY');
          assert.equal(metadata.query, 'EIO=4&transport=websocket&t=a%2Bb');
          assert.equal(metadata.headers.upgrade.toLowerCase(), 'websocket');
          stage = 1;
          socket.send('fixture-text');
        } else if (stage === 1) {
          assert.equal(event.data, 'fixture-text');
          stage = 2;
          socket.send(Uint8Array.from([0, 1, 127, 255]));
        } else {
          assert.deepEqual([...new Uint8Array(event.data)], [0, 1, 127, 255]);
          clearTimeout(timer);
          resolve();
        }
      } catch (error) { clearTimeout(timer); reject(error); }
    };
  });
  console.log('PASS Rust process: health, URI/query/body/auth/Origin echo, polling and real WebSocket text/binary roundtrips');
} finally {
  if (socket) socket.close();
  child.kill();
  await new Promise(resolve => {
    const timer = setTimeout(resolve, 3_000);
    timer.unref();
    if (child.exitCode !== null) { clearTimeout(timer); resolve(); }
    else child.once('exit', () => { clearTimeout(timer); resolve(); });
  });
}
