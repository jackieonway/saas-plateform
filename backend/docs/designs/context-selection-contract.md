# 统一身份选择协议 v1：实施合同

状态：核心校验、存储、交互创建/来源核验、查询/确认接口、认证中心分步页面与 code 扩展已加入；员工 BFF 已接入同库消费，客户门户已接入消费及原结果恢复。Phase 12 验证了 canonical `/oauth/*` 交互、账号/业务上下文分流、一次性结果恢复与来源代际栅栏。管理平台和客户门户的真实 HTTPS 登录/续期仍分别由 Phase 13/14 验收，不得据此声明完整登录流程已经可部署。

本合同细化[总体方案](centralized-context-selection.md)，是后续认证中心、平台与客户门户接入的共同约束。接口上线时，必须将此状态更新为实际验证范围，再纳入现行文档索引。

## 1. 编码与版本

合同版本固定为 `context-selection/1`。所有请求与响应使用 UTF-8 JSON，字段为 camelCase，拒绝未知字段和重复字段。标识使用规范小写、有连字符的 RFC UUIDv7；代际和会话版本使用无前导零的十进制字符串，范围为 `1..=i64::MAX`。客户端策略、应用策略、会话版本以及新账号与成员代际从 1 开始。

选择凭据和交互秘密均为 32 字节密码学随机数的 64 位小写十六进制编码。选择凭据由平台生成，平台只保存其 SHA-256 摘要；认证中心为了向后端 code 兑换结果交付凭据，只能短期加密保存原文。禁止将原文写入日志、审计、公开 URL 或浏览器持久化存储。

平台核心类型及判定见 [context_selection.rs](../../src/context_selection.rs)，回归测试见 [context_selection.rs 测试](../../tests/context_selection.rs)。这些类型只能判断已取得的权威证明是否匹配，不能代替调用方认证、在线来源核验或数据库锁。

## 2. RP 登记与服务用途

身份服务的 RP 登记新增统一选择能力标记；启用或停用都必须改变 RP 策略版本。`ACCOUNT_ONLY` 模式不启用业务身份选择。已启用 RP 在 code 兑换时必须携带完整选择扩展，缺失即拒绝建立业务会话；refresh 不产生该扩展。

平台持有对应 RP 的上下文登记：RP 标识、登记版本、用途、目标应用、允许成员类型和状态。员工控制台为 EMPLOYEE；客户门户为 CUSTOMER。统一 `AUTH` 客户端必须绑定目标应用和其策略版本；仅阶段性 `OIDC` 员工登记可不绑定单一业务应用。平台检查应用、租户应用及组织应用准入，客户端不能自行指定另一个目标应用。

反向私有调用分开登记服务用途：

| 服务用途 | 凭据拥有方 | 允许操作 |
| --- | --- | --- |
| `CONTEXT_SELECTION_AUTHORITY` | 认证中心 | 调用平台查询可选成员、确认并签发选择凭据 |
| `CONTEXT_SELECTION_SOURCE` | 平台 | 调用认证中心核验交互账号来源及消费来源 |
| `CONTEXT_SELECTION_CONSUMER` | 独立应用 BFF | 调用平台消费和恢复本人 RP 的选择结果 |

服务登记需限定允许的 RP 集合。员工平台 BFF 在进程内使用同一个平台用例，不通过共享服务密码伪装成外部 BFF。客户门户的消费凭据与原有本人资料资源凭据分开。全部用途与 OIDC RP secret、账号命令及 OIDC_ADMIN 凭据分开。

认证中心和平台都不接受浏览器 Cookie 作为私有接口认证。私有接口使用精确一个规范 Basic Authorization、独立登记的 UUIDv7 服务 ID 和随机秘密，验证状态、用途、RP 范围与凭据隔离；网关限制私有网络，客户端启用 HTTPS、CA 验证、禁止重定向和自动重试。

## 3. 发起交互

已实现 `POST /internal/identity/auth/context-interactions`，由 BFF 使用其 OIDC RP Basic 认证调用。此接口是已登记 RP 的后端端口，不能挂载到认证页面的公开浏览器调用路径。

请求包含 `contractVersion`、`transactionId`、`intent`、`redirectUri`、`stateHash`、`nonceHash`、`codeChallenge`。后四项绑定 BFF 随后发出的授权请求。平台保存返回路径，禁止把返回路径传入身份服务作为任意重定向目标。

`intent` 仅允许两个结构：

```json
{"kind":"LOGIN"}
```

```json
{"kind":"SWITCH","sessionId":"019a1163-0000-7000-8000-000000000007","expectedVersion":"1"}
```

响应包含短期、不透明的 `contextRequest` 引用与 `expiresAt`。BFF 将此引用作为 `/oauth/authorize` 的扩展参数提交。身份服务把它与本次 flow 原子绑定，逐项检查 RP、回调、state/nonce 摘要与 PKCE challenge；不能将同一引用绑定另一个 flow。确认使用的账号由该 flow 的当前 SSO 来源决定。

页面按需推进账号登录、应用授权、租户和组织选择。首次授权通过独立 CSRF `consent` 操作写入身份所有的 `auth_consents`；后端在账号锁内检查当前账号、RP 和 scope，已有未撤销的覆盖授权不再重复展示，退出登录/令牌过期/组织切换不清除记录。请求新增 scope 需要重新确认。授权有效前不得查询成员或提交 `membershipId`，确认授权时不签发 code。只有一个租户时直接展示组织；选定租户下只有一个可用成员身份时，页面自动通过相同的 CSRF `approve` 与完整服务端来源/成员校验继续，包括 SWITCH。搜索过滤不改变自动选择的真实候选数。`prompt=none` 仍不能绕过受管 RP 所需的服务端选择交互。

认证中心 `/sso/applications` 提供账号级授权管理；列表和撤销 API 在 `/api/sso/consents` 下。撤销与 code 签发使用同一个账号锁，原子撤销该账号对 RP 的持久授权、现有 grants 及未完成请求；旧标签页不能恢复授权。其他 RP 的持久授权保持有效。平台 runtime 不能直接读写此表。

## 4. 来源证明

认证中心为已绑定且已认证的交互创建短期来源证明引用。该引用只通过服务通道交付平台，不能依赖可猜测 flow ID 作为秘密。

已实现 `POST /internal/identity/auth/context-selection/source`。平台以 CONTEXT_SELECTION_SOURCE 身份发送合同版本、来源证明引用和预期 RP。认证中心核对原 flow、SSO 会话、账号代际、RP 策略和有效期后，返回：

- 账号来源：`issuer`、`userId`、`accountVersion`、`globalSessionId`、`interactionId`、`authTime`。
- 原 RP 登记与回调、本次 `transactionId` 和 `intent`。
- 来源及交互的原始截止时间。

在 code 兑换后消费阶段，该操作还需要验证原 RP 的 access token 与同一 interaction 派生的原始 grant。仅有 ID token 或 refresh token 不构成消费来源。不能接受浏览器上报的 userId、sessionId 等字段作为来源证明。

请求有两个互斥模式：选择前使用 `sourceProof` 与 `expectedClientId`；消费时使用 `transactionId`、`oidcAccessToken` 与 `expectedClientId`。消费方不需要取得绑定认证中心浏览器的来源秘密。两种模式不能混传；事务 ID 自身不构成授权，消费模式必须证明与此交互关联的原 grant。

此核验处理器只能读取身份域账号、SSO、OIDC 与交互数据，不调用平台；因此不会形成“平台查来源 → 身份服务查成员 → 平台查来源”的循环。

## 5. 查询与确认

拟新增平台私有接口：

- `POST /internal/platform/context-selection/choices`：认证中心提供合同版本和来源证明引用；平台在线解析权威来源，验证 RP 登记并返回当前有效成员及展示信息。
- `POST /internal/platform/context-selection/confirm`：同一来源引用与所选 `membershipId`；平台重新核验成员、租户、组织链、成员类型及应用资格，生成一次性凭据。

确认时不允许用账号、组织、租户或成员类型覆盖来源。DTO 只返回选择所需的名称、组织路径、身份类型和标识。不得返回其他账号的成员、完整角色菜单或员工/客户用途之外的信息。

平台保存不可变绑定：transaction、issuer、账号及代际、SSO session、OIDC interaction、RP 及策略、精确回调、应用及策略、intent、原 BFF session/version、成员及代际。确认有效期为 `min(数据库当前时间+120秒, 原来源截止, 原交互截止)`。

## 6. code 扩展交付

OIDC code 消费事务需绑定选择结果，后端 token 响应新增独立的 `context_selection` 字段：

```json
{
  "contractVersion":"context-selection/1",
  "transactionId":"019a1163-0000-7000-8000-000000000001",
  "credential":"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
}
```

标准 `id_token`、`access_token`、`refresh_token` 与既有 OIDC 校验继续生效。选择结果不能成为另一个账号的凭据，也不能由 refresh 响应重新生成。浏览器回调只携带原有 code/state/issuer 或标准错误字段。

code 重放沿用现有拒绝/撤销规则，不再次交付有效选择授权。其他 RP 未启用扩展时保留标准 OIDC 行为。

## 7. 消费、提交与恢复

拟新增 `POST /internal/platform/context-selection/consume`，独立 BFF 以 CONTEXT_SELECTION_CONSUMER 认证调用；员工平台使用同一消费用例的进程内适配器。

消费方提交合同版本、事务 ID、选择凭据和原 RP 的 access token；SWITCH 还必须证明原 BFF session/version。平台在线核验原始账号/interaction/grant，重新读取当前登记及成员，把完整绑定与确认时的原始值逐项比较。任何代际变化都拒绝，不能修补绑定后继续。

在同一个平台事务内完成凭据消费、旧上下文撤销、新上下文建立与审计。选择表必须对凭据摘要和交互结果建立唯一性约束。按现有锁顺序锁定账号、RP/应用、成员与会话，同一旧版本只能有一个切换提交。

消费结果按 RP 用途区分：员工结果只关联员工业务会话；客户结果只关联客户专用上下文及原有客户资源证明。每个结果保存新上下文引用和版本，不将可直接使用的明文 token pair 存入通用 JSON。

拟新增 `POST /internal/platform/context-selection/result`：只允许原消费服务、RP、事务、原始来源以及原 BFF session/version 查询已提交结果。查询不能重新消费凭据、建立另一个上下文、重新签发 token pair 或延长有效期。还需校验已提交的新上下文仍然有效。

恢复期限不晚于原选择凭据截止时间。跨库门户先完成平台提交，再保存本地加密会话；本地失败期间冻结交互，通过结果恢复完成保存，超时后要求重新登录。员工平台优先在同库事务内完成全部更新。

## 8. 错误与生命周期

| HTTP 状态 | 稳定错误标识 | 行为 |
| --- | --- | --- |
| 400 | `invalid_selection_request` | 编码、版本、字段、组合或绑定格式错误 |
| 401 | `invalid_selection_source` | 人员认证来源、grant 或账号代际失效 |
| 403 | `selection_not_allowed` | 服务身份、RP 用途、成员或应用准入不匹配 |
| 409 | `selection_changed` | 原会话版本、策略代际或已确认成员状态变化 |
| 409 | `selection_consumed` | 一次性选择已消费；只能走受认证结果恢复 |
| 410 | `selection_expired` | 交互、来源或凭据截止；必须重新发起 |
| 429 | `selection_rate_limited` | 限流，附带 Retry-After |
| 503 | `selection_unavailable` | 权威服务、数据库、容量或网络不可用；失败关闭 |

公开 OIDC 接口继续使用标准 OAuth 错误封装；上表用于私有选择合同，不能改变既有 token 端点的协议。

每次外部调用总截止时间不超过 3 秒，连接时限不超过 1 秒；响应体上限 64 KiB、查询结果分页或限制数量，内部并发信号量有界。失败不回退为浏览器自报身份。日志只记录事务 ID、操作、结果、耗时和脱敏错误类别。

已过期交互、来源引用和选择摘要通过有范围、有限批次的任务清理；恢复窗口内不得提前清理结果。正式安全审计按其独立保留策略处理，不与短期凭据一同删除。

## 9. 实施证据要求

核心类型测试通过，只证明编码和绑定判定，不能证明服务认证、事务原子性或真实登录可用。后续分别补齐：真实数据库与最小角色权限验证、私有 HTTP 合同测试、OIDC 兑换绑定测试、跨标签页及跨应用浏览器验证、故障和恢复验证。

总体方案的完整验收矩阵继续有效。未实施或未验证项不得以“协议兼容”“单元测试通过”替代。

## 10. 当前代码接入说明

目前已接入的路由为认证中心 `/internal/identity/auth/context-interactions`、`/internal/identity/auth/context-selection/source`，以及平台 `/internal/platform/context-selection/choices`、`confirm`、`consume` 和 `result`。员工 BFF 使用进程内消费适配器；外部消费和结果恢复端口目前只允许登记的 CUSTOMER 门户用途。员工 `POST /auth/context-switch` 必须携带 Cookie、CSRF、`expectedVersion` 与经过校验的站内 `returnPath`，仅返回服务器构建的认证中心授权地址。

平台来源传输逐次读取已启用的 `context_selection_registrations.client_registry`：`AUTH` 固定调用 canonical `/internal/identity/auth/context-selection/source`，过渡期 `OIDC` 只调用其明确登记的 `/internal/identity/context-selection/source`。返回 proof 的 `clientRegistry` 必须与原登记一致；canonical 拒绝、超时或不可用都不能尝试旧入口。过渡 OIDC 路径在 Phase 24 删除。

客户消费/恢复请求采用平铺字段：`contractVersion`、`transactionId`、`credential`、`expectedClientId`、`oidcAccessToken`、新门户 `bffSessionId` 和十进制字符串 `version`。LOGIN 不带 `oldOidcAccessToken`（可为 null）；SWITCH 必须带原门户 access，原会话 ID/版本来自身份服务的不可变 intent。新门户版本必须大于原版本。原客户证明记录关联原 BFF ID/版本，平台在切换事务内撤销该版本的全部客户证明；同一旧版本只允许一笔选择消费提交。

`result` 在未消费时返回 `{"status":"PENDING"}`，已消费且原来源、消费方、成员、新门户会话和版本仍匹配时返回 `status=COMMITTED` 与原客户资源 DTO。`consume` 成功也返回相同 COMMITTED DTO。通用结果 JSON 只保存上下文 ID/版本；可恢复的原客户能力单独使用平台 BFF AES keyring、独立 `customer-selection-result` 用途加密，并绑定事务、issuer 和 RP。恢复不创建上下文、不换 token、不延长期限。查询和消费复用 120 秒原截止，当前私有错误转换仍使用 409 表达选择变化/过期，410 的细分尚未单独落地。

门户通过 `CONTEXT_SELECTION_ENABLED=true` 及独立 `CONTEXT_SELECTION_CONSUMER_ID/SECRET` 明确启用；回调先加密保存待完成操作，随后才能外呼消费。受同源读取保护的 `GET /auth/session` 只恢复服务端已保存的操作，不从查询串、Cookie 或浏览器表单创建新选择。待完成记录不会进入普通 session/profile 用例。已启用模式的 `/auth/context` 拒绝本地直接选择。

统一身份域通过 `auth_clients` 的 `context_mode=PLATFORM_CONTEXT` 与允许成员类型登记能力，使用 `auth_context_interactions` 绑定原交互；策略变更使原 grant 失效。平台使用 `context_selection_registrations` 登记 RP 策略快照和应用映射，使用 `context_selection_services` 登记反向调用的服务用途与允许 RP。身份来源服务登记继续位于身份域 `identity_service_clients`，以 `CONTEXT_SELECTION_SOURCE` 用途与 `allowed_auth_clients` 精确限定统一客户端范围。

平台外呼来源核验使用独立配置 `CONTEXT_SOURCE_SERVICE_CLIENT_ID` 和 `CONTEXT_SOURCE_SERVICE_CLIENT_SECRET`。必须成对提供，要求独立身份部署，并禁止复用账号命令、OIDC 管理、客户证明或 BFF RP 的凭据；HTTPS/CA、连接超时、禁止重定向及禁止自动重试复用既有身份服务传输配置。未配置时私有选择接口失败关闭，不改用本地身份表。

当前授权端点将受管选择交互与精确回调、state/nonce 摘要及 PKCE challenge 绑定，并保留尚处于来源核验窗口内的授权请求。来源核验是只读账号证明，不请求平台调用方可能已经持有的账号写锁。受管 RP 只有在本次应用授权和成员选择完成后才能签发 code；code 兑换只交付一次选择凭据，refresh 不交付。员工 BFF 校验发起时能力快照与当前登记，并在同一事务完成选择消费、旧上下文及派生凭据撤销、新业务会话和续期证明建立；缺少选择扩展时失败关闭。

Phase 12 已将交互比较扩展到 scope、state/nonce、CSRF、prompt、max-age 与原 session 快照；查询、确认和消费都重新验证原始授权来源。平台用选择凭据摘要实现一次性消费，并以受配置 AES-GCM keyring 加密保存仅用于不确定提交恢复的短期凭据；恢复只能返回原交易结果，不能重签发凭据或延长原来源期限，成功消费后删除密文。canonical `0001` 限制恢复密文不可替换且消费后必须清空。`ACCOUNT_ONLY` 路径不调用平台成员服务；受管上下文确认仍按未过滤前的服务端候选和当前业务资格判定。
