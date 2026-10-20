# Micro application SDK

本目录是公开协议、浏览器辅助层和生成产物的唯一源码入口。`release.json` 首次登记 SDK **1.0.0**，状态为仓库候选；没有发布 npm 包或编造更早的 SDK 发行历史。SDK 版本与 wire contract 版本分开管理。

| 登记输入 | 当前版本与用途 |
| --- | --- |
| SDK | 1.0.0，宿主、Vue/jQuery、工单与门店前端共同消费 |
| application context | 1.0，平台实时返回的授权视图 |
| MicroProps / iframe | 2.0，宿主注入与独占 MessagePort 协议 |
| notification | 1.0，发布、结果、收件箱、socket context 与实时 DTO |
| legacy 静态 bridge | 当前仓库已有、无 SDK 版本字段的 iframe 2.0 实际文件，单独保留兼容基线 |

`compat/legacy-iframe-2.0.js` 的 SHA-256 为 `fb041923bbcff457b4d8b559f42637c5ee74ebc9c34824a901ba87d60e74b42a`。Phase 6 起点的共享 bridge 与工单 vendor 字节原本一致；保留这份文件用于实际旧静态版本激活/回滚验证，不代表支持任意未登记脚本。明确声明的未知 SDK、context 或 iframe 版本会拒绝。不能因为版本较旧而保留缺少来源、nonce、epoch、取消或隔离能力的实现。

## 生成与校验

使用根工程锁定的 Node 24/npm 和 `frontend` 构建依赖，不需要额外运行时依赖：

```sh
npm --prefix frontend ci
npm run sdk:sync
npm run sdk:check
npm run test:sdk
```

`scripts/sync-micro-sdk.mjs` 从登记、`contracts/schema.json` 和 `src/` 生成：

- `generated/`：协议常量、标准 JSON Schema、TypeScript 类型与独立 Rust wire DTO。
- `releases/1.0.0/`：可独立复制的 ESM、classic、类型、schema、Rust 源码及逐文件摘要清单。
- 根兼容入口、工单 vendored bridge/ESM、Vue/jQuery classic SDK 和平台 Rust 副本。
- 门店独立 `vendor/micro-sdk/` 包含完整登记 release 的运行时、声明、协议、schema 和 artifact manifest；引用类型不依赖宿主源码或根 node_modules。

`--check` 不会修复或覆盖漂移，缺文件、字节差异、legacy 变动或当前版本目录的未登记文件均失败。CI 在安装后、构建前执行全量检查，不能先同步再掩盖漂移。`--scope sdk` 仅供只包含 SDK 的 frontend Docker 构建上下文；全量 CI/native 仍检查所有 vendor 与 Rust 副本。生成不删除旧版本目录或不明文件。

修改应先改登记、schema 或 `src/`，再同步并审查生成差异。当前 1.0.0 尚未外部发行；实际发行后的既有静态字节必须保留，升级应创建新的登记版本。主版本内保持 wire 兼容，破坏性升级的 90 天只是候选窗口，当前没有已冻结下线日期，也不降低安全要求。

## 浏览器使用

ESM/Vite 消费 `releases/1.0.0/index.mjs`，TypeScript 声明随产物解析；独立部署可整体复制该版本目录。classic 页面通过相对脚本加载 `iframe-bridge.js` 或同字节 `sdk.js`，得到 `window.MicroSdk` 和 `window.MicroIframe`。opaque iframe 的 classic 资源不依赖 module CORS。

```js
import { createContextClient, createNotificationPublisher } from './sdk/index.mjs'

// props 由宿主提供，包含应用 token、epoch、到期时间和 AbortSignal。
const client = createContextClient(props)
const data = await client.requestJson('/business-api/items')
const notifications = createNotificationPublisher(client)
// 对同一业务事件持久保留同一个 idempotencyKey；授权由后台执行。
// await notifications.publish({ idempotencyKey, target, type: 'INFO', title })
// 卸载时停止请求。
client.dispose()
```

React 可在 mount/effect 清理时 dispose；Vue 可在挂载时创建、生命周期 update 时调用 client.update(props)、卸载时 dispose。SDK 不读 localStorage/sessionStorage，不持有平台 session/client secret，也不自行刷新 token；它只消费宿主捕获的应用 token、epoch 和 signal。独立页面间的共享会话仍由后续 Phase 10 处理。

所有请求强制 `credentials:omit`，覆盖调用方的 cookie 配置。JSON 默认限额 1 MiB，通用响应上限 4 MiB；按流计算实际字节，不能用无界 `json()`/`text()` 回退。授权替换/过期、调用方取消和卸载都会拒绝旧请求；迟到响应不能交给新授权。

纯主题、路由和回调更新只在 epoch、应用凭据、两个到期时间、身份、完整权限/grants/菜单/组织上限及部署版本均未变时复用授权 controller。ESM client 还要求接受时的同一个宿主 signal 仍有效；iframe 从已有 wire 字段推导同一授权，不增加协议字段。展示更新保留在途 JSON/CSV、原到期时间和当前回调能力，真实授权替换仍立即取消。

缺少 lifecycle `update` 的应用收到纯主题更新时保留当前实例，新的主题在下一次真正重挂时生效；路由或授权替换继续串行卸载、重挂并取消旧实例信号。不能把这类旧生命周期或固定 legacy 静态脚本描述成支持增量路由更新。

`authorizationContextSignature` / `sameAuthorizationContext` 是这套字段比较的纯辅助函数；前者保存接受时的内存签名，避免调用方原地修改对象隐藏授权变动。签名含 token，禁止写入日志、遥测或存储，且不能当成认证凭据。未知字段拒绝复用，类型标记避免数组/对象/缺字段相互碰撞；signal 身份、取消和到期仍需分别检查。此修改更新未外部发行的 1.0.0 仓库候选，保留实际 legacy bridge 与其他历史静态版本，不宣布新的外部发行。

`MicroSdkError` 区分 UNAUTHORIZED、FORBIDDEN、CONFLICT、NETWORK、PROTOCOL、HTTP，提供安全 `status/code/requestId`。`toJSON()` 只返回公开错误 DTO 的白名单字段和固定消息，不保留原错误 cause、URL、响应正文或凭据。取消使用 `DOMException` 的 `AbortError`。`X-Request-Id` 仅接受 UUID；远程业务 CORS 需允许请求头并暴露响应头，不能为此扩大 origin、credentials 或授权范围。

## 能力与隔离

`detectBrowserCapabilities(mode)` / `assertBrowserCapabilities(mode)` 分别检查 client、iframe、host 实际使用的 API。client 要求 fetch/Headers、有限响应流 Response/ReadableStream、URL、AbortController、DOMException、UTF-8 编解码及安全 UUID；iframe 另要求 MessageChannel 和消息监听；host 还要求 AbortSignal.any/timeout、MutationObserver、iframe sandbox、Blob 和下载 object URL。缺能力时明确停止，没有 TRUSTED、同源或无界读取降级。

支持的浏览器产品目标仍为 Chrome/Edge 最近两稳定版，实际测试产品和完整版本以 [Phase 6 证据](../../docs/validation/phase-06.md)为准。未验证的旧稳定版、Firefox/Safari 不能仅凭能力检测宣称支持。nonce/精确 parent 与 origin、专属 MessagePort、epoch/到期取消、组织硬上限和下载 broker 均继续适用；iframe 不增加 allow-same-origin 或 allow-downloads。

## Rust 与授权边界

`generated/protocol.rs` 可作为纯 serde/serde_json 模块使用，不依赖平台 SQLx、业务模型、Axum 或数据库。平台自身在 context 输出边界消费同源 DTO。`contracts/fixtures/public-wire.json` 是 JS/schema/Rust 共用的非秘密 wire fixture。

组织上限缺字段、显式 null、有限列表必须保留三态：旧 context 缺字段只回退当前组织；null 只能来自平台实际根上下文；列表不得扩大现有授权上限。公开解析、schema 或 DTO 都不等于鉴权。Rust 业务服务仍须实时内省、校验 context 的受众/用户/租户/组织并限定 SQL。本阶段没有迁移工单、通知后台或改变这些授权语义。
