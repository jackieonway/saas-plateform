daemon off;
worker_processes 1;
pid "@@ROOT@@/nginx.pid";
error_log "@@ROOT@@/nginx-error.log" warn;
events { worker_connections 1024; }
http {
    include "@@MIME_TYPES@@";
    types { application/javascript mjs cjs; }
    default_type application/octet-stream;
    server_tokens off;
    sendfile on;
    client_max_body_size 5m;
    # Request paths may contain private data. Log routing/service status only;
    # never Authorization, query strings, cookies or POST bodies.
    log_format safe '$time_iso8601 $request_method $status $request_time $upstream_status';
    access_log "@@ROOT@@/nginx-access.log" safe;
    client_body_temp_path "@@ROOT@@/nginx-temp/client_body";
    proxy_temp_path "@@ROOT@@/nginx-temp/proxy";
    fastcgi_temp_path "@@ROOT@@/nginx-temp/fastcgi";
    uwsgi_temp_path "@@ROOT@@/nginx-temp/uwsgi";
    scgi_temp_path "@@ROOT@@/nginx-temp/scgi";
    map $http_upgrade $notification_connection { default upgrade; '' close; }
    map "$status:$uri" $micro_cache_control {
        default "no-cache";
        ~^[45][0-9][0-9]: "no-store";
        ~*\.html?$ "no-cache";
        ~/$ "no-cache";
        ~^200:/sub-apps/[a-z0-9-]+/versions/[A-Za-z0-9._-]+/ "public, max-age=31536000, immutable";
    }
    server {
        listen @@LISTENER@@ ssl;
        server_name @@HOST@@;
        ssl_certificate "@@CERTIFICATE@@";
        ssl_certificate_key "@@CERTIFICATE_KEY@@";
        ssl_protocols TLSv1.2 TLSv1.3;
        ssl_session_cache shared:DanteTLS:1m;
        ssl_session_tickets off;
        root "@@HOST_ROOT@@";
        index index.html;
        add_header Strict-Transport-Security "max-age=31536000" always;
        add_header X-Content-Type-Options "nosniff" always;
        add_header Referrer-Policy "same-origin" always;
        add_header Permissions-Policy "camera=(), microphone=(), geolocation=()" always;
        add_header Content-Security-Policy "@@CSP@@" always;
        # Internal health authorization and metrics are consumed directly on
        # private loopback ports, never exposed through the public gateway.
        location ^~ /api/internal/ { return 404; }
        location = /metrics { return 404; }
        location = /work-orders-api/metrics { return 404; }
        location = /notification-health {
            proxy_pass http://@@NOTIFICATION@@/health;
            proxy_set_header Host $http_host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $remote_addr;
            proxy_set_header X-Forwarded-Proto https;
        }
        location /api/ {
            proxy_pass http://@@BACKEND@@;
            proxy_set_header Host $http_host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $remote_addr;
            proxy_set_header X-Forwarded-Proto https;
        }
        location /oauth/ {
            proxy_pass http://@@BACKEND@@;
            proxy_set_header Host $http_host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $remote_addr;
            proxy_set_header X-Forwarded-Proto https;
        }
        location = /work-orders-api {
            proxy_pass http://@@WORK_ORDERS@@;
            proxy_set_header Host $http_host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $remote_addr;
            proxy_set_header X-Forwarded-Proto https;
        }
        location /work-orders-api/ {
            proxy_pass http://@@WORK_ORDERS@@;
            proxy_set_header Host $http_host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $remote_addr;
            proxy_set_header X-Forwarded-Proto https;
        }
        location /socket.io/ {
            proxy_pass http://@@NOTIFICATION@@;
            proxy_http_version 1.1;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection $notification_connection;
            proxy_set_header Host $http_host;
            proxy_set_header X-Real-IP $remote_addr;
            proxy_set_header X-Forwarded-For $remote_addr;
            proxy_set_header X-Forwarded-Proto https;
            proxy_read_timeout 75s;
            proxy_buffering off;
        }
        location ~ ^/sub-apps/([a-z0-9-]+)/versions/([A-Za-z0-9._-]+)/(.*)$ {
            alias "@@MICRO_ROOT@@/$1/versions/$2/$3";
            add_header Cache-Control $micro_cache_control always;
            add_header Access-Control-Allow-Origin "*" always;
            add_header Strict-Transport-Security "max-age=31536000" always;
            add_header X-Content-Type-Options "nosniff" always;
            add_header Referrer-Policy "same-origin" always;
            add_header Content-Security-Policy "@@CSP@@" always;
        }
        location /sub-apps/ { try_files $uri $uri/ =404; }
        # Missing host build chunks also remain 404 instead of receiving HTML.
        location /assets/ { try_files $uri =404; }
        location / {
            try_files $uri $uri/ /index.html;
            expires -1;
        }
    }
}
