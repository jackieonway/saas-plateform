# 实时通知协议

实时通知由独立 Rust 服务 `saas-notification` 提供 Socket.IO 传输。平台后端负责持久化通知、收件箱查询和会话可见性验证；通知服务通过受限数据库角色读取已提交通知，通过平台 HTTP 接口验证会话。qiankun 微应用发布通知使用应用专用令牌，平台壳负责当前用户的实时收件箱连接。

依据为 `saas-notification/src/{server,protocol,platform,store}.rs`、平台 `src/routes/notifications.rs`、`src/notifications.rs` 和前端 `src/hooks/use-notification-socket.tsx`。

## 连接与身份

默认 Socket.IO path 为 `/socket.io/`，可由服务端 `NOTIFICATION_SOCKET_PATH` 和壳 `VITE_NOTIFICATION_SOCKET_PATH` 对齐修改。服务端仅接受 `NOTIFICATION_ALLOWED_ORIGINS` 中的精确 Origin，缺失、重复或未登记 Origin 会拒绝。不要使用原生 WebSocket 客户端发送自定义裸帧替代 Socket.IO 客户端。

连接后，客户端只发送一次：

```javascript
socket.emit("authenticate", { token: platformSessionAccessToken })
```

payload 只能含 `token`，去除首尾空白后长度 1–4096，不能含内部空白或控制字符。客户端不提交 user、tenant、organization、session ID，不自行选择房间。重复 authenticate、无效凭据或超出认证等待时间都会终止连接。刷新 token 或切换身份应断开后新建连接。

通知服务使用这个 Bearer token 调用：

```http
GET /api/internal/notifications/context-proof
Authorization: Bearer <platform-session-access-token>
```

内部适配返回精确十一字段：`proofVersion="1"`、`contractVersion="1.0"`、`userId`、`tenantId`、`organizationId`、`sessionId`、`expiresAt`、`membershipId`、`memberType`、`identityVersion` 和 `accountVersion`。后两个版本是 JSON i64 整数，分别至少为 1 和 0。`expiresAt` 是平台会话和当前 JWT 到期时间的较小值。该端点要求实时有效的具体成员会话，应用 token 不作为平台收件箱身份。原公开 `/api/notifications/socket-context` 仍保留六字段 DTO，Notification 1 公共载荷不增加内部证明。

## 服务端事件

| 事件 | payload 与处理 |
| --- | --- |
| `authenticated` | `contractVersion`、`userId`、`tenantId`、`organizationId`、`expiresAt`；不向浏览器暴露 session ID |
| `notification` | 以下通知 DTO；客户端需校验契约和当前身份 |
| `unauthorized` | `{ "code": "SESSION_INVALID", "message": "实时通知会话已失效" }`，随后断开 |

```json
{
  "contractVersion": "1.0",
  "id": "019a1163-0000-7000-8000-000000000101",
  "userId": "019a1163-0000-7000-8000-000000000102",
  "tenantId": "019a1163-0000-7000-8000-000000000103",
  "organizationId": "019a1163-0000-7000-8000-000000000104",
  "applicationId": "019a1163-0000-7000-8000-000000000105",
  "publicationId": "019a1163-0000-7000-8000-000000000106",
  "type": "INFO",
  "title": "工单已更新",
  "content": "请查看工单详情",
  "link": "/micro/work-orders/tickets/019a1163-0000-7000-8000-000000000107",
  "createdAt": "2026-10-23T12:00:00Z"
}
```

`type` 为 `INFO`、`SUCCESS`、`WARNING`、`SECURITY`。`applicationId` 和 `publicationId` 对系统通知可以为 `null`。事件不携带 `isRead`、`readAt`、`sourceKind` 或凭据。浏览器不能据实时事件自行认定已读状态或未读总数。

## 投递链路与失效

通知写入事务提交后，PostgreSQL NOTIFY 携带精确八字段私有提示：`hintVersion="1"`、`id`、`userId`、`tenantId`、`organizationId`、`membershipId`、`memberType`、`identityVersion`。UUID 为规范 UUID v7，身份版本是正 JSON i64 整数。通知服务按具体成员和版本分区，从持久行重新读取 `APP` 或 `SYSTEM` 通知；提示不携带消息正文，旧四字段提示与无成员证明的历史行不参与实时投递。

每次实际投递前，服务端调用内部 `context-proof?notificationId=<id>`。平台用相同的收件箱可见性条件核验用户、租户、所选组织、具体成员类型与身份版本；应用通知还要求应用启用、允许当前成员类型，并且租户、组织仍授权该应用。成员 ID、类型、版本、session 或账号版本不能在同一连接改绑。周期重查失败、凭据到期、账号／成员／会话撤销会使连接失效；逐条投递依赖故障拒绝该条消息，不缓存旧成功结果继续授权。

默认认证 HTTP 超时 2000 ms、等待首个 authenticate 10000 ms、周期重查 30000 ms，并有小幅错峰。服务端不持有平台 JWT 签名密钥，配置中提供 `JWT_SECRET` 会导致启动失败；平台验证地址由 `PLATFORM_API_BASE_URL` 指定，禁止请求重定向。

## 恢复与前端行为

实时通道不提供持久事件重放或恰好一次投递保证。丢包、服务重启、队列饱和及断线通过 HTTP 收件箱恢复。当前壳在 authenticated/reconnect 后重新查询列表、未读总数和偏好；恢复出的历史通知不补弹 toast。

壳捕获完整用户/租户/组织/会话上下文，切换开始即中止旧连接，并检查迟到回调。实时通知按 ID 去重，未读数重新从数据库读取。免打扰及静音类型只影响提醒展示；相同标题在 10 秒内合并，10 秒窗口最多 3 个 toast。

HTTP 接口包含 `GET /api/notifications`、`/{id}`、`/unread-count`、`/prefs`，以及标记已读和删除操作。列表支持 `filter=UNREAD`、type、q 和分页。旧无成员证明通知只通过 `/api/notifications/legacy` 由具有 `system:audit` 的平台根员工上下文查看。偏好按成员和身份版本独立保存，不从旧账号偏好复制。通知修改在行锁等待后重新验证会话及受影响应用的可见性，失败回滚。

## 微应用发布通知

`POST /api/apps/{appCode}/notifications` 使用当前应用 Bearer 令牌，JSON 包含 `idempotencyKey`、`target`、`type`、`title` 及可选 `content`、`link`。target 的 `kind` 为 `USER`（userId、organizationId）、`ORGANIZATION`（organizationId）或 `TENANT`。

发布要求当前有效员工、应用员工准入、`notification:publish` 操作权限和具体成员的 `notification/publish` 独立数据授权，范围受所选组织硬边界限制。广播与旧 SYSTEM 接口只展开有效员工，客户发布用途保持关闭。idempotency key 为 1–128 位 ASCII 字母数字或 `._:-`，命名空间绑定发布成员和身份版本；标题最多 200 字、正文 8192 字、链接 300 字，单次投递上限 1000。`link` 只能为空或位于当前应用 `/micro/{appCode}` 内，编码绕过和跨应用跳转会拒绝。

成功响应包含 publication ID、recipient/delivery count、duplicate 和创建时间。幂等键相同但内容不同会冲突；通知持久化、投递记录与事务审计共同提交。平台会话发布接口 `POST /api/notifications` 另需明确 `applicationId`，不改变应用归属和独立授权规则。

## 验证入口

通知服务普通测试内嵌于各模块，独立 PostgreSQL／真实 binary 测试位于其 `tests/identity_isolation.rs`。平台 `src/notification_tests.rs` 保留原范围、幂等和事务合同，`src/notification_identity_tests.rs` 另覆盖双身份、旧结构升级、锁等待撤权及真实平台 HTTP＋外部通知 binary 联调。ignored 测试必须显式执行，源码覆盖不等于通过；身份绑定细节见[当前初始化与部署](fresh-installation.md)。部署参数、数据库最小权限及网关 path 配置结合[部署文档](deployment.md)与 `saas-notification/README.md` 核对。
