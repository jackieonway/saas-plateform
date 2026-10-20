# OAuth 演练 Rust API

这是独立的 Axum 服务，不引用平台业务 crate，不访问平台数据库。平台的身份、授权码、设备审批、撤销与实时失权仍由平台负责。此服务只保存有界、短期的演练会话并调用配置的平台 HTTPS API。独立构建与发布不需要根 Cargo workspace。

```powershell
# 在本目录执行；环境从私有进程配置/部署 env_file 提供，不自动读取 .env。
cargo build --locked --release
cargo run --locked
```

生产使用同一个公开 HTTPS origin 提供 Vite 静态页面并反向代理 `/api/` 到 Rust。开发前端默认 `http://127.0.0.1:3010`，Rust 默认 `127.0.0.1:3011`，Vite 代理 `/api/`。平台 origin 应是浏览器可访问的前端入口，支持 `/oauth/` 和 `/api/` 代理及 `/consent`、`/device` 页面。平台登记的回调必须精确等于 `OAUTH_PUBLIC_ORIGIN` + `/callback`。

| 私有环境设置 | 约束/默认 |
| --- | --- |
| `OAUTH_PUBLIC_ORIGIN` | 必填，页面公开 origin，不含路径、账号、query 或 fragment |
| `OAUTH_PLATFORM_ORIGIN` | 必填，唯一受信平台 origin；所有出站路径由代码固定 |
| `OAUTH_BIND_ADDR` | `127.0.0.1:3011`；容器设为 `0.0.0.0:3011` |
| `OAUTH_ALLOW_INSECURE_LOOPBACK` | 默认 `false`；仅显式 `true` 时允许 localhost/127.0.0.1/[::1] 的 HTTP，生产仍为 HTTPS |
| `OAUTH_SCOPE` | 默认 `profile`，必须与客户端注册 scope 一致 |
| `OAUTH_AUTH_CLIENT_ID` / `OAUTH_AUTH_CLIENT_SECRET` | 同时配置才支持机密客户端授权码 + PKCE S256 |
| `OAUTH_DEVICE_CLIENT_ID` / `OAUTH_DEVICE_CLIENT_SECRET` | id 开启设备流；public 客户端不配置 secret，机密设备客户端必须配置 secret |
| `OAUTH_CC_CLIENT_ID` / `OAUTH_CC_CLIENT_SECRET` | 同时配置才支持受控服务凭据演练 |
| `OAUTH_PASSWORD_ENABLED` | 默认 `false`；仅明确 `true` 时启用历史兼容演练，另需 PASSWORD_CLIENT_ID/SECRET |
| `OAUTH_PASSWORD_CLIENT_ID` / `OAUTH_PASSWORD_CLIENT_SECRET` | 仅私有配置；历史密码演练不用于新业务登录 |
| `OAUTH_SESSION_TTL_SECONDS` | 默认 1800，范围 1–3600，固定绝对会话期限 |
| `OAUTH_STATE_TTL_SECONDS` | 默认 300，范围 1–600 |
| `OAUTH_TOKEN_TTL_SECONDS` | 默认 3600，范围 1–3600；同时受上游 access 有效期和会话绝对期限约束 |
| `OAUTH_MAX_SESSIONS` / `OAUTH_MAX_STATES` | 均默认 200，范围 1–10000；容量满明确 503，不驱逐有效会话/state |
| `OAUTH_HTTP_TIMEOUT_SECONDS` | 默认 5，范围 1–15，单次演练操作累计 deadline |

这些设置不能放入 `VITE_*`、静态资产或浏览器存储。代码和示例没有真实客户端 id/secret 默认值。设置格式错误时只输出设置名称，不回显值。

GET `/health` 与 `/api/health` 返回进程健康状态。GET `/api/config` 创建或复用 HttpOnly、SameSite=Lax、Path=/ 的随机会话 cookie，生产附带 Secure 及 `__Host-` 前缀，返回 `publicOrigin`、`platformOrigin`、`scope` 与四个 `flows.<name>.enabled`。同一 cookie 只允许访问其自身各流程的状态；未知/伪造 cookie 不被接纳为预设 session id。

所有 POST 要求精确匹配公开 origin 的 `Origin`、`Content-Type: application/json`、已有会话 cookie，以及严格 JSON body（拒绝未知字段）。所有响应均 `Cache-Control: no-store`；无 permissive CORS。请求 body 最大 16 KiB，出站响应体最大 256 KiB，最多 32 个并发演练请求。

| 路径 | JSON body / 返回 |
| --- | --- |
| POST `/api/auth-start` | `{}` → `authorizeUrl`、`state`。URL 只有公开客户端信息、state 与 S256 challenge，无 verifier/secret |
| POST `/api/callback` | `{state,code? ,error?}` → token summary；错误/拒绝/交换失败同样消费一次 state/verifier |
| POST `/api/device-start` | `{}` → RFC 设备审批公开字段；device_code 只保存在会话，不返回浏览器 |
| POST `/api/device-token` | `{}` → summary；待审批 `authorization_pending`，过早轮询 `slow_down`，终态不恢复 device code |
| POST `/api/cc-token` | `{}` → summary |
| POST `/api/password-token` | `{username,password}` → summary；默认 `unsupported_flow` |
| POST `/api/refresh-token` | `{flow}` → summary；取走旧 token 对后交换，失败不恢复旧 refresh token |
| POST `/api/userinfo` | `{flow}` → 平台用户信息，CC 无用户时平台明确拒绝 |
| POST `/api/introspect` | `{flow}` → 平台内省；使用该 flow 的所属机密客户端，public 设备客户端明确 `unsupported_flow` |
| POST `/api/revoke` | `{flow}` → `{revoked:true,flow}`；先移除本地 token 对，再分别吊销 access/refresh，未知结果也保持本地失效 |
| POST `/api/external-menus` / `/api/external-roles` | `{flow}` → 平台资源结果；普通 OAuth token 不自动获得平台应用 token 的 aud/权限，拒绝结果如实体现 |
| GET `/api/verifier` | 始终 404，无按 state 枚举 verifier 的接口 |

`flow` 仅接受 `authorization_code`、`device`、`client_credentials`、`password`。成功 token 响应只有 `{granted:true,tokenType,expiresIn,scope,hasRefreshToken,flow}`。请求不得提交 access/refresh token、客户端秘密、verifier 或任意 URL。不存在从浏览器导入平台应用 token 的后门。

每个会话最多一个 PKCE state、一个设备状态、四个 token 槽。state 绑定发起会话、按 TTL 校验并原子消费；错误会话不会消费正确会话的 state。刷新、签发、轮询和撤销按 flow 排斥并发；操作 deadline 取消后按请求所有权释放占位，保留旧凭据失效，用户可以重新发起新流程。会话/进程重启会丢弃内存状态，须重新授权；部署多个副本需同源粘性路由，本演练没有共享 token 库。

出站使用 reqwest + rustls 的默认正常证书校验，不关闭 TLS、不跟随重定向，也不接受请求输入作为目标 origin/path。客户端禁用环境/操作系统的隐式代理，直接连接配置的平台 origin。仅透传白名单 OAuth 错误代码与固定说明，丢弃上游错误 body、URL 与底层错误详情。成功资源 JSON 会递归移除凭据字段。服务不启用 HTTP URL/body trace；运行日志只记录固定启动消息。

验证命令：

```powershell
cargo fmt --all -- --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo doc --locked --no-deps
```

`tests/security.rs` 走公共 router 验证浏览器安全边界；`tests/upstream_contract.rs` 使用隔离的本地假平台验证真实 HTTP 转发与故障边界。Phase24 的真实平台/PG/浏览器和生产镜像证据由仓库 `docs/validation/phase-24.md` 独立记录；假平台测试本身不代表真实平台验收。

依赖均沿用平台已采用的同类库：Axum/Tokio 处理 HTTP 与有界异步流程；reqwest 仅启用 rustls/json；serde/serde_json 为严格 DTO；url 只解析配置及固定路径；rand 提供 OS CSPRNG，sha2/base64 实现 PKCE S256；thiserror 表达无敏感值配置错误。tower 仅用于集成测试，没有平台业务 crate、SQLx 或根 workspace 依赖。
