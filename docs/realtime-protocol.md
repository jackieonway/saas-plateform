# Rust 实时协议选型（Phase 22）

Phase 22 的结论是 **SOCKET_IO_COMPATIBLE**：保留现有 Socket.IO 传输和业务事件，Phase 23 使用锁定的 `socketioxide 0.18.7` / `engineioxide 0.17.7` 实现通知服务。条件 Phase 37 **未触发**。可复核的实际命令、运行分类、源码摘要及限制见 [Phase 22 证据](validation/phase-22.md)。此结论只覆盖本次版本和下表协议能力。

## 版本与依赖边界

| 对象 | 固定版本 / 能力 | 依据 |
| --- | --- | --- |
| 宿主与聊天客户端 | socket.io-client 4.8.4 | 各自提交的 npm lock；测试实际加载这两份安装 |
| 客户端 Engine.IO | engine.io-client 6.6.7 / parser 5.2.3 | Engine.IO wire EIO=4 |
| 客户端 Socket.IO parser | 4.2.7 | Socket.IO wire protocol 5 |
| Rust Socket.IO 候选 | socketioxide 0.18.7，MIT，MSRV 1.94 | 两个独立 Cargo.lock；无需 v4/msgpack/state/tracing feature |
| Rust Engine.IO | engineioxide 0.17.7，MIT，MSRV 1.94 | socketioxide 直接依赖；不启用旧 EIO=3 |
| 工具 | Rust 1.97.0 / Node 24.18.0 / npm 11.16.0 | 工程固定基线 |
| 通知公开 DTO | contractVersion 1.0 | 当前宿主校验器，wire 版本保持 |

标准库和平台的 Axum 原生 WS 不实现 Engine.IO polling、upgrade、Socket.IO namespace/event 编解码，因此增加兼容库有明确必要性。Axum/Tokio/serde 沿用项目技术；chrono 只生成现协议所需 RFC3339 时间。测试 fixture 的 Tower dev dependency 仅用于内存 HTTP Origin 拒绝测试。未建根 workspace、共享业务 crate 或新消息中间件。

本次从锁定 Cargo metadata 核对许可证/MSRV，并向 OSV 查询两套 Rust lock 涉及的101个registry包，未返回已知漏洞；版本与查询时点见[实际证据](validation/phase-22.md)。

## 真实客户端支持矩阵

测试入口为 `npm run test:realtime`。所有服务 peer 是 Rust；Node 仅编译、驱动 HTTP/真实 Socket.IO 客户端和检查结果。通知测试直接加载宿主安装的 `socket.io-client`，并将收到的 DTO 交给当前 `isNotificationSocketContext` / `isRealtimeNotification`；聊天测试直接调用产品 `createChatSocket`。没有用手写帧模拟候选库成功。

| 场景 | 验证行为 | 结果 |
| --- | --- | --- |
| 强制 polling | connect → authenticate → authenticated → notification | 支持 |
| 强制 WebSocket | 相同事件与公开 DTO | 支持 |
| polling → WebSocket | 真实 Engine.IO upgrade 后仍能接收通知 | 支持 |
| 事件认证 | 首次显式 `{token}`；握手 auth 不替代事件 | 支持 |
| 错误与关闭 | 无效/已过期合成 token、非法类型/数组/额外字段/超长、超时、重复事件 | unauthorized 后 io server disconnect |
| 到期 | polling 与 WS 均到 acknowledged expiresAt 后断开 | 支持 |
| 网络恢复 | polling 与 WS transport close 自动重连，新连接重新 authenticate | 支持 |
| 主动拒绝 | polling 与 WS 服务端 disconnect 后不自动重连 | 支持 |
| 心跳 | polling 与 WS 均观察真实 ping/pong，再投递事件 | 支持 |
| 身份房间 | 不同用户；同用户不同租户；同用户同租户不同组织 | 独立房间，无交叉事件 |
| 来源 | 缺失/null/不同端口/scheme/恶意后缀拒绝 | polling 与 WS 均拒绝 |
| Upgrade 来源变化 | 允许的 polling SID 遭外来 Origin upgrade | upgrade 拒绝，原 polling 仍可用 |
| namespace | 未登记 namespace | connect_error |
| 聊天参考 | join/list/双客户端广播/leave、身份绑定、重连后重 join、test 扩展、严格对象/限额 | 当前客户端支持 |

宿主当前配置为 `transports: ["websocket", "polling"]`，没有开启 `tryAllTransports`。两种传输单独通过不表示初始 WebSocket 失败会自动轮询回退；保留现行为，不为取得兼容结论修改客户端配置。HTTP 收件箱补拉属于业务恢复，也不是 Socket.IO 自动 polling 回退。

## 保持的通知契约

通知使用 `/socket.io/`、namespace `/`。transport connect 后先发严格对象 `authenticate {token}`；认证成功事件包含 `contractVersion/userId/tenantId/organizationId/expiresAt`。token 与 sessionId 不对外发送。`notification` 仅含公开归属、application/publication ID、type/title/content/link/createdAt；SYSTEM 的两个应用字段为 null。

拒绝与到期使用 `unauthorized {code:"SESSION_INVALID",message:"实时通知会话已失效"}`，随后 Socket.IO server disconnect。宿主收到 unauthorized 会主动 stop；transport 中断则重新连接和认证，认证成功后通过 HTTP 补拉数据库事实。聊天使用独立根路径 `/` 和 join/message 业务事件，不接收平台会话。

## Phase 23 的必需输入与未验证边界

本次通知 peer 是 [loopback fixture](../tests/fixtures/socketio-rust/README.md)，只有合成 token、合成到期和内存房间。兼容结论没有证明真实平台 session、HTTP 鉴权、租户成员撤销、APP 权限、PG LISTEN、逐投递复验、分区调度、资源容量、真实 Nginx/TLS 或 UI cache 恢复。这些能力仍按 Phase 23/34 验收。

Phase 23 必须沿用已有业务语义：平台 `socket-context` 确认身份和 sessionId；只允许缩短到期；逐条 APP 403/404 丢当前提示，401/身份改变/到期断开；初始/周期无法确认则拒绝；绑定身份不能被二次认证替换。它还须验收真实外部校验延迟/取消/晚到成功竞态，不能将本次无外部 await 的重复事件测试作为该证据。

上述是 Phase 22 验证时的边界。Phase 23 已在 `mini-services/notification-ws` 实现独立 Rust 通知服务，移除旧 Node 运行入口；有界调度、最小 DB 权限、LISTEN 重连、真实平台复验、双页组织切换、HTTP 补拉与生命周期的独立证据见 [Phase 23](validation/phase-23.md)。本地验收不等于目标生产部署。Phase 37 未触发，也不声明已完成原生 WS 适配。

## 回退与复验

聊天参考可独立回退其源码与锁文件，无平台/业务数据库迁移。通知协议没有切换，现 Node 服务及宿主配置继续保留。后续升级 Rust 兼容库或产品客户端版本应重新执行上述矩阵，并重新记录支持结论；不能继承本次结果。若真实复验暴露关键不满足，再以证据重新评估 Phase 37；缺环境始终记为未运行或 BLOCKED。
