import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export function renderEdge(config) {
  if (!config || Object.keys(config).some((key) => !['publicOrigin', 'frontend', 'api'].includes(key))) throw new Error('Unknown edge configuration field')
  let origin
  try { origin = new URL(config.publicOrigin) } catch { throw new Error('publicOrigin must be an exact HTTPS origin') }
  if (origin.protocol !== 'https:' || origin.username || origin.password || origin.search || origin.hash || origin.pathname !== '/' || origin.origin !== config.publicOrigin) {
    throw new Error('publicOrigin must be an exact HTTPS origin')
  }
  if (!/^[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/.test(origin.hostname)) throw new Error('publicOrigin must use a DNS host')
  for (const name of ['frontend', 'api']) {
    if (!/^[a-z][a-z0-9-]*:[1-9][0-9]{0,4}$/.test(config[name] || '') || Number(config[name].split(':')[1]) > 65535) throw new Error(`${name} must be a private Compose service DNS name and port`)
  }
  return `worker_processes auto;
pid /tmp/nginx.pid;
error_log /dev/stderr crit;
events { worker_connections 512; }
http {
    access_log off;
    server_tokens off;
    client_max_body_size 32k;
    client_body_temp_path /tmp/client_body;
    proxy_temp_path /tmp/proxy;
    fastcgi_temp_path /tmp/fastcgi;
    uwsgi_temp_path /tmp/uwsgi;
    scgi_temp_path /tmp/scgi;
    resolver 127.0.0.11 valid=5s ipv6=off;
    resolver_timeout 2s;
    server {
        listen 8080;
        server_name ${origin.hostname};
        return 308 ${origin.origin}$request_uri;
    }
    server {
        listen 8443 ssl;
        server_name ${origin.hostname};
        ssl_certificate /etc/nginx/tls/fullchain.pem;
        ssl_certificate_key /etc/nginx/tls/privkey.pem;
        ssl_protocols TLSv1.2 TLSv1.3;
        add_header Strict-Transport-Security "max-age=31536000" always;
        add_header Referrer-Policy no-referrer always;
        set $oauth_api ${config.api};
        set $oauth_frontend ${config.frontend};
        proxy_http_version 1.1;
        proxy_connect_timeout 3s;
        proxy_read_timeout 20s;
        proxy_send_timeout 10s;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto https;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header Connection "";
        proxy_hide_header X-Powered-By;
        proxy_hide_header Referrer-Policy;
        location = /healthz { return 200 "ok\\n"; }
        location = /api { return 404; }
        location ^~ /api/ {
            proxy_hide_header Cache-Control;
            proxy_hide_header Referrer-Policy;
            proxy_hide_header X-Powered-By;
            add_header Cache-Control no-store always;
            add_header Strict-Transport-Security "max-age=31536000" always;
            add_header Referrer-Policy no-referrer always;
            proxy_pass http://$oauth_api$request_uri;
        }
        location / { proxy_pass http://$oauth_frontend$request_uri; }
    }
}
`
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 4) throw new Error('Usage: node deploy/render-edge.mjs <config.json> <output-nginx.conf>')
    const config = JSON.parse(await readFile(process.argv[2], 'utf8'))
    await writeFile(process.argv[3], renderEdge(config), { flag: 'wx', mode: 0o600 })
  } catch (error) {
    // Configuration may contain accidental credentials. Never echo JSON input.
    console.error(error instanceof SyntaxError ? 'Invalid edge JSON configuration' : error.message)
    process.exitCode = 1
  }
}
