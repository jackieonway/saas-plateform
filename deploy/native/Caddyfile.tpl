{
    admin off
    log {
        level WARN
    }
}
@@ORIGIN@@ {
    bind @@LISTEN_HOST@@
    tls "@@CERTIFICATE@@" "@@CERTIFICATE_KEY@@"
    encode zstd gzip
    header {
        Strict-Transport-Security "max-age=31536000"
        X-Content-Type-Options "nosniff"
        Referrer-Policy "same-origin"
        Permissions-Policy "camera=(), microphone=(), geolocation=()"
        Content-Security-Policy "@@CSP@@"
        -Server
    }
    @private path /api/internal/* /metrics /work-orders-api/metrics
    handle @private {
        respond 404
    }
    handle /notification-health {
        rewrite * /health
        reverse_proxy @@NOTIFICATION@@ {
            header_up X-Real-IP {remote_host}
            header_up X-Forwarded-For {remote_host}
            header_up X-Forwarded-Proto https
        }
    }
    @platform path /api/* /oauth/*
    handle @platform {
        reverse_proxy @@BACKEND@@ {
            header_up X-Real-IP {remote_host}
            header_up X-Forwarded-For {remote_host}
            header_up X-Forwarded-Proto https
        }
    }
    handle /socket.io/* {
        reverse_proxy @@NOTIFICATION@@ {
            header_up X-Real-IP {remote_host}
            header_up X-Forwarded-For {remote_host}
            header_up X-Forwarded-Proto https
        }
    }
    @business path /work-orders-api /work-orders-api/*
    handle @business {
        reverse_proxy @@WORK_ORDERS@@ {
            header_up X-Real-IP {remote_host}
            header_up X-Forwarded-For {remote_host}
            header_up X-Forwarded-Proto https
        }
    }
    handle_path /sub-apps/* {
        root * "@@MICRO_ROOT@@"
        header Cache-Control "no-cache"
        header Access-Control-Allow-Origin "*"
        @module path *.mjs *.cjs
        header @module Content-Type "application/javascript"
        @immutable {
            path_regexp version ^/[a-z0-9-]+/versions/[A-Za-z0-9._-]+/.*[^/]$
            not path_regexp html (?i)\.html?$
        }
        header @immutable Cache-Control "public, max-age=31536000, immutable"
        file_server
    }
    handle /assets/* {
        root * "@@HOST_ROOT@@"
        file_server
    }
    handle {
        root * "@@HOST_ROOT@@"
        try_files {path} /index.html
        header Cache-Control "no-cache"
        file_server
    }
    handle_errors {
        header Cache-Control "no-store"
        respond "{err.status_code} {err.status_text}" {err.status_code}
    }
}
