# 原生构建与 Nginx 配置

原生构建入口从**当前工作树**创建白名单源码快照，分别安装锁定的前端依赖、构建三个 Vite 示例及宿主，并编译 Rust 平台 API。Node 只参与构建；交付 API 是 `saas-platform-backend`。通知和工单旧 Node 后台分别等待台账 Phase 23、17 迁移，本入口不将其打包为生产服务。

## 工具与构建

使用 Node 24.18+（24.x）、npm 11.16+（11.x）、Rust/Cargo 1.97.0，以及 Rust 目标平台所需的链接器。原生产 CI 使用 Linux；在 Windows 构建得到的是 Windows `.exe`，不能将其称为 Linux 验收。构建不会运行数据库迁移或启动后台。

```sh
node deploy/native/build.mjs --out-dir /srv/platform-builds/release-20261015 --cargo /usr/bin/cargo
```

`--out-dir` 必须是**尚不存在的绝对路径**；成功或失败的既有目录均拒绝覆盖。修复后换用新的目录。默认源目录为本脚本所在工程，可显式传入 `--source-root /absolute/checkout`；该目录必须具有有效 Git HEAD。若 npm 未安装于 Node 相邻的标准位置，使用 `--npm-cli /absolute/path/to/npm-cli.js`。没有自动读取私有配置的构建参数。

构建顺序为四个包逐一执行 `npm ci --ignore-scripts --no-audit --no-fund`，以 `scripts/sync-micro-sdk.mjs --check` 校验 SDK 生成物与登记的 vendor 副本，再构建 Vue/jQuery/聊天示例，同步 Vue/jQuery 静态目录到快照内的宿主，再构建宿主并执行 `deploy/static/finalize-build.mjs` 校验/合并已准备的历史 assets，最后执行 `cargo build --release --locked --bin saas-platform-backend`。漂移检查失败即停止，不在构建前自动同步 SDK。编译缓存属于本次输出目录，旧 `node_modules`、`dist`、`target` 不进入源码快照。前端源码中的变更和新文件会被纳入快照，构建不依赖 `git archive HEAD`。

SDK 漂移检查的源码快照包含完整 `business-apps/stores/frontend/vendor/micro-sdk/` 登记副本及配套声明。此项只维持统一 SDK 校验；门店前端的独立构建使用根 `build:stores:frontend`，门店镜像与外部版本 URL 交付仍按后续阶段执行。

## 输出接口

```text
<out-dir>/
  build-result.json
  source-manifest.json
  source/                         # 本次白名单源码；未包含本地私有配置
  logs/                           # 分步构建日志
  work/                           # 本次依赖/编译缓存，不是部署包
  artifacts/                      # 全部步骤成功后才发布此目录
    platform/saas-platform-backend[.exe]
    frontend/index.html
    examples/{vue,jquery,websocket}/
    deploy/{gateway,static,native}/
    deploy/README.md
    deploy/compose.zip.yml
    deploy/compose.env.example
    deploy/compose.legacy.env.example
    docker-compose.yml
```

`source-manifest.json` 中的 `sourceSha` 指向 Git HEAD；`files` 的逐文件 SHA-256 和 `inputsSha256` 标识**实际工作树快照**。工作树有未提交变更时，不能只用 HEAD SHA 声称产物来自那个提交。`buildId` 由 HEAD 前缀和实际输入摘要组成。`build-result.json` 记录工具版本、平台/架构、步骤、退出码、日志位置及每个交付文件的摘要。部署前应检查 `status === "SUCCEEDED"`，并验证产物摘要；原生二进制要求兼容的 OS、架构和系统运行库。

失败保留 `status: "FAILED"`、已完成的步骤和日志，且不发布 `artifacts/`。不要部署 `work/` 中间目录。构建目录本身包含依赖缓存和源码，归档交付应只选择成功 manifest 与 `artifacts/`；CI 采用该接口。

## 配置与秘密边界

宿主从仓库内 `examples/micro-app-sdk` 导入已登记协议和能力检查，源码快照因此包含该 SDK 与 `scripts/sync-micro-sdk.mjs`。全量漂移校验另精确包含工单的 `vendor/iframe-bridge.js`、`vendor/micro-sdk.mjs`，不引入工单后台或其他业务源码。宿主 Docker build 保留 `/app/frontend` 与 `/app/examples/micro-app-sdk` 的同层目录关系，在锁定依赖安装后运行 `--check --scope sdk`；完整 Rust/vendor 漂移由 fast 与原生构建的默认 `--check` 覆盖。

输入仅限源码和版本化模板。构建先按路径排除 `.env*`、`*.env*`、`.npmrc`、证书/私钥、private/secret 目录、credentials/runtime/deployment 等私有配置文件、日志及旧生成目录，**不打开私有 `.env`**。环境模板的明确例外仅为公开的 `deploy/compose.env.example` 与 `deploy/compose.legacy.env.example`；不会复制相邻真实 `compose.env`。源码快照中的 `frontend/.npmrc` 是另一个精确例外：只允许公开的 `engine-strict=true` 与空行，任何 token、registry、注释或其他配置在写入快照前即被拒绝，保证该快照仍满足宿主 Dockerfile 的 COPY 输入。其他 `.npmrc` 不读不复制。`runtime/` 等正常源码目录会被保留。不复制整个工程；宿主旧 `public/sub-apps` 会被排除，再从本次示例产物重建。源码符号链接被拒绝，npm 锁文件中的非 HTTPS、带用户密码或查询参数的资源 URL 被拒绝。若显式旧服务兼容 override 已存在，也仅复制已列出的 `compose.legacy.yml`/`deploy/compose.legacy.yml`，不打包旧 Node 服务源码。

构建子进程只继承系统工具所需的环境，不继承 `DATABASE_URL`、`JWT_SECRET`、`VITE_*`、`MICRO_PUBLIC_*`、`NODE_OPTIONS` 或 npm token。npm 使用本次受控空白用户/全局配置，不读取操作者的私有 npmrc。运行时连接串、签名密钥、TLS 私钥应通过独立受限文件或部署平台注入，不能放在 `artifacts/`、静态目录或构建日志中。

Nginx 原生配置使用同目录的 [config.example.json](config.example.json) 作为公开路径模板，在构建产物之外另存配置后渲染：

```sh
node deploy/native/render.mjs --config /srv/platform/config.json --out-dir /srv/platform/configuration-new
```

原生配置输出 `nginx.conf`。独立入口配置使用 `node deploy/gateway/render.mjs /srv/platform/gateway.json /srv/platform/nginx.conf`，其中 `kind` 为 `gateway` 或 `micro-static`。容器与代理策略位于 [../gateway](../gateway)；宿主静态配置、历史 assets 导入和回退步骤位于 [../static/README.md](../static/README.md)。`deploy/static/history` 是经 manifest/摘要校验的受控构建输入，不能拿任意旧 dist 代替。完整推荐镜像部署及网络说明以 [根 README](../../README.md) 为准。监控和加密恢复工具尚属后续阶段，不能以历史路径或本构建命令代替其运行验收。

## 验证

```sh
node --test deploy/native/tests/build.test.mjs tests/production-native-config.mjs
```

构建器测试对快照、参数、秘密排除、锁、失败拒绝和不可覆盖输出使用受控假编译器，**只证明构建控制流**。真实构建必须另外运行上述 `build.mjs`；镜像、Nginx、HTTPS、Rust 传输和 Compose 验收由 Phase 3 独立证据记录。

既有 `tests/production-native-e2e.mjs` 是单独启用的完整生产联合/恢复测试，不由构建命令自动执行。除其隔离 PG、Nginx、宿主产物、浏览器、TLS/NSS 与 PostgreSQL 客户端前置外，Phase 25 起还须预先构建并显式指定 Rust 管理 CLI：

```sh
cargo build --manifest-path tools/platform-admin/Cargo.toml --locked
export PLATFORM_ADMIN_BIN=/absolute/path/to/saas-platform-admin
node --test tests/production-native-e2e.mjs
```

`PHASE25_CLI_BIN` 可作同义输入，在导入工单 fixture 前规范化为 `PLATFORM_ADMIN_BIN`；指定路径必须是本机预构建二进制的绝对路径。缺少显式前置会报告跳过，已配置但无效会失败；测试不自动构建或回退旧 Node 管理实现。生产 fixture 的受控公开 `PRODUCTION_CA_FILE` 显式传给 CLI 的 `PLATFORM_CA_FILE`，保持 TLS 证书与主机名验证，私钥仍留在网关配置边界。
