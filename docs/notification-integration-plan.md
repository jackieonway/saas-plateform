# Phase 10：持久通知接入设计

> 状态：实施前设计存档。Phase 10 已完成，本文的“拟”与 SQL 为设计过程记录；最终实现以代码、[通知接入契约](./notification-integration.md)、[运维说明](./notification-operations.md)及 [Phase 10 验收](./phase10-validation.md)为准。
>
> 依据：[任务台账 Phase 10](./saas-microfrontend-task-ledger.md)、[SaaS 改造需求](./saas-microfrontend-requirements.md)、[应用授权上下文](./app-authorization-context.md)。

## 1. 范围与已确认规则

业务应用通过受权接口发布站内通知；宿主展示、查询和标记已读；实时服务负责提供新消息提示。`MicroProps.onNotify` 仍是纯界面 Toast，不写通知表，也不赋予发布权限。

用户已确认以下业务规则：

- 发布同时要求 `notification:publish` 操作权限与 `notification/publish` 独立数据授权。角色决定操作权限，数据授权决定收件范围。
- 收件人必须是当前租户的有效成员，且其接收组织可访问该应用。组织广播只取该组织的直接成员，不隐式广播子树。
- 幂等键按租户、发布组织、应用、发布用户和键隔离。同键同内容返回原结果；同键不同内容返回 HTTP 409。

迁移实现采用保留并隔离旧无归属通知的策略，不猜测租户、不复制到用户现有租户、不混入新收件箱。本文采用当前组织收件箱作为保守实现约定：一个用户在多个组织接收同一发布时，每个接收组织各有一条投递记录；列表、未读数、已读和删除操作限定当前 session 的用户、租户、组织。发布组织与接收组织分别记录，不能用发布者组织替代收件人组织。

短信、邮件、机器身份的无人值守发布、长期消息队列和历史消息自动归属不在本阶段范围内。正式 SLA 仍属于台账 Q7；下文超时、轮询及再验间隔是工程默认参数，不构成可用性或投递时限承诺。

## 2. 现有实现与改造点

| 位置 | 当前实现 | 本阶段拟改造 |
| --- | --- | --- |
| `backend/src/routes/notifications.rs` | `notifications` 只按 `user_id` 读写；发送要求 `system:user`，目标没有租户边界；循环写入失败会吞错误 | 增加归属、应用发布权限、目标验证、事务发布和幂等记录；全部收件操作带上下文条件 |
| `backend/migrations/0004_notifications.sql` | 通知表只有用户、类型、内容、链接、已读状态和时间 | 以新迁移增量添加归属，不修改 0004～0006 |
| `mini-services/notification-ws/index.ts` | 写死数据库地址和 Linux 密钥文件；本地验证 JWT；房间只按用户区分 | 环境配置；内部 HTTP 验证平台 session；租户/组织/用户房间；逐次投递和定时再验 |
| `frontend/src/hooks/use-notification-socket.tsx` | 固定 `XTransformPort`；组织切换开始时未断旧连接；重连不立即补拉 | 统一 Socket.IO 路径；token/epoch/切换状态共同控制连接；认证成功后刷新数据库视图 |
| 通知中心、铃铛、通知 API types | 查询键缺上下文；已有列表/计数/已读/删除/偏好界面 | 保留既有交互，查询绑定捕获 token 与 signal，并隔离缓存 |
| Compose、Caddy、Vite | Compose 未编排实时服务；生产网关未配置同构转发 | 增加服务、配置、健康检查和 `/socket.io/` 转发 |

## 3. 应用发布接口

### 3.1 请求 DTO

拟新增：`POST /api/apps/{appCode}/notifications`。

认证：`Authorization: Bearer <应用专用 token>`。普通平台 session、credential ticket、refresh token、旧 client_credentials access token 均不能作为此接口的应用身份。URL `appCode` 必须与 token 的 audience 一致。

```ts
type NotificationType = "INFO" | "SUCCESS" | "WARNING" | "SECURITY"

type NotificationTarget =
  | { kind: "USER"; userId: string; organizationId: string }
  | { kind: "ORGANIZATION"; organizationId: string }
  | { kind: "TENANT" }

interface NotificationPublishBody {
  idempotencyKey: string
  target: NotificationTarget
  type: NotificationType
  title: string
  content?: string
  link?: string
}

interface NotificationPublishResponse {
  publicationId: string
  recipientCount: number // 不重复的用户数
  deliveryCount: number  // 用户×接收组织的投递记录数
  duplicate: boolean
  createdAt: string      // 首次发布的 RFC 3339 时间
}
```

示例：

```json
{
  "idempotencyKey": "ticket:7301:assigned:v1",
  "target": {
    "kind": "USER",
    "userId": "00000000-0000-0000-0000-000000000101",
    "organizationId": "00000000-0000-0000-0000-000000000201"
  },
  "type": "INFO",
  "title": "工单已分配给你",
  "content": "请查看工单 7301。",
  "link": "/micro/ticket-center/tickets/7301"
}
```

约束：

- 租户、发布组织、应用、发布用户全部从服务端 token 绑定取得；DTO 不接受这些身份的覆盖字段。
- `target` 是有判别字段的严格联合，拒绝未知字段和混合目标。USER 显式指明接收组织，消除同一用户多组织成员关系的歧义。
- `idempotencyKey` 拟限制为 1～128 位 ASCII `[A-Za-z0-9._:-]`，大小写敏感。使用业务事件键或 UUID，不使用 token、密码等秘密。
- `title` trim 后非空，最多 200 个 Unicode 字符；`content` 默认空串，拟限制 8192 个 Unicode 字符；`link` 默认空串，最多 300 个 Unicode 字符，与现有数据库列一致。字符数与 UTF-8 字节数应分别处理。
- `link` 只允许当前应用 `/micro/{appCode}` 内的路由。禁止外部 URL、协议相对 URL、反斜线和控制字符；规范化后以及解码后的路径都必须仍属于该应用。
- 请求体拟上限 64 KiB；单次发布拟上限 1000 条投递记录，超限在入库前拒绝，不截断为部分成功。该容量属于可配置工程约束，不能扩大授权范围。

成功首次发布和合法重试均返回 HTTP 200；客户端根据 `duplicate` 区分重试。`recipientCount` 和 `deliveryCount` 的值与首次发布相同，不根据重试时新增成员变化。

### 3.2 授权与目标矩阵

每次请求，包括重试，都应重新检查发布者的应用 token、平台 session、当前成员关系、租户/组织有效状态、应用两级授权和当前功能/数据授权。匹配到已提交幂等记录时不重新枚举收件人：收件成员在首次发布后退出，不把原结果改成零，也不重建通知；发布者的权限或 grant 被撤销则仍拒绝重试。

| 数据 grant | USER 目标 | ORGANIZATION 目标 | TENANT 目标 |
| --- | --- | --- | --- |
| 缺 grant | 拒绝 | 拒绝 | 拒绝 |
| SELF | 仅发布者本人、当前组织 | 拒绝 | 拒绝 |
| ORG | 当前组织的有效直接成员 | 仅当前组织直接成员 | 拒绝 |
| ORG_SUBTREE | 当前组织或有效子组织的直接成员 | 允许指定范围内一个组织的直接成员 | 拒绝 |
| TENANT | 当前租户有效组织的直接成员 | 当前租户指定组织的直接成员 | 当前租户符合条件的全部成员 |

收件组织必须属于 token 的租户，且 `org_chain_active()` 成立；租户、用户与应用应启用且未到期。应用目标还应同时命中 `tenant_applications` 和对应收件组织的 `organization_applications`。组织广播的接收者由 `memberships` 直接关系取得，不从角色继承推导成员资格。

USER 指定的用户/组织不存在或不属于允许范围时采用不暴露存在性的拒绝响应。组织/租户广播只解析合规成员；结果为空则返回 400，说明没有可接收的成员。TENANT 广播按不同 `(organization_id, user_id)` 形成投递记录，用户数另行去重。

祖先组织角色可以提供操作权限，但不能替代或继承独立的 `notification/publish` 数据 grant。超管的应用 token 也不能因超管身份自动获得未声明的数据范围。

### 3.3 错误响应

沿用项目 `AppError` 的 JSON 错误结构，不返回 token 或客户端密钥。

| HTTP | 场景 |
| --- | --- |
| 400 | DTO/类型/长度/链接/键非法，混合目标，空收件集合或超出容量 |
| 401 | token 无效、类型错误、撤销、到期或其 session/成员上下文失效 |
| 403 | 错误应用受众、缺发布权限、缺数据 grant、请求的目标类别不受 grant 支持 |
| 404 | 单一用户/组织目标不存在或不属于允许范围，避免跨租户存在性泄露 |
| 409 | 幂等命名空间内同一键对应不同规范化请求 |
| 500/503 | 数据库或必需依赖失败；不得把部分写入伪装为成功 |

## 4. 幂等与事务边界

规范化请求包含 `target/type/title/content/link`，使用固定字段与规范化字符串生成 SHA-256；幂等键由唯一约束单独管理，不把 token、到期时间、contextEpoch 或重试次数加入请求哈希。实现应以服务端确定顺序构造规范化 JSON，不依赖请求 JSON 的原始键顺序。

命名空间为：`tenant_id + publisher_organization_id + application_id + publisher_user_id + idempotency_key`。

请求拟在单一数据库事务内完成：

1. 校验发布者身份、权限及可声明的目标范围，生成规范化请求哈希。
2. 按完整幂等命名空间取得事务级 advisory lock，再读取原发布记录；存在时比较哈希，合法重试直接返回首次结果。唯一约束仍是最终一致性门槛；锁哈希碰撞只增加串行等待，不能把不同命名空间认作同一次发布。
3. 没有原记录时才查询并冻结当前合规接收集合，插入发布记录和通知行。
4. 写首次发布审计及每条通知的 `pg_notify` 提示。
5. 提交后返回发布 ID 和固定结果。

任何错误都回滚本次事务。并发同键请求应只产生一次发布和一份接收集合；并发失败请求不能留下永久 PENDING 占位。身份和目标必须绑定本次捕获的上下文，不在处理过程中从全局可变状态替换 token。

删除收件通知只删除收件行，不删除发布记录。首版不设置自动幂等 TTL；后续若建设数据保留策略，应同时制定业务键重用规则。发布组织、租户或应用被显式删除时对应域数据可按既有级联删除语义清理；此后旧身份不能通过 token 验证，不能重试旧发布。

实时 NOTIFY 只是提示。数据库提交是持久发布成功的依据；NOTIFY 消费者离线不能把已提交发布改成失败，也不能让客户端盲目生成新键重发。

## 5. 拟新增迁移与 schema

拟新增 `0022_notification_context_and_publications.sql`，只作为后续实现命名建议。本文中的 SQL 是设计草案，尚未作为迁移文件执行。

### 5.1 发布记录

```sql
CREATE TABLE notification_publications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    publisher_organization_id UUID NOT NULL,
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    -- 发布者身份快照，不因账号删除而删除其他用户收到的通知。
    publisher_user_id UUID NOT NULL,
    publisher_username VARCHAR(50) NOT NULL,
    application_name VARCHAR(200) NOT NULL,
    idempotency_key VARCHAR(128) NOT NULL,
    request_hash CHAR(64) NOT NULL,
    request_body JSONB NOT NULL CHECK (jsonb_typeof(request_body) = 'object'),
    recipient_count INTEGER NOT NULL CHECK (recipient_count >= 0),
    delivery_count INTEGER NOT NULL CHECK (delivery_count >= recipient_count),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, publisher_organization_id, application_id,
            publisher_user_id, idempotency_key),
    UNIQUE (id, tenant_id, application_id),
    FOREIGN KEY (publisher_organization_id, tenant_id)
        REFERENCES organizations(id, tenant_id) ON DELETE CASCADE
);
```

复合外键复用 0013 已存在的 `organizations(id, tenant_id)` 唯一索引。发布记录没有公开修改接口；首次提交后的身份、请求哈希、请求正文和计数不可改写。实现可用数据库触发器保护不可变字段。发布者用户名和应用名为历史显示快照，应用/用户改名不重写历史消息。

### 5.2 通知投递行

保留 0004 既有 `id/user_id/type/title/content/link/is_read/read_at/created_at`。拟增加：

```sql
ALTER TABLE notifications
    ADD COLUMN source_kind VARCHAR(12) NOT NULL DEFAULT 'LEGACY',
    ADD COLUMN tenant_id UUID REFERENCES tenants(id) ON DELETE CASCADE,
    ADD COLUMN organization_id UUID,
    ADD COLUMN application_id UUID REFERENCES applications(id) ON DELETE CASCADE,
    ADD COLUMN publication_id UUID,
    ADD CONSTRAINT ck_notification_context CHECK (
        (source_kind = 'LEGACY' AND tenant_id IS NULL AND organization_id IS NULL
         AND application_id IS NULL AND publication_id IS NULL)
        OR
        (source_kind = 'APP' AND tenant_id IS NOT NULL AND organization_id IS NOT NULL
         AND application_id IS NOT NULL AND publication_id IS NOT NULL)
        OR
        (source_kind = 'SYSTEM' AND tenant_id IS NOT NULL AND organization_id IS NOT NULL
         AND application_id IS NULL AND publication_id IS NULL)
    ),
    ADD CONSTRAINT fk_notification_organization_tenant
        FOREIGN KEY (organization_id, tenant_id)
        REFERENCES organizations(id, tenant_id) ON DELETE CASCADE,
    ADD CONSTRAINT fk_notification_publication_context
        FOREIGN KEY (publication_id, tenant_id, application_id)
        REFERENCES notification_publications(id, tenant_id, application_id)
        ON DELETE CASCADE,
    ADD CONSTRAINT uq_notification_publication_recipient
        UNIQUE (publication_id, organization_id, user_id);

CREATE INDEX idx_notifications_context_inbox
    ON notifications(user_id, tenant_id, organization_id, is_read,
                     created_at DESC, id)
    WHERE source_kind <> 'LEGACY';
```

`organization_id` 表示接收组织；发布组织在 publication 中保存。APP 行必须具有完整归属；SYSTEM 行由可信平台内部调用显式给出接收上下文；LEGACY 行保持原始未归属状态。现有 `user_id REFERENCES users ... ON DELETE CASCADE` 仍按既有账号删除语义生效。

新代码必须显式传 `source_kind` 和归属，不能继续依靠旧 INSERT 默认值创建未归属新消息。迁移前预检应验证当前行数和既有字段可读；迁移后核对旧记录数、内容、已读状态均未变化。

### 5.3 旧数据隔离与内部系统通知

所有存量行初始化为 LEGACY。即使某用户现在只有一个成员关系，也不能据此推断历史通知属于该租户；未来新增成员关系不会改变 legacy 状态。

拟提供只读平台审计入口：`GET /api/notifications/legacy?page=1&size=20`。要求有效平台 session、平台租户上下文与既有 `system:audit` 权限；应用 token 和普通租户 session 均不得访问。该入口用于保留检查，不承担自动迁移、复制或广播。

旧 `notify(pool, user_id, ...)` 在用户创建、密码/角色变更、设备授权等调用点使用。后续应替换成要求显式接收租户/组织的内部写入接口，服务端业务代码负责选择真实受影响的成员关系。没有明确归属的全局账号事件先保留审计，不能任取用户第一个组织充当归属。若某系统事件要通知多个合法成员上下文，应显式枚举并审计，不能复用 legacy 推断逻辑。

内部 SYSTEM 写入与应用发布分开：可信内部调用没有公开 HTTP 绕过入口；应用不能把 `sourceKind: SYSTEM` 传给发布 DTO 获取豁免。

## 6. 收件箱接口与管理发送兼容

现有 GET 列表、未读数、已读、批量已读、删除、批量删除的路径可以保持。每个查询/更新必须同时限定：

```sql
user_id = 当前用户
AND tenant_id = 当前租户
AND organization_id = 当前组织
AND source_kind IN ('APP', 'SYSTEM')
```

批量操作对不属于当前上下文的 ID 不做写入；详情/单删使用不暴露存在性的 404。列表计数和分页使用完全相同的 WHERE 条件。查询保留 `created_at DESC, id` 的确定排序，已读字段仍使用原类型。

拟将现有 `NotificationRecord` 增加：

```ts
interface NotificationSourceFields {
  sourceKind: "APP" | "SYSTEM"
  tenantId: string
  organizationId: string
  applicationId: string | null
  publicationId: string | null
  applicationName: string | null
}
```

`notification_prefs` 仍是按用户保存的个人显示偏好，不推断为组织授权，也不用于决定消息能否入库。前端缓存应按用户/上下文隔离，防止另一登录账号复用旧偏好。

原 `POST /api/notifications` 不能继续通过 `system:user` 向任意目标发送。最终实现让通知管理界面选择明确应用，并为平台 session 增加兼容适配：DTO 增加 `applicationId` 与 `idempotencyKey`，目标及消息字段统一使用发布 DTO；服务端根据 ID 取得稳定 appCode 并构造该应用的授权视图，同样验证 `notification:publish` 和独立 grant，再调用同一事务管线。不得把平台 session 当作应用 token 解码，也不得自动给系统管理员授予 TENANT 范围。

当前 `NotificationSendBody` 的 `userId/orgId/tenantId` 旧联合应由前端改成严格 target DTO。TENANT 不再接受其他租户 ID；管理界面显示当前租户。第一版不提供跨租户广播。兼容发送响应可以保留 `sent = recipientCount`，同时附带 publicationId/duplicate/deliveryCount，避免现有提示语失真。

## 7. 平台 session 的实时校验协议

### 7.1 内部 HTTP 协议

拟新增：`GET /api/notifications/socket-context`。

- 使用 `CurrentUser` 完整验证平台 session；只有 `typ=session` 可通过。
- WS 用捕获的 session Bearer 调用。平台 endpoint 不接收 user/tenant/org 覆盖参数。
- 返回 DTO：

```json
{
  "contractVersion": "1.0",
  "userId": "00000000-0000-0000-0000-000000000101",
  "tenantId": "00000000-0000-0000-0000-000000000301",
  "organizationId": "00000000-0000-0000-0000-000000000201",
  "sessionId": "00000000-0000-0000-0000-000000000401",
  "expiresAt": "2026-10-02T14:00:00Z"
}
```

`expiresAt` 取 JWT 与 platform_sessions 到期时间的较早者。成员移除、密码版本变化、踢出、租户/组织停用或到期沿用 `session::active_user`，下一次校验拒绝。该接口无需业务发布权限，因为读取自己的通知本来就不要求发布权。

WS 不持有 JWT 签名密钥，不读取 backend `.env`，不本地相信 JWT 自称的身份。`PLATFORM_API_BASE_URL` 由服务部署配置指定，仅允许 HTTP(S)，请求不跟随 redirect；内部网络受限，Bearer 不写日志。拟单请求超时 2000 ms、响应上限 4 KiB；网络失败、5xx、未知 contractVersion、字段缺失、过期均停止该次投递。

### 7.2 Socket.IO 事件

保留当前事件式认证，统一 path 为 `/socket.io/`：

| 方向 | 事件 | DTO/行为 |
| --- | --- | --- |
| 宿主 → WS | `authenticate` | `{ token: string }`；仅允许初次认证，拒绝用同一连接切换身份 |
| WS → 宿主 | `authenticated` | `{ contractVersion, userId, tenantId, organizationId, expiresAt }`；不回传 token |
| WS → 宿主 | `unauthorized` | `{ code: "SESSION_INVALID", message: "实时通知会话已失效" }`，随后断开 |
| WS → 宿主 | `notification` | `{ contractVersion: "1.0", id, userId, tenantId, organizationId, applicationId, publicationId, type, title, content, link, createdAt }` |

房间拟命名 `tenant:{tenantId}:org:{organizationId}:user:{userId}`，只由平台验证结果生成。未认证连接拟 10 秒内关闭；认证后设置到期断开 timer。

监听 `new_notification` 的提示 payload 拟为 `{id, tenantId, organizationId, userId}`，不包含全文。WS 按这些绑定查询通知行，拒绝 legacy 或归属不一致记录；不能把未经数据库核对的 pg_notify 内容直接广播。

每次向某 socket 发送通知前，重新调用 socket-context，并确认返回身份/sessionId 与初次认证一致。任何变更或失败都不得使用旧正向结果继续发送。拟每 30 秒再验一次空闲连接，清除没有新通知时仍在线的失效会话。间隔是工程参数；安全边界来自每次实际投递前的实时验证，不以 timer 代替逐次验证。

最终内部 `socket-context` 返回 sessionId，公开 `authenticated` 不返回 sessionId 或 token。投递校验传 `?notificationId=<id>`，检查接收归属与 APP 当前应用授权；403/404 跳过该条，会话失效／身份变化断开。无参数的初次或定时校验不能确认时断开。APP 历史收件操作也要求当前两级应用授权和应用启用，SYSTEM 仅按有效接收上下文读取。

LISTEN 断开应重建监听并更新就绪状态。首版不要求重放全部错过的即时事件；不得用当前不稳定的 created_at 水位轮询宣称可靠重放。数据库查询、重连补拉与 60 秒收件数轮询是恢复依据。

## 8. 宿主连接与恢复规则

连接生命周期至少绑定 `token/contextEpoch/contextSwitching`：开始切换组织时立即移除事件监听并断开旧 socket，而不是等新 token 到达。回调再次核对捕获 token/epoch，旧 authenticated、notification 和异步读取结果不得回流。

认证成功或重连认证成功后：

1. 校验服务端返回 user/tenant/org 与当前 profile 一致。
2. 立即刷新当前上下文的数据库通知列表和未读数；刷新个人偏好。
3. 仅将连接成功后收到且属于当前上下文的新事件用于铃铛动画/Toast。
4. 相同通知 ID 的重复实时事件只触发一次提示；保留现有免打扰、类型静音与防风暴节流。

不得把数据库恢复的离线列表批量补弹 Toast。未读数不通过实时事件加减推测，必须以数据库 API 响应为准。实时 ACK 不代表用户已读，只有原已读 API 改变 read_at。

查询键拟统一前缀 `['notifications', userId, tenantId, organizationId, contextEpoch, ...]`；偏好使用当前用户/epoch 键。API 方法应能传捕获的 bearer 与 AbortSignal；注销和切换时取消查询，防止旧请求在新账号/组织缓存下成功写入。

配置认证拒绝时停止当前连接的无效重试；token 改变再建连接。临时连接失败使用有上限的重连退避并保留数据库轮询。Socket.IO Origin 允许列表同时覆盖 polling CORS 与 WebSocket 握手，不能仅设置 `cors.origin` 就声称约束全部传输。

## 9. 部署、配置与健康

拟增加 notification-ws Dockerfile 与 Compose 服务，依赖 PostgreSQL 和 backend，不向宿主暴露数据库端口。服务从环境变量读取配置；不允许写死数据库 URL 或搜索个人开发机密钥文件。

| 参数 | 拟默认/约束 | 用途 |
| --- | --- | --- |
| `DATABASE_URL` | 必填 | 通知行查询及 LISTEN |
| `PLATFORM_API_BASE_URL` | 必填，容器示例 `http://backend:3001` | 平台 session 校验 |
| `NOTIFICATION_PORT` | 3003 | 服务监听端口 |
| `NOTIFICATION_SOCKET_PATH` | `/socket.io/` | WS/宿主/网关一致 |
| `NOTIFICATION_ALLOWED_ORIGINS` | 必填，精确 origin 列表 | polling 和 WebSocket 来源校验 |
| `NOTIFICATION_AUTH_TIMEOUT_MS` | 2000 | 内部 HTTP 验证超时 |
| `NOTIFICATION_AUTH_RECHECK_MS` | 30000，配置需设合理上下限 | 空闲连接再验，工程参数 |
| `VITE_NOTIFICATION_SOCKET_PATH` | `/socket.io/` | 前端构建时或运行时对应配置 |

Caddy/Nginx 将 `/socket.io/*` 转发给 notification-ws，保留路径；支持 WebSocket Upgrade 和 polling POST。Vite 开发代理使用同一路径，不再使用根路径 bypass 与 `XTransformPort`。`/api/*` 仍由 backend 处理。

拟提供 `/health/live`（进程存活）与 `/health`（就绪）。就绪响应检查配置已加载、数据库可查询、监听连接可用、平台校验 endpoint 可达；未携 Bearer 请求该 endpoint 返回预期 401 可用于无凭据的路由存活探针，不表示用户认证通过。依赖不可用时 `/health` 返回 503，并描述 DB/LISTEN/platform 的 UP/DOWN/UNKNOWN，不能固定报告 UP。日志不打印连接串、Bearer 或密钥。

数据库提交成功、实时服务健康、单个在线客户端实际收到事件分别记录，不合并成一个成功指标。投递重试与客户端补拉应避免形成二次持久发布。

## 10. 验收矩阵

以下均为待执行验证；不得据本文勾选 Phase 10 完成。

| 编号 | 场景 | 必需结果 |
| --- | --- | --- |
| AUTH-1 | 错误 audience、普通 session、旧 access、credential、refresh、撤销应用 token 发布 | 401/403，无 publication、通知或成功审计 |
| AUTH-2 | 有功能权限但无 grant；有 grant 但无功能权限 | 拒绝，不从角色推断范围 |
| AUTH-3 | SELF/ORG/ORG_SUBTREE/TENANT 分别发送合法/非法 USER、ORG、TENANT 目标 | 与范围矩阵一致 |
| AUTH-4 | 跨租户目标；无应用授权的接收组织；停用/到期租户、组织、祖先或用户 | 不产生越界投递 |
| AUTH-5 | 组织广播含子组织成员、一个用户属于多个接收组织 | 只取指定组织直接成员；多组织投递按上下文隔离 |
| IDEM-1 | 同 namespace/key/body 串行与并发重试 | 单一 publication、单一接收集合，返回首次 ID/计数 |
| IDEM-2 | 同 key 但更换标题、目标、类型、内容或链接 | 409，原数据不变化 |
| IDEM-3 | 同 key 跨发布者、应用、租户或发布组织 | 命名空间互不影响；各请求仍需授权 |
| IDEM-4 | 删除个人收件行后重试；首次事务中途失败后重试 | 前者不重建；后者可完整重试，不留下假成功 |
| DB-1 | 批量入库/审计/通知提示前注入失败 | 全事务回滚，不返回部分 sent |
| DB-2 | 旧用户属于 0/1/多租户，迁移前后比较旧通知内容与已读字段 | 旧行保留 LEGACY，不复制或猜归属；新收件箱/count/WS 不读取 |
| DB-3 | 隔离复合外键、重复收件行、缺归属 APP、伪造 SYSTEM 写入 | 数据库或 API 拒绝；内部豁免没有公开入口 |
| READ-1 | 单删、批量已读、已读全部包含另一租户/组织/用户通知 ID | 越界行不变化，count 与列表范围一致 |
| WS-1 | 初次无 token、access token、过期或撤销 session | 不入房间、不推送；认证等待有超时 |
| WS-2 | 用户同租户/跨租户多组织同时登录，广播另一房间 | 只有对应 tenant/org/user 房间收到 |
| WS-3 | 踢出/密码版本变化/成员移除/组织停用后再发布 | 下一次投递前校验拒绝；定时再验清理空闲连接 |
| WS-4 | WS/PG LISTEN 断线期间继续发布，随后服务及宿主重连 | 数据库通知存在；立即补拉后列表和未读数一致，不补弹历史风暴 |
| UI-1 | 切换开始时旧 socket 迟到事件，旧 API 在新上下文完成 | 不弹旧 Toast，不污染新缓存/count |
| UI-2 | `onNotify`、免打扰、重复实时 ID、已读操作 | Toast 不写库；显示偏好不阻止入库；重复提示受控；已读只由 API 改变 |
| OPS-1 | Compose 生产参数、健康探针、Caddy/Nginx polling 和 ws upgrade | 服务可启动、来源受控、依赖故障可诊断、不依赖开发机路径 |

后端矩阵应在隔离 PostgreSQL 中运行；实时服务矩阵应使用实际 Socket.IO 连接和平台 HTTP 校验，不能以伪造 JWT 验证结果替代。重连和切换场景应使用真实浏览器。测试凭据只从 Git 忽略的本机环境配置读取，测试报告不记录连接串。

## 11. 后续实施清单

1. 编写新迁移和通知服务模块，完成 legacy 保留与 scope 约束。
2. 增加应用发布、用户适配发布、平台 legacy 审计和 socket-context 路由，替换旧不带归属 notify 调用。
3. 改造 notification-ws 认证、房间、逐次再验、监听恢复和配置；增加单独生产启动脚本和容器。
4. 改造宿主通知 DTO、API、查询键、连接/恢复规则和管理发送表单。
5. 增加 Compose 与三类网关/开发代理配置。
6. 执行验收矩阵，记录实际证据；通过后再把台账 Phase 10 标记完成并提交该阶段。

本文是实现输入。任何字段或行为调整应同步发布契约、迁移/兼容说明和验收记录；不能仅更新设计文档就认定持久通知接入已完成。
