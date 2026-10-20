import assert from 'node:assert/strict';
import test from 'node:test';
import { parseChatEndpoint } from '../src/chat-config.ts';
import { createChatSocket } from '../src/chat-client.ts';

test('the standalone demo stays offline until a public endpoint and explicit connect are supplied', () => {
  assert.equal(parseChatEndpoint(undefined), null);
  assert.equal(parseChatEndpoint('  '), null);
  assert.equal(parseChatEndpoint(' https://chat.example.invalid/ '), 'https://chat.example.invalid');
  assert.equal(parseChatEndpoint('http://127.0.0.1:3003'), 'http://127.0.0.1:3003');
  const socket = createChatSocket('http://127.0.0.1:1');
  assert.equal(socket.connected, false);
  assert.equal(socket.active, false);
  assert.equal(socket.io.opts.autoConnect, false);
  assert.equal(socket.io.opts.withCredentials, false);
  assert.equal(socket.io.opts.path, '/');
  socket.disconnect();
});

test('endpoint configuration rejects credentials, query, fragment, paths and non-HTTP protocols', () => {
  for (const endpoint of [
    '/socket.io', 'wss://chat.example.invalid', 'file:///tmp/socket',
    'https://user:password@chat.example.invalid', 'https://chat.example.invalid?token=private',
    'https://chat.example.invalid#session', 'https://chat.example.invalid/socket.io',
  ]) {
    assert.throws(() => parseChatEndpoint(endpoint), /public HTTP\(S\) origin/);
  }
});
