import assert from 'node:assert/strict';
import { launchBrowser } from '../../../tests/browser/runtime.mjs';
import { createServer } from 'vite';

const previousEndpoint = process.env.VITE_CHAT_ENDPOINT;
process.env.VITE_CHAT_ENDPOINT = 'https://chat.example.invalid';
const bindHost = process.platform === 'win32' ? '::1' : '127.0.0.1';
const dev = await createServer({ server: { host: bindHost, port: 0 }, logLevel: 'error' });
let browser;
try {
  await dev.listen();
  browser = await launchBrowser();
  const page = await browser.newPage();
  const errors = [];
  const events = [];
  let connections = 0;
  let messageId = 0;
  const message = (username, content, type) => ({
    id: `message-${++messageId}`, username, content, type, timestamp: '2026-10-15T00:00:00.000Z',
  });
  page.on('pageerror', (error) => errors.push(error.message));
  await page.routeWebSocket(/chat\.example\.invalid/, (socket) => {
    connections++;
    const user = { id: `user-${connections}`, username: '' };
    socket.onMessage((packet) => {
      if (packet === '40') socket.send(`40${JSON.stringify({ sid: user.id })}`);
      if (!String(packet).startsWith('42')) return;
      const [event, payload] = JSON.parse(String(packet).slice(2));
      events.push({ event, payload });
      if (event === 'join') {
        user.username = payload.username;
        socket.send(`42${JSON.stringify(['user-joined', { user, message: message('System', 'Joined controlled chat', 'system') }])}`);
        socket.send(`42${JSON.stringify(['users-list', { users: [user] }])}`);
      }
      if (event === 'message') socket.send(`42${JSON.stringify(['message', message(user.username, payload.content, 'user')])}`);
    });
    socket.send(`0${JSON.stringify({ sid: `transport-${connections}`, upgrades: [], pingInterval: 25000, pingTimeout: 20000, maxPayload: 1000000 })}`);
  });
  await page.goto(`http://${bindHost.includes(':') ? `[${bindHost}]` : bindHost}:${dev.httpServer.address().port}/`);
  await page.getByRole('button', { name: 'Connect', exact: true }).waitFor();
  assert.equal(connections, 0, 'page mounting must not connect to the configured endpoint');
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await page.getByText('Connected', { exact: true }).waitFor();
  await page.getByRole('textbox', { name: 'Username' }).fill('  Alice  ');
  await page.getByRole('button', { name: 'Join Chat' }).click();
  await page.getByText('Online users: 1').waitFor();
  await page.getByRole('textbox', { name: 'Message' }).fill('  Controlled hello  ');
  await page.getByRole('textbox', { name: 'Message' }).press('Enter');
  await page.getByText('Controlled hello', { exact: true }).waitFor();
  assert.deepEqual(events, [
    { event: 'join', payload: { username: 'Alice' } },
    { event: 'message', payload: { content: 'Controlled hello', username: 'Alice' } },
  ]);
  await page.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await page.getByText('Disconnected', { exact: true }).waitFor();
  assert.equal(await page.getByRole('textbox', { name: 'Username' }).isDisabled(), true);
  await page.getByRole('button', { name: 'Connect', exact: true }).click();
  await page.getByText('Connected', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Join Chat' }).click();
  await page.getByText('Online users: 1').waitFor();
  assert.equal(connections, 2);
  assert.equal(events.filter(({ event }) => event === 'join').length, 2);
  assert.deepEqual(errors, []);
  console.log(`PASS: browser ${await browser.version()}; explicit connect, unchanged join/message events, disconnect and rejoin; no page errors; all chat transport intercepted.`);
} finally {
  await browser?.close();
  await dev.close();
  if (previousEndpoint === undefined) delete process.env.VITE_CHAT_ENDPOINT;
  else process.env.VITE_CHAT_ENDPOINT = previousEndpoint;
}
