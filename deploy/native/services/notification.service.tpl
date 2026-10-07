[Unit]
Description=Dante notification service
After=network-online.target dante-backend.service
[Service]
User=dante-notification
Group=dante-notification
WorkingDirectory=@@ROOT@@/current/notification-ws
EnvironmentFile=@@ROOT@@/secrets/notification.env
ExecStart=@@NODE@@ @@NOTIFICATION_ENTRY@@
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
UMask=0077
[Install]
WantedBy=multi-user.target
