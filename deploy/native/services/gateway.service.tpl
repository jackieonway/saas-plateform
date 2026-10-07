[Unit]
Description=Dante HTTPS Nginx gateway
After=network-online.target dante-backend.service dante-notification.service
[Service]
User=dante-gateway
Group=dante-gateway
WorkingDirectory=@@ROOT@@
ExecStartPre=@@NGINX@@ -t -p @@ROOT@@/ -c @@ROOT@@/configuration/nginx.conf
ExecStart=@@NGINX@@ -p @@ROOT@@/ -c @@ROOT@@/configuration/nginx.conf
ExecReload=/bin/kill -HUP $MAINPID
KillSignal=SIGQUIT
Restart=on-failure
RestartSec=5
AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=@@ROOT@@
UMask=0077
[Install]
WantedBy=multi-user.target
