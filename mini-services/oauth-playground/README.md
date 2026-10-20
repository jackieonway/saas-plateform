# OAuth 演练

此示例由独立 Vite 页面和 Rust 流程 API 组成，用于验证平台已有 OAuth 契约。原 `index.ts` Node HTTP 后台与 `pages.ts` 内嵌页面已退役；不再占用工单 API 的 3004。此工程不新增生产登录体系，其他服务的迁移状态仍按架构台账维护。

| 工程 | 开发默认地址 | 独立依赖与运行 |
| --- | --- | --- |
| `frontend/` | `http://127.0.0.1:3010`，callback `/callback` | 本工程 `package-lock.json`，Vite dev/build/preview |
| `backend/` | `http://127.0.0.1:3011`，`GET /health` | 本工程 `Cargo.lock`，`oauth-playground-api` Rust 二进制 |

浏览器只访问页面同源 `/api/*`。开发由 Vite 代理到 API；生产由独立 HTTPS Nginx 入口代理到私网 API。客户端 secret、PKCE verifier、平台 token、设备码及 refresh 凭据只在 Rust 内存中；页面收到的是本会话的结果摘要及操作句柄，不把这些秘密放入 Vite、URL、localStorage 或 sessionStorage。设备确认所需的短期 user code 属于明确展示的信息。

## 流程与平台条件

| 流程 | 演练行为 | 平台登记条件 |
| --- | --- | --- |
| 授权码 + PKCE S256 | Rust 创建 state/verifier、固定 callback、一次性消费授权结果 | 机密客户端实际 ID/私有 secret、`authorization_code`、精确重定向 URI；此演练不支持 public authorization 客户端 |
| 设备码 RFC 8628 | Rust 保存 device code、约束轮询并由用户在平台确认 | 已允许 device grant 的客户端，按其类型配置 secret |
| 服务凭据 | Rust 交换 `client_credentials`、内省与撤销；机器 token 无用户身份 | 已允许该 grant 的机密客户端及私有 secret |
| 历史 password 兼容 | 默认 `OAUTH_PASSWORD_ENABLED=false`，请求明确拒绝；仅在隔离演练时显式开启 | 历史允许 password 的客户端，用户凭据只临时提交 Rust；不作为新业务登录方案 |

具体支持能力来自配置和真实平台结果。未配置的流程、平台拒绝的 scope/grant、机器 token 的 userinfo 都保留明确错误，不返回模拟成功。refresh、userinfo、introspect、revoke 使用 Rust 内存中的 token 句柄，不接受任意资源代理 URL。

state/verifier 与发起会话绑定，有容量与 TTL 上限，回调一次性消费；错误 state、重放、过期及其它发起会话都会拒绝。API 重启或多副本切换使进程内会话/流程失效，应重新发起；本模板为单副本，不宣称共享状态或自动恢复未知交换结果。OAuth code/state 会出现在标准授权与 callback URL，callback 页面读取后立即清除查询串；verifier、secret、access/refresh token 不出现在 URL。

## 本地运行

使用仓库固定的 Rust 1.97.0、Node 24.18.0 与 npm 11.16.0。独立安装前端：

```powershell
npm --prefix mini-services/oauth-playground/frontend ci
```

Rust 不自动读取 `.env`。在启动进程的私有环境中设置下面参数；客户端 ID 必须使用平台注册产生的真实 ID，示例没有固定客户端或默认秘密。scope 默认 `profile`，可按客户端实际许可设置 `OAUTH_SCOPE`。与平台联调前须在平台登记精确 callback `http://127.0.0.1:3010/callback`。

```powershell
$env:OAUTH_PUBLIC_ORIGIN='http://127.0.0.1:3010'
$env:OAUTH_PLATFORM_ORIGIN='http://127.0.0.1:3000'
$env:OAUTH_ALLOW_INSECURE_LOOPBACK='true'
$env:OAUTH_AUTH_CLIENT_ID='YOUR_REGISTERED_PKCE_CLIENT_ID'
$env:OAUTH_AUTH_CLIENT_SECRET='YOUR_PRIVATE_AUTH_CLIENT_SECRET'
$env:OAUTH_DEVICE_CLIENT_ID='YOUR_REGISTERED_DEVICE_CLIENT_ID'
# 其它流程的 ID/secret 仅在需要时配置；完整键见 deploy/api.env.example。
npm run dev:oauth
```

`npm run dev:oauth` 与组件目录的 `npm run dev` 都由根 supervisor 启动 Rust API 和此工程 Vite；先等待 API `GET /health` 成功，再等待页面 HTTP 成功。Ctrl+C 只回收本次拥有的进程树；端口占用会失败并保留原监听者，不启动 PostgreSQL 或平台，也不自动安装依赖。

```powershell
npm run dev:oauth -- --host ::1 --oauth-port 4310 --oauth-api-port 4311 --timeout-ms 180000
```

上述调度器令 `OAUTH_BIND_ADDR`、`OAUTH_PUBLIC_ORIGIN` 和 Vite 代理自动跟随选定地址/端口；平台登记的 callback 也必须改为 `http://[::1]:4310/callback`。首次 Cargo 构建可能需要增加总 readiness 时限。HTTP 只允许显式开启的 loopback 开发；绑定非 loopback 地址不会放宽公开 origin 与平台 origin 的 HTTPS 规则。

分别运行时，Rust 使用 `OAUTH_BIND_ADDR`（默认 `127.0.0.1:3011`）、`OAUTH_PUBLIC_ORIGIN`、`OAUTH_PLATFORM_ORIGIN` 和私有凭据；Vite 的 `OAUTH_PLAYGROUND_API_TARGET` 为仅供 dev/preview 代理的 server 配置，默认指向 `http://127.0.0.1:3011`，`OAUTH_PLAYGROUND_FRONTEND_PORT` 默认 3010。它们都不是 `VITE_*` 变量。

```powershell
cargo run --locked --manifest-path mini-services/oauth-playground/backend/Cargo.toml
npm --prefix mini-services/oauth-playground/frontend run dev
npm --prefix mini-services/oauth-playground/frontend run build
```

## 独立镜像与 Linux HTTPS 部署

构建上下文固定为本组件目录，Dockerfile 仅复制各自的工程；`.dockerignore` 排除 `.env*`、`*.env`、证书、私钥、node_modules、dist 和 target。不要把运行私有配置转换成构建参数，尤其不能使用 `VITE_*SECRET`。`SOURCE_REVISION` 必须是已审阅源码的完整 40 位 Git SHA，镜像标签记录该 SHA；部署前另记录包含未提交补丁时的实际源码摘要和 OCI digest。默认基础镜像有固定版本，可通过 `NODE_IMAGE`、`NGINX_IMAGE`、`RUST_IMAGE`、`RUNTIME_IMAGE` 替换为已批准 digest。

```sh
docker build -f mini-services/oauth-playground/deploy/Dockerfile.frontend \
  --build-arg SOURCE_REVISION="$SOURCE_REVISION" \
  -t oauth-web:"$SOURCE_REVISION" mini-services/oauth-playground
docker build -f mini-services/oauth-playground/deploy/Dockerfile.api \
  --build-arg SOURCE_REVISION="$SOURCE_REVISION" \
  -t oauth-api:"$SOURCE_REVISION" mini-services/oauth-playground
```

静态镜像运行用户为 101，API 为 10001，运行层没有 Node/Bun。静态 Nginx 只提供真实首页、callback HTML 与 assets；缺资源、未知页面及 API 不回退到首页。两个镜像均可只读运行，PID/临时目录使用 `/tmp`。access log 关闭，避免记录 callback 查询串；API 本身也只记录固定类别诊断。

1. 为此演练准备独立域名与 TLS 证书，将已审核内容从 `deploy/api.env.example` 写入源码目录外的私有 `/srv/oauth/secrets/api.env`。生产 `OAUTH_PUBLIC_ORIGIN` 必须为精确 HTTPS origin，`OAUTH_PLATFORM_ORIGIN` 为平台 HTTPS origin；保持 `OAUTH_ALLOW_INSECURE_LOOPBACK=false`。平台 authorization 客户端重定向 URI 设为同一 origin 的 `/callback`。
2. 根据 `deploy/edge.example.json` 创建非秘密入口配置 JSON，设置同一 `publicOrigin`；运行下面 renderer 输出新的 Nginx 配置。renderer 拒绝 HTTP、userinfo、query/fragment/path、任意 upstream URL 和未知键，不覆盖已有审阅文件。
3. 将 `deploy/compose.env.example` 保存到私有运维目录，替换全部镜像占位符为批准 digest，核对绝对证书/配置路径及 UID 101 的只读权限。只有 `oauth-edge` 发布 80/443；`oauth-web`/`oauth-api` 仅供私网 DNS 访问。API 只需访问平台，没有业务库或共享产物卷。

```sh
node mini-services/oauth-playground/deploy/render-edge.mjs /srv/oauth/config/edge.json /srv/oauth/config/nginx.conf
docker compose -f mini-services/oauth-playground/deploy/compose.yml --env-file /srv/oauth/compose.env config --quiet
docker compose -f mini-services/oauth-playground/deploy/compose.yml --env-file /srv/oauth/compose.env up -d
```

部署属于操作者明确选择的实际目标；配置检查不会启动服务。生产入口与已有根 Nginx 模板使用相同非 root/只读/私网模式，此独立 Compose 不修改平台部署和 TLS。已共用入口的环境可将 `/api/*` 与页面规则加入该独立演练 vhost，并加入受控网络，保留准确 public origin、私网 DNS 与唯一 HTTPS 发布入口。DNS 上游使用 Docker resolver 动态重解析，重建 API 或静态容器不依赖旧 IP。自定义公网 HTTPS 端口须同时进入 origin 与客户端 callback，不通过任意 Host 头推导。

## 验证

```powershell
npm --prefix mini-services/oauth-playground/frontend test
node --test mini-services/oauth-playground/tests/delivery.mjs tests/dev-runner.mjs
cargo test --locked --manifest-path mini-services/oauth-playground/backend/Cargo.toml
```

独立实际 Linux Docker/HTTPS 检查使用根 `scripts/ci/oauth-playground-delivery.mjs`，参数为 `--out-dir <全新绝对目录>`、`--source-revision <完整40位SHA>`，可选 `--tools-image <已审核工具镜像>`。它从不含私有配置和依赖缓存的公开源码副本构建两个候选镜像，使用仅本次拥有的私网容器、自签测试 CA 和正常 TLS 验证，保存命令退出码、源码摘要、镜像 ID、实际 HTTP 结果及脱敏日志；不发布镜像或操作生产。仅 `private/` 中的临时配置和 TLS 私钥不应上传为证据。正常结束会核对 owner 标签并回收本次容器、网络及 WSL lease，本地候选镜像保留供审阅。

上述 fixture/无数据库检查不替代真实平台验收。真实平台四种流程矩阵、state/会话拒绝、实际 Linux 镜像/HTTPS 以及产物/日志扫描结果由 [Phase 24 验证记录](../../docs/validation/phase-24.md)单独记录。缺少部署或客户端条件时保留实际限制，不继承历史阶段的 PASS。
