# 使用现有宿主机 Nginx

适用：Nginx 安装在 Linux 宿主机，平台和业务容器运行在同一台机器。Nginx 负责公开 HTTPS，容器只发布 127.0.0.1 端口。本方案使用独立 Compose 入口，不启动平台自带 gateway。

## 1. 容器端口

| 服务 | 宿主机回环地址 | 容器端口 |
| --- | --- | --- |
| 平台静态前端 | 127.0.0.1:18080 | 8080 |
| 平台 API | 127.0.0.1:13001 | 3001 |
| 工单 API / 静态前端 | 127.0.0.1:3004 / 3013 | 3004 / 8080 |
| 门店 API / 静态前端 | 127.0.0.1:3031 / 3014 | 3031 / 8080 |
| 可选通知 | 127.0.0.1:13003 | 3003 |

工单与门店使用自身 Compose 的默认回环端口。若修改端口，同步改 Nginx 上游。正式使用各仓库审阅的镜像摘要，构建步骤沿用各自部署指南。

## 2. 启动平台

按 backend/compose.host-nginx.env.example 准备源码之外的私有环境文件，例如 /srv/saas/private/platform.env。填写平台域名、数据库、镜像、初始化管理员、签名与独立指标凭据，以及实际数据授权策略。

准备受控 edge 网络；例子选择未冲突的 172.30.0.0/24：

```sh
docker network create --driver bridge --subnet 172.30.0.0/24 platform-edge
docker network inspect platform-edge --format '{{(index .IPAM.Config 0).Gateway}}'
```

已有网络不重复创建。HOST_NGINX_PROXY_IP 填 API 实际收到的 Nginx 连接对端，通常为该 Docker bridge 网关，但必须按实际部署确认。它不是访客地址或域名。进程将其设为 TRUSTED_PROXY_IPS，以正确解析来源、审计和登录预算。

在平台 backend 执行：

```sh
docker compose --env-file /srv/saas/private/platform.env -f compose.host-nginx.yml config --quiet
docker compose --env-file /srv/saas/private/platform.env -f compose.host-nginx.yml up -d
docker compose --env-file /srv/saas/private/platform.env -f compose.host-nginx.yml ps
```

只用这个入口文件，不与原 compose.yml 合并，否则旧 gateway 仍可能启动并占用 80/443。

## 3. 配置已有 Nginx

使用 [完整示例](examples/host-nginx.conf.example)，复制到已有 Nginx 的 http 包含目录，例如 /etc/nginx/conf.d/saas.conf。不要把含 map/upstream 的完整文件粘到现有 server 块内部。面板只允许填写 server 内容时，map/upstream 要放到面板支持的 http 级配置位置。

全局替换 saas.example.com，填写已经准备好的真实证书链和私钥路径。示例同时包含可选工单、门店、通知上游：只启用实际部署的 location，未部署的改为 return 404。同一域名只能保留一套生效的 server 配置，避免与已有站点冲突。

API 的 proxy_pass 不带尾部 /，保留 /api、/oauth、/work-orders-api、/stores-api 前缀；微应用静态版本 proxy_pass 带尾部 /，将公开版本前缀剥离后交给静态镜像。

平台 PUBLIC_APP_URL、业务前端 HOST_ORIGIN 和平台注册入口都使用实际 HTTPS 来源。微应用 APP_VERSION 必须匹配 /sub-apps/<应用>/versions/<版本>/；旧版本继续由独立旧静态服务/版本存储提供，不能把所有旧 URL 指向同一个新容器。

```sh
sudo nginx -t
sudo systemctl reload nginx
```

先测试配置成功，再 reload。平台页面、API、OAuth 回调、静态 MIME 与缺资源 404 都需要通过真实域名检查。

## 4. 可选通知

通知镜像独立构建，按 compose.notification.env.example 准备私有 notification.env 和只读数据库身份。在 backend 执行：

```sh
docker compose --env-file /srv/saas/private/platform.env --env-file /srv/saas/private/notification.env -f compose.host-nginx.yml -f compose.notification.yml -f compose.host-nginx.notification.yml --profile notification config --quiet
docker compose --env-file /srv/saas/private/platform.env --env-file /srv/saas/private/notification.env -f compose.host-nginx.yml -f compose.notification.yml -f compose.host-nginx.notification.yml --profile notification up -d
```

有私有 CA 时按已有 CA overlay 再配置正常 TLS 信任。启用后 Nginx /socket.io/ 转发到 127.0.0.1:13003，保留升级头和 75 秒读取期限。Nginx 不额外伪造 CORS，业务和通知继续执行精确来源及身份授权。

## 5. 其他部署形态

如果现有 Nginx 也在 Docker 容器里，127.0.0.1 指向该 Nginx 容器自身，不能使用本示例的回环上游。应将 Nginx 加入受控 edge 网络，改用 backend:3001、frontend-web:8080 和业务服务 DNS，并核对实际可信代理 IP。

这些文件是可审阅的部署模板，没有连接你的服务器或修改现有 Nginx。当前本机未运行 nginx -t、真实 Docker/Compose 或线上验证。
