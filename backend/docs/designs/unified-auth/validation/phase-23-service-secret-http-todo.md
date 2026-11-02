# Phase 23：11 个 service secret 的 HTTP 补齐项

状态：`COMPLETED`，2026-10-29。文件名保留用于既有引用；此矩阵的11项已在tools-r21/apps-r3实际HTTPS恢复演练中完成，见[最终原始记录](phase-23-integration.json)的serviceGet/Access/ContextSource与Target，以及[阶段报告](phase-23.md)。新文件、数据库rearm或gate本身不替代HTTP证据。

当前 `phase23-browser.mjs` 已实际完成表中11项的源正向、恢复后新凭据正向及旧凭据拒绝对照；4项POST使用实际员工／客户会话的AEAD绑定解密访问令牌，另两项通过客户上下文切换获取新来源和待恢复结果。runner对源和目标各有11项覆盖门禁，最终均通过。[当前runbook](../../../../ops/identity-recovery/README.md)要求旧service secret在恢复前曾成功的正常端点验证拒绝。

最后两项使用 `phase23-service-context-cases.mjs`：按canonical browser／flow合同生成source proof，实际新凭据200、旧凭据403且错误码为`selection_not_allowed`；portal pending envelope的用途绑定解密另有组件覆盖。runner在源与目标分别创建客户切换并提取各自的真实交互材料；临时数据库故障触发器在finally中按所有权标记移除。

约定：I 为 `https://auth.saas.test:24444`，P 为 `https://console.saas.test:24444`。全部使用配置 CA、单个 Basic Authorization，无 Cookie。每项先记录 source 原 secret 的实际 200；target 对照固定合法的新 body、service ID 和依赖状态，只替换旧／新 secret。429、503、错误 proof 或自然过期不计旧 secret 的认证拒绝。

| 绑定／secret 所在配置 | 正常端点 | 最小 body／target 正控制 | old secret 预期 |
|---|---|---|---|
| accountService／platform.IDENTITY_SERVICE_CLIENT_SECRET | GET I/internal/identity/accounts/{ids.admin} | 无 body | 403 |
| reviewService／platform.IDENTITY_REVIEW_SERVICE_CLIENT_SECRET | GET I/internal/identity/auth/review/applications/{ids.application}/test | 无 body；合法但未登记时亦可 200 UNREGISTERED | 403 |
| invitationService／platform.IDENTITY_INVITATION_SERVICE_CLIENT_SECRET | GET I/internal/identity/auth/invitations | 无 body，列表可空 | 403 |
| authAdminService／platform.IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_SECRET | GET I/internal/identity/auth/clients | 无 body，列表可空 | 403 |
| oidcAdminService／platform.IDENTITY_ADMIN_SERVICE_CLIENT_SECRET | GET I/internal/identity/oidc/clients | 无 body，列表可空 | 403 |
| sourceService／platform.CONTEXT_SOURCE_SERVICE_CLIENT_SECRET | POST I/internal/identity/auth/account-proof | platform-account-proof-request/1，fresh 员工 console access，expectedClientId=ids.client | 403 |
| portalSourceService／portal.CUSTOMER_SOURCE_SERVICE_CLIENT_SECRET | POST I/internal/identity/auth/customer-account-proof | customer-account-proof-request/1，fresh 客户 portal access，expectedClientId=ids.portalClient | 403 |
| customerAuthorityService／platform.CUSTOMER_AUTHORITY_SERVICE_CLIENT_SECRET | POST I/internal/identity/customer-access-proof | customer-access-proof-request/1，fresh 客户 portal access，expectedClientId=ids.portalClient | 403 |
| profileClient／portal.CUSTOMER_RESOURCE_CLIENT_SECRET | POST P/internal/customer-portal/memberships | `{oidcAccessToken: fresh客户portal access}`；Basic ID 用配置中的 `https-profile-*` 公共值，不能误用内部 UUID | 401 |
| selectionService／identity.CONTEXT_PLATFORM_SERVICE_SECRET | POST P/internal/platform/context-selection/choices | context-selection/1，fresh sourceProof，expectedClientId；交互尚未结束且未过期 | 403 selection_not_allowed |
| portalConsumerService／portal.CONTEXT_SELECTION_CONSUMER_SECRET | POST P/internal/platform/context-selection/result | 保留真实新 portal result 的完整准确 body：contractVersion、transactionId、credential、expectedClientId、fresh oidcAccessToken、精确 bffSessionId、version 字符串 | 403 selection_not_allowed |

三个 access proof body 使用字段 `contractVersion`、`oidcAccessToken`、`expectedClientId`。员工／客户 access 必须属于恢复后的新正常会话；可由 operator 在保护路径内解密 owned BFF 的 `access_ciphertext`，只在进程内存中使用并纳入日志 canary 扫描。不得把它写入普通日志、argv 或正式报告。

最后两项需要一个新合法 context 流程。choices 必须拿尚未结束的 fresh sourceProof；portal result 可使用准确新 body 获得 200 PENDING，或原已提交 body 获得 200 COMMITTED。仅重放恢复前的旧 sourceProof／transaction／credential／access，即使 old secret 请求被拒绝，也只是组合旧来源拒绝，不能证明单独换 secret 的效果。

建议补齐顺序为 5 个 GET → 4 个 fresh access 请求 → 2 个 fresh context 请求。target 测试放在旧载体 invariant 完成、新员工／客户业务成功之后、停止 writers 之前；source 的 11 个原 secret 200 控制须在 checkpoint 前完成。新增 helper 或 runner 改动必须进入新的冻结工具快照，并重新核对应用构建输入；不需要因为纯工具变更伪造新应用编译。

源码依据：身份服务的 [Basic 认证](../../../../../../saas-identity/src/internal.rs)、平台的 [context handlers](../../../../src/context_selection_http/mod.rs)、[context service 认证](../../../../src/context_selection_http/store.rs)、[profile handler](../../../../src/customer_portal/mod.rs)、[portal result handler](../../../../src/customer_portal/selection.rs)，以及 [11 项绑定和 rearm](../../../../tests/unified-auth-https/phase23-recovery-files.py)。结果仅适用于正式报告记录的冻结输入和实际环境。
