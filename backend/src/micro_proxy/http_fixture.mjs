// Optional HTTP fixture for hosts whose firewall allows Node but blocks Rust test executables.
// This does not change network policy. All listeners use an ephemeral loopback port.
import http from 'node:http';
const routes = JSON.parse(process.argv[2]);
const requests = [];
const server = http.createServer((req, res) => {
  const path = new URL(req.url, 'http://localhost').pathname;
  if (path === '/__requests') {
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify(requests));
    return;
  }
  requests.push({ method: req.method, url: req.url, headers: req.headers });
  const response = routes[path] ?? routes['*'] ?? { status: 404, body: 'Not found' };
  const body = response.bodyBytes === undefined ? (response.body ?? '') : Buffer.alloc(response.bodyBytes, 'x');
  const headers = { ...(response.headers ?? {}) };
  if (response.bodyBytes !== undefined && !headers['transfer-encoding']) headers['content-length'] = body.length;
  res.writeHead(response.status ?? 200, headers);
  res.end(body);
});
server.listen(0, '127.0.0.1', () => process.stdout.write(`http://127.0.0.1:${server.address().port}\n`));
