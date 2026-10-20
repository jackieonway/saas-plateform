# Linux 同机 Nginx 镜像部署

Phase 3 提供独立入口、宿主静态镜像、Rust 平台与 PostgreSQL 的部署基线。入口终止 HTTPS，通过私有 bridge 的 service DNS 访问上游；只有入口映射公网 80/443。平台不持有 Docker socket。所有命令由操作者在明确目标上执行，本目录不会自动部署或改动生产数据。

默认 [docker-compose.yml](../docker-compose.yml) 使用已构建的固定镜像，不挂微应用产物目录。通知已由 [独立 Rust override](compose.notification.yml) 提供；工单和门店的 Rust API、四个 v1/v2 静态镜像通过新的 [业务 override](compose.business.yml) 显式接入，完整构建、健康门禁、版本路由与发行清单说明见 [microapps/README.md](microapps/README.md)。Phase34已退役旧Node工单；compose.legacy.yml仅保留空服务兼容文件，业务运行使用Rust business override。

Phase28 受管验收的 API 路径是在本次冻结源码的隔离 Linux 工具环境执行独立 locked Cargo 编译，再校验 ELF/source/inputs/platform/SHA-256 并分别装入 runtime 镜像，报告标为 `verified-prebuilt-ELF`。两业务默认 Dockerfile 保留独立源码 builder；没有实际执行该 builder 时不能将其记为 PASS。四个静态镜像仍各自真实按应用 lockfile 从源码 Docker build。这里说明当前实现和验收路径，不预先声明 registry 拉取、浏览器或生产部署已通过；实际结果以本次 Phase28 证据为准。

## 构建及配置

Phase32 的[私有监控、指标语义、状态转移事件与候选基线](monitoring/README.md)通过 `compose.monitoring.yml` 显式启用。工单/门店源码 Docker build 均以仓库根为 context，包含纯固定指标协议 crate；运维凭据与应用认证凭据独立，公网入口禁止采集私有 metrics。

使用仓库声明的 Node 24.18.0/npm 11.16.0/Rust 1.97.0。全新源码快照、原生构建及产物 manifest 见 [native/README.md](native/README.md)。宿主 Docker build 不复制已有 node_modules/dist；构建阶段锁定安装，运行阶段仅 Nginx。以下标签只用于本机构建，发行需要记录实际最终 digest 与基础镜像身份，不向示例仓库推送：

```sh
docker build -f frontend/Dockerfile --build-arg SOURCE_REVISION="$(git rev-parse HEAD)" --build-arg HOST_BUILD_ID=host-candidate -t platform-host:candidate .
docker build -f backend/Dockerfile -t platform-api:candidate .
docker build -f deploy/gateway/Dockerfile -t platform-edge:candidate .
# 仅 ZIP 兼容需要
docker build -f deploy/gateway/Dockerfile.micro-static -t platform-zip-static:candidate .
```

将 [compose.env.example](compose.env.example) 复制到检出目录之外的受限文件，填入实际镜像 `registry/repository@sha256:<64 hex>`、连接串、随机密钥和证书路径。`.invalid` 只是占位域名，不能直接用于部署。镜像 digest 与源码 SHA、构建输入摘要、buildId 的绑定由发行清单保存。平台 JWT、DB URL 和 TLS 私钥均不得进入前端构建变量或静态 metadata。

将 [gateway/config.example.json](gateway/config.example.json) 复制到操作者目录，设置与环境文件完全相同的 `publicOrigin`，然后渲染真实配置：

```sh
node deploy/gateway/render.mjs /secure/platform-gateway.json /secure/nginx.conf
```

TLS 证书必须覆盖实际域名，UID/GID 101 必须能读取证书、私钥和配置。用最小组权限或 ACL 提供读取，不能公开私钥。入口不管理 ACME、不会自动签发证书；证书续期由部署方完成，更新后进行 `nginx -t` 和受控 reload/重建。客户端始终正常验证 CA/主机名。

## 先创建受控网络，再检查部署

检查宿主及 Docker 已有路由，选择不冲突的私有子网。`172.30.0.0/24` 和 `.10` 只是候选，不代表实际目标配置：

```sh
docker network create --driver bridge --subnet 172.30.0.0/24 platform-edge
docker network inspect platform-edge
docker compose --env-file /secure/platform.env config --quiet
```

确认 `EDGE_NETWORK_NAME` 指向该 bridge，`GATEWAY_PRIVATE_IP` 位于实际子网内且没有占用。该值同时用于入口固定 IP 和 Rust `TRUSTED_PROXY_IPS`，不能填公共域名、前端 IP 或任意 CIDR。外部受控负载均衡存在时才在 gateway JSON 的 `trustedProxyIps` 填其精确私网 IP。默认入口覆盖客户端提交的 forwarded headers；业务 CORS 由上游 Rust 处理，入口透传。

配置检查仅解析，不启动数据库。实际部署会启动平台并执行其正常启动/迁移行为，操作者必须先核对目标与升级/备份条件：

```sh
docker compose --env-file /secure/platform.env up -d
docker compose --env-file /secure/platform.env ps
docker compose --env-file /secure/platform.env exec gateway nginx -t
```

入口、宿主及 ZIP Nginx 都以非 root 用户、只读根文件系统运行，只向 `/tmp` tmpfs 写 PID/临时文件，不在只读配置目录运行 envsubst。上游容器不发布宿主端口。平台库另处 internal network，宿主静态服务不加入数据库网络。外部 edge network 的生命周期归操作者，Compose down 不删除它。

## 微应用镜像与 ZIP 兼容

推荐微应用镜像自己持有静态字节。操作者在 gateway JSON 的 `versionRoutes` 中逐版本指定 `app/version/upstream`，将服务名加入 `allowedServices`，把对应不可变镜像接入同一受控私网。平台登记公开 URL 不会自动创建 Nginx 规则。旧版本仍须保留原路由/静态字节；未知 `/sub-apps` 版本严格 404。开发用未版本化 Vue/jQuery 路径不作为生产路由。

工单/门店使用 [compose.business.env.example](compose.business.env.example) 提供的六个固定 digest 和两个独立业务库/机密客户端配置；[业务版本路由示例](microapps/gateway.example.json) 保留两应用各自 v1/v2。生产目标可以通过 [pull-start.sh](microapps/pull-start.sh) 执行 Compose pull/start/health，随后先由 Rust CLI 在权威持久 inventory 执行 artifact-bind/verify，再受控推进路由与 URL register/preflight/activate。CI/Compose 拒绝 unhealthy、平台拒绝入口/资源或 healthUrl 预检失败，是分别记录的门禁。外部静态 origin 使用明确绝对业务 HTTPS API base 或自有受控代理；宿主白名单仍为公开宿主来源。平台不保存镜像 digest，不读取 Docker health，也不因发布而自动授权。

ZIP 兼容显式合并 [compose.zip.yml](compose.zip.yml) 并启用 profile；默认纯镜像部署不需要该文件的变量或目录。预先创建持久绝对 `MICRO_APPS_DIR`，由 Rust UID 10001 可写、静态 UID 101 可读。仅平台挂 RW；独立 `micro-static` 挂 RO；入口和宿主均不挂此目录。`create_host_path: false` 可防止拼错路径时悄悄创建空目录。

将 [gateway/micro-static.example.json](gateway/micro-static.example.json) 渲染到 `MICRO_STATIC_CONFIG_FILE`，并在入口 JSON 设置 `zipStatic: "micro-static:8080"`，重新渲染入口：

```sh
node deploy/gateway/render.mjs /secure/micro-static.json /secure/micro-static.conf
docker compose --env-file /secure/platform.env -f docker-compose.yml -f deploy/compose.zip.yml --profile zip config --quiet
docker compose --env-file /secure/platform.env -f docker-compose.yml -f deploy/compose.zip.yml --profile zip up -d
```

仅 `/sub-apps/<app>/versions/<version>/...` 对外可读；暂存、点文件、symlink 和缺失文件为 404。HTML no-cache、版本资源 immutable、错误 no-store；微应用匿名 CORS 不能扩展成平台 API 全局 CORS。ZIP 原子安装与恢复对账由 Phase 5 完成，本阶段不宣称已解决发布事务。

## 更新、回退及验收

宿主更新和回退都须导入当前在线镜像的全部旧 assets，再构建候选；具体可执行命令与不可变 digest/本地 image ID 校验见 [static/README.md](static/README.md)。只保存旧镜像不保证旧 URL 仍可访问。该策略没有运行时资源共享卷，也不自动删除历史文件。

Docker DNS resolver 每五秒重新解析变量上游，仍须实际验证换 IP 后恢复；不能只凭配置文本推断。运行专用验收（Node 只驱动测试，不提供后台服务）：

```sh
node --test tests/nginx-render-policy.mjs tests/production-native-config.mjs deploy/static/tests/static-image-policy.mjs deploy/native/tests/build.test.mjs
node --test tests/micro-release-gateways.mjs
```

后者使用真实 Linux Docker/Nginx、正常验证的本地 HTTPS 和 test-only Rust transport fixture，创建并清理自有资源；工具、镜像或 TLS 环境缺失须失败并记录，不能用 skip 替代必需验收。它只证明传输层，不证明完整 Socket.IO 事件/授权、业务迁移或现网发布。实测分类及限制见 [Phase 3 证据](../docs/validation/phase-03.md)。监控和恢复分别见 monitoring/README.md 与 recovery/README.md，当前入口为独立 Rust 工具。

Phase33新增独立Rust加密三库备份和隔离新库恢复：[恢复runbook](recovery/README.md)。试点候选24h/4h；完整远端取回、凭据失效与业务验收结果见阶段证据。生产PITR、真实异机/密钥托管、外部告警和定时安装另行验收。
