[Unit]
Description=Dante independent work orders API
After=network-online.target dante-gateway.service
[Service]
User=dante-work-orders
Group=dante-work-orders
WorkingDirectory=@@ROOT@@/current/work-orders-api
EnvironmentFile=@@ROOT@@/secrets/work-orders.env
ExecStart=@@NODE@@ @@WORK_ORDERS_ENTRY@@
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
UMask=0077
[Install]
WantedBy=multi-user.target
