# 持久通知接入契约

## 接入前提

应用需要当前租户及组织两级授权、有效成员及应用专用令牌。发布者同时具有本应用菜单对应的 `notification:publish` 操作权限，以及当前组织成员的独立 `notification/publish` 数据 grant。祖先角色可提供操作权限，但不提供数据范围；超管也需要显式 grant。这里不新增同名 OAuth scope。

管理员在应用菜单中登记操作权限，按原流程授权租户、组织及角色，再在数据权限管理中为直接成员、应用、资源 `notification`、操作 `publish` 选择范围。OAuth 机密客户端关联应用，密钥留在业务后端；浏览器只拿短期应用 token。

`onNotify` 继续只弹界面 Toast。写入收件箱必须调用下文接口，两者独立。

## 应用发布 API

`POST /api/apps/{appCode}/notifications`，Bearer 为该应用独立受众的 app token。平台 session、refresh、credential ticket 和旧 client_credentials access token不能替代。

```json
{
  "idempotencyKey": "ticket:7301:reminder:v1",
  "target": {"kind":"USER","userId":"user-uuid","organizationId":"org-uuid"},
  "type": "INFO",
  "title": "工单待处理",
  "content": "请查看工单 7301",
  "link": "/micro/work-orders/tickets/7301"
}
```

目标严格三选一：

| kind | 附加字段 | 接收集合 |
| --- | --- | --- |
| USER | userId、organizationId | 指定组织的指定有效直接成员 |
| ORGANIZATION | organizationId | 该组织有效直接成员，不隐式扩展子树 |
| TENANT | 无其它身份字段 | 当前租户的合规成员／组织投递 |

目标须在当前数据 grant 范围内，组织有效、用户有效且两级可访问该应用；不能在 body 覆盖发布者、租户或应用身份。

| grant | 允许目标 |
| --- | --- |
| SELF | 本人、当前组织的 USER |
| ORG | 当前组织 USER／ORGANIZATION |
| ORG_SUBTREE | 当前组织及有效子组织 USER／ORGANIZATION |
| TENANT | 当前租户 USER／ORGANIZATION／TENANT |

接口拒绝未知字段和混合目标。键为 1–128 位 ASCII 字母、数字或 `._:-`；标题去首尾空白后 1–200 字符，正文最多 8192 字符，链接最多 300 字符且只能位于本应用 `/micro/{appCode}` 路由；类型为 INFO、SUCCESS、WARNING、SECURITY。body 上限 64 KiB，单次最多 1000 条投递。

成功返回 HTTP 200：

```json
{
  "publicationId":"publication-uuid",
  "recipientCount":1,
  "deliveryCount":1,
  "duplicate":false,
  "createdAt":"2026-10-02T11:00:00Z"
}
```

recipientCount 是不同用户数，deliveryCount 是用户×接收组织投递数。发布记录、收件行、审计及 NOTIFY 同事务提交；失败不能报告部分成功。

幂等命名空间为租户、发布组织、应用、发布用户和键。同键相同规范化正文返回首次记录与固定计数、duplicate=true；不同正文返回 409。每次重试仍验证当前发布权限和目标范围，不重新枚举已提交的收件集合。收件成员退出或用户删除通知不会使重试重建通知；收件组织失效或越出当前范围时仍拒绝。网络结果未知时保留同键同正文重试，不能生成新键重复发送。

## 浏览器 SDK 示例

使用已建立的 createContextClient 或 iframe bridge：

```js
import { createNotificationPublisher } from "./notification-publisher.mjs"

const publisher = createNotificationPublisher(client)
const event = {
  idempotencyKey: "ticket:7301:reminder:v1",
  target: { kind: "USER", userId: assigneeId, organizationId: props.organization.id },
  type: "INFO", title: "工单待处理", content: "请查看工单 7301",
  link: "/micro/work-orders/tickets/7301",
}
await publisher.publish(event)
```

辅助层使用当前应用 token、取消信号和 `credentials: omit`；权限、有效组织和目标仍以服务端为准。发布失败可用同一 event 重试。classic 页面也可直接通过 iframe bridge 的 requestJson 调同一路径，不需要引入构建工具。

## 宿主发送适配与收件操作

`POST /api/notifications` 使用平台 session，body 比应用接口多一个必填 applicationId；同样经过该应用的发布权限及独立数据范围校验。成功响应额外保留 `sent=recipientCount`。旧 userId/orgId/tenantId 发送 body 不再受理，也不以 system:user 自动赋予发布权限。

列表、未读数、已读、批量已读、删除和批量删除只操作当前 session 的用户／租户／组织。APP 行还要求当前应用启用且当前租户及组织仍授权；失去授权后历史内容和迟到投递不可见，原行保留。SYSTEM 行按有效收件上下文读取。个人通知偏好仍按用户保存，客户端缓存按完整上下文隔离。

0022 保留升级前无归属通知为 LEGACY，不猜测或复制租户。`GET /api/notifications/legacy` 仅平台上下文且具有 system:audit 的用户可只读审计；普通收件箱和实时服务不读取 LEGACY。

## 实时协议与恢复

统一 Socket.IO 路径 `/socket.io/`。初次 authenticate 只传 `{token}`。公开 authenticated 为 1.0、userId、tenantId、organizationId、expiresAt；不回传 token 或 sessionId。notification 为 1.0、id、完整接收归属、applicationId/publicationId、类型、内容、链接及 createdAt，不包含 HTTP 已读状态。

内部 `GET /api/notifications/socket-context` 验证平台 session，返回上述身份和 sessionId；实时服务在连接、每次实际投递及定时复验时调用。投递加 notificationId，平台检查本条归属和当前应用授权。通知特定 403/404 跳过该条，会话失效、过期或身份改变断开；无法确认时停止投递。

组织切换开始就中止旧 HTTP scope 并断旧 socket；旧事件、旧响应和旧 mutation 回调不能影响新组织。认证与重连立即补拉数据库列表、未读数及个人偏好，离线历史不批量弹 Toast，未读数以数据库 API 为准。部署、健康、LISTEN 恢复与实测命令见[运维说明](./notification-operations.md)。

## 错误响应

400 表示 body、目标、长度、范围内空接收集合或容量错误；401 表示无效／撤销／过期身份；403 表示错误受众、功能权限或数据授权不足；404 表示目标或通知不存在／不可见；409 表示同键不同正文。沿用平台 JSON 错误结构，日志不写 Bearer 或客户端密钥。
