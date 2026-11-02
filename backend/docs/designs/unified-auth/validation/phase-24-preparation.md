# Phase 24 退役准备盘点

日期：2026-10-28。状态：**只读准备完成，尚未执行退役，不能作为 Phase 24 验收通过证据。** 本次读取当前工作树、[Phase 1 调用清单](../callsite-inventory.md)、[Phase 2 暂存清单](../phase-24-retirement-list.md)和[主台账](../../unified-oauth-oidc-sso-task-ledger.md)。Phase 21、22 正在并行实施，Phase 23 的最终密钥／恢复合同还需输入；正式删除必须等待台账列出的全部依赖完成。

本工作区是八个独立仓库，不是根 Cargo workspace。本次仅新增本准备文档，没有修改生产代码、SQL、配置、测试或台账，没有操作数据库，没有启动／停止服务，没有创建或复用 Docker 容器（0），也没有清理共享缓存、已有资源或磁盘。下面是当前源码的静态分类，不继承旧报告的通过结论。

## 先固定不能误删的边界

1. **统一签发仍依赖名称为 `oidc_*` 的当前权威对象。** `saas-identity/src/oidc/protocol.rs` 已公开 canonical `/oauth/*`，但 `oidc/mod.rs::initialize/authority` 仍校验 `oidc_provider_settings`，`oidc/keys.rs` 的 JWKS、发布／激活／退役／撤销／签名仍读写 `oidc_signing_keys`。`sign_delivery` 也复用该签名器。主代理已明确裁决：这些唯一当前权威对象可保留实际名称并用用途 allowlist 解释；不为清除 `oidc` 字符串进行无收益改名。不能删整个 `src/oidc` 包，也不能把它们当作无调用者的旧表直接移除。
2. **旧名称的来源函数已有 canonical 实现。** 当前 `bff_oidc_proof_active` 委托 `auth_user_grant_source_active`；`customer_portal_account_source_active` 检查 `auth_grants/auth_access_tokens`；`identity_lock_oidc_source` 按 application → client → consent group 加锁，但仍从 `oidc_provider_settings` 校验 issuer。这些是现行来源／事务 fence，必须保留语义；名称本身不构成第二签发链路。`bff_sessions` 中 `oidc_grant_id/oidc_public_sid/oidc_public_sub` 是当前 canonical RP 的原始 grant、opaque sid/sub，不是旧 token 兼容路径。
3. **内部应用能力必须保留。** `/api/oauth/session-token` 目前由 `routes/oauth2.rs::consent_routes` 挂载，实际 handler 已是 `app_capabilities::issue`，写 `platform_app_capabilities`，只接受 `platform_session` 并签发限定 audience、来源和短期限的 `app_session`。宿主 `MicroAppContainer.tsx` 仍调用它。退役旧 issuer 时应把该路由移交独立能力模块，或与宿主同时改成明确的内部能力路径；不可随 `oauth2` 模块一起丢失。
4. **平台与身份职责继续分离。** 平台保留应用／合作方业务登记、成员、permission、dataGrant、私有目录／通知接口；identity `auth_*` 独占 OAuth 注册和签发。`platform_sessions`、浏览器会话、BFF 事务、内部能力、通知会话与 customer portal 独立库都不是第二授权服务器。
5. **回执和 intent 名称保留历史术语，合同是当前的。** `identity_oidc_admin_receipts`、`platform_oidc_command_intents` 和 `identity_oidc_command_intent_active` 已承载 APPLICATION_PROJECTION、AUTH_ADMIN、AUTH_REVIEW、AUTH_INVITATION 等现行操作。按主代理裁决可保留实际名称，以当前用途和唯一合同解释；保留 operation、digest、原来源、目标路径、权限及结果恢复约束，不能直接删除幂等边界。若正式退役确实改变接口，再同步 owner/ACL/readiness/恢复工具并最小充分重验。

## 逐项退役／去向清单

路径均相对工作区；正式执行前需重查并发阶段的新改动。

| 编号 | 当前对象及可追踪调用 | Phase 24 去向 | 删除前必须保留的证据／约束 |
| --- | --- | --- | --- |
| R01 | 平台 `backend/src/bootstrap.rs` 无条件 `.nest("/oauth", routes::oauth2::oauth_router())`；`routes/oauth2.rs` 提供 authorize/token/userinfo/introspect/revoke/device/discovery | 移除平台 public OAuth router、旧签发实现及旧 consent decision；公开 issuer 只剩 identity `/.well-known/openid-configuration` 和 `/oauth/*` | 8–11、13–18、20–21 的 canonical 替代；新增平台旧协议入口缺席回归，权威不可达时失败关闭 |
| R02 | 平台 `routes/mod.rs` 始终 merge `oauth_admin::routes()`；`routes/oauth_admin.rs` 读写 `oauth_clients` 和旧 token/device 表 | 删除旧客户端 CRUD／密钥重置／旧 token 与 device 管理路由。若保留平台会话运维，移到明确会话 API 并保留原细权限、来源及审计检查 | 统一客户端页面和 `/api/identity/clients*`；当前会话功能不能通过删整模块意外消失 |
| R03 | 平台 `api_router_for_deployment(false)` 仍 merge 同进程 `sso::routes()` 与 `oidc::routes()`；`config.rs` 仍能配置本机 SSO/OIDC；`src/oidc/{protocol,clients,interaction,store,tokens,keys,account_logout}.rs` 是平台旧权威副本 | 生产只保留远程 identity 模式；移除本机授权服务器装配、签名私钥配置及代码；仍需的纯测试辅助改用现行合同或删除 | 平台实际启动配置明确拒绝本机旧签发；保留 remote_admin 当前命令门面，不因目录名相同误删 |
| R04 | identity `oidc/protocol.rs` 仍挂载 public `/oidc/*`、`/internal/identity/context-selection/source` 和旧 `/internal/identity/oidc/clients*`；`oidc/{clients,interaction,store,tokens,context,account_logout,consents}.rs` 含旧表路径 | 删除旧 public／private client 与上下文协议及其专属模块；保留 canonical `oidc/oauth/*`、共享错误／签名／subject 实现，按真实引用精简 | 两个 BFF、标准库示例只发现／调用 canonical endpoints；旧路由 404，无重定向兼容或自动 fallback |
| R05 | identity `sso/routes.rs` 的 `/api/sso/applications*`；`sso/routes/applications.rs` 读写 `oidc_consents/oidc_grants`。静态 applications 页已使用 `/api/sso/consents*` | 删除旧 list/revoke/delete handlers 和旧 consent 存储；保留 `/sso/applications` 页面及 canonical consent API | canonical consent 的撤销、展示删除、重授权／group generation 证据；不能误删页面路由 |
| R06 | `oidc_signing_keys`、`oidc_provider_settings`、密钥 immutable trigger；identity `oidc/keys.rs`、`oidc/mod.rs`；当前 keys 私有路径 `/internal/identity/oidc/keys/*`，服务用途 OIDC_ADMIN | 保留唯一当前 authority 的实际对象／keys 路径并登记 allowlist，与 Phase 23 最终 key/runbook/恢复合同一致；仅在有真实边界收益时另行修改接口，不保留第二实现或自动fallback | 发布、激活、并存窗口、last_signed_exp、紧急撤销、restart、JWKS、worker 签名和恢复验证全部保留；当前 OIDC_ADMIN 不能误当 AUTH_ADMIN 已可互换 |
| R07 | 旧表 `oauth_clients/oauth_tokens/oauth_authorization_codes/oauth_authorization_requests/oauth_device_codes`；旧 identity `oidc_clients/oidc_consents/oidc_authorization_requests/oidc_authorization_codes/oidc_grants/oidc_access_tokens/oidc_refresh_tokens/oidc_account_logout_receipts/oidc_context_clients/oidc_context_interactions` | 从当前 `0001_init.sql` 删除完整 DDL、外键、索引、旧 immutable／generation／revoke triggers 及专属 helper；不是对已有数据库执行 DROP 或搬迁 | R01–R06 和测试调用者先闭合；空库新建时只产生目标对象，不能修改历史 migration checksum 接管旧库 |
| R08 | `identity_service_clients.allowed_oidc_clients`、`context_selection_services.allowed_oidc_clients` 与 `allowed_auth_clients` 并存；`context_selection_http/store.rs` 接受二者 OR；旧 context client 表／trigger | 移除旧 client 允许列表分支、旧列及旧 fixture，canonical `allowed_auth_clients` 成为唯一登记；检查 config/owner 工具同步 | 服务用途、精确 client、generation 及 source/result proof 保持；credential 隔离检查仍覆盖现行 service/canonical client |
| R09 | `bff/store.rs::revoke`、`authentication/postgres.rs`、`routes/users*.rs`、`routes/monitor.rs`、`seed.rs`、`bootstrap.rs` 周期任务仍访问旧 oauth 表；SQL account/member/application 撤销 helper 包含旧 oauth 更新 | 删除旧 oauth 分支／自动 purge／device expiry；改剩余统计和恢复事实到当前 canonical 或内部会话对象，清除旧 secret reuse 检查中的旧表分支 | 不能删掉同一事务的 current platform/BFF/session/app capability 撤销；账号停用、成员变化、应用授权收缩继续使内部能力失效 |
| R10 | `identity_oidc_admin_receipts/platform_oidc_command_intents` 和 fence；identity `admin_operations.rs`；平台 `oidc/remote_admin.rs` 的旧 clients/keys/operations facade 与现行 AUTH commands 混居 | 保留现行命令引擎及实际receipt/intent名称；移除旧 client API和仅服务旧命令的route allowlist，当前keys／operation恢复路径按用途保留；不为字符串清零机械重命名 | operation lookup 必须与原服务、method/path、digest和原授权源绑定；APPLICATION_PROJECTION/AUTH_ADMIN/AUTH_REVIEW/AUTH_INVITATION 各用途隔离不能合并放宽 |
| R11 | 平台前端 `api/oauth.ts` 留旧 client/token/device/discovery 调用；`TokensPage.tsx` 仍为 `App.tsx` 当前 `/system/oauth/tokens` 页面；`MonitorPage.tsx` 请求平台 `/oauth/.well-known/openid-configuration`；`ClientsPage.tsx` 已 re-export UnifiedClientsPage | 删除无调用旧 client DTO/API；旧 token/device 视图移除，所需 platform session 运维独立保留；监控改 current identity 健康／公开能力的受控投影；同步菜单/权限/类型/浏览器断言 | `UnifiedClientsPage`、PartnerIntakePanel、TOTP step-up 和现行细权限保留。不能只删后台而留下当前页面请求旧表 |
| R12 | 平台 `bff_tests.rs/bff_tests/centralized.rs`、`customer_portal_tests/legacy_source_projection.sql`、`customer_identity/{dto,proof,routes}.rs`、`oidc/customer_portal.rs`，以及大量旧 oauth fixture | 用等价 canonical source fixture 表达仍有价值的锁、迟到回调、未知结果、隔离、源撤销回归；删除仅证明已退役协议的测试／SQL。`customer_identity/mod.rs` 已使 production router 为空，proof 模块仅 `cfg(test)` | 不把 test-only adapter 报成 live fallback；当前 `identity_deployment_tests::phase14/15/20`、BFF current-source 与真实 HTTPS 验收必须保留／重跑 |
| R13 | Stores `authorization/client.rs::{introspect,context}` 与 `policy::validate_intro` 是 `cfg(test)` 的旧 transport adapters；work-orders/reminder 已用 current private contract；notification 仍用平台通知 context-proof | Stores 旧测试适配器移除或将负向传输 case 转为 `resource-authorization/1`／`resource-directory/1`；work-orders 与 notification 保留当前私有 source/permission/notification 合同 | STORE 限制、token 类别/audience/scope、SQL rollback、direct dataGrant、result-first、unknown outcome、Socket.IO 来源断开和超时上限不能丢 |
| R14 | 旧示例 `saas-examples/oauth-playground` 的 `OAUTH_PLATFORM_ORIGIN`、手写 code/refresh/device/password 协议、平台 issuer 指南和部署模板；Phase21 新 `oidc-rp` 将使用标准库和公开 authority | 待21完成，选定删除旧演练或明确改造为 current issuer 支持能力；所有现行默认/guide改到21独立示例。设备／password 流不能继续宣称统一 issuer 已支持 | 第三方默认 USER/ACCOUNT_ONLY/openid profile 与 offline/MACHINE 拒绝政策不变；示例标准库不读取平台／身份库、无内部 proof、无 UUID sub 假设 |
| R15 | `ops/identity-deployment/{provision,verify}.sql`、identity `readiness.rs`、平台 bootstrap role checks、seed/init、HTTPS prepare、recovery/witness inventory、backup manifest 仍含旧对象；`ops/recovery/invalidate.sql` 仅更新旧 oauth token | 同一 schema owner 统一更新所有清单。Phase23旧 recovery.py/witness.py 必须按最终 canonical inventory 替换；24实际移除旧对象时同步清单，当前保留对象沿用真实名称；不保留旧恢复路径为当前操作指南 | real LOGIN role ACL、无跨owner凭据读取、窄 SECDEF 归属和锁、恢复后授权／同意／撤销代际／RP tombstones 不复活；新库只跑当前0001 |
| R16 | 当前 README/docs/配置/compose/nginx/网关中旧端点、OIDC_ADMIN配置、OAUTH_AUTO_PURGE、旧恢复说明；历史阶段报告中也包含原字符串 | 更新现行说明和可执行配置、移除过时索引；历史证据按历史保留，静态 gate 为旧字符串建立有理由的 history／negative-test allowlist | public gateway 继续拒绝 `/internal/*`；禁止把所有 `OIDC_*` 字样当旧物（BFF与公开OIDC配置仍是当前能力）；不把历史report当25新版本通过证据 |

## 八仓库处置边界

| 仓库 | 本轮静态判断 | Phase 24 允许改动的方向 |
| --- | --- | --- |
| saas-identity | canonical 和 legacy 同在 oidc 包；keys/provider仍是当前权威；旧applications API仍挂载 | 删除旧协议／client／consent，保留并收敛 keys/subjects/current oauth、MFA、邀请、delivery、private proofs、readiness |
| saas-plateform | 旧平台 issuer/admin 仍挂载；本机权威可配置；内部能力已独立实现；前端旧设备与会话页仍有调用 | 平台旧权威和旧表退出；current管理门面、BFF、app_session、成员／业务授权和运维投影保留 |
| saas-customer-portal | 生产BFF已使用canonical `/oauth/*`；仅前端负向URL测试中出现 `/oidc/authorize` | 通常无需协议重写；同步最终配置／文档并重跑current contracts；保留负向旧URL拒绝断言 |
| saas-stores | 生产为current resource/directory合同；旧introspection/context仅测试适配器 | 退役测试适配器；保持现行资源、组织和来源验证 |
| saas-work-orders | 生产为current resource/directory/private-notification合同 | 主要同步文档／配置与最终矩阵；不降级为平台旧introspection |
| saas-notification | `/api/internal/notifications/context-proof` 是当前用途明确的通知证明，不是OAuth签发 | 保留并重跑来源撤销／连接隔离；不按“internal旧路径”误删 |
| saas-micro-app-sdk | 不签发身份凭据，消费宿主上下文及能力；旧版本release/vendor为版本工件 | 保留当前3.0.0契约；仅移除当前指南或真实消费者中的旧issuer调用，不能以搜到release文本就删除已发布工件 |
| saas-examples | 旧playground仍宣称平台issuer/device/password；21新标准库示例进行中 | 由21最终结果决定旧playground去向；当前索引只指有效示例 |

## 建议的并行所有权与合并顺序

以下是执行分工建议，不表示已启动这些变更。

| Owner | 独占文件组 | 可并行工作 | 合并点 |
| --- | --- | --- | --- |
| A：identity协议／keys | identity `src/oidc`、`sso/routes/applications`、`admin_operations`、`internal`、对应tests/config | 在23完成后删旧入口，保留当前签名／receipt实际名称及合同 | 先与C固定被删除对象和保留allowlist；不得与Phase22同时改同一源文件 |
| B：平台运行及前端 | 平台 `routes/oauth2/oauth_admin/device`、`oauth_issuance`、`oidc`、bootstrap/config、监控/session API；frontend相关API/page/menu/tests | 删旧issuer，移交app capability路由，清理当前页面调用，拆出仍需会话运维 | C只接收schema要求，不与B同时批量替换同一Rust／前端文件；remote_admin统一由B维护 |
| C：schema／ACL／init／恢复 | 平台 `0001_init.sql`、provision/verify、owner/bootstrap清单、identity readiness、Phase23恢复inventory和key工具、HTTPS prepare | 依A/B给出的唯一合同完成全新基线，删除旧表/trigger，收敛对象清单与权限 | **单一owner编辑0001**；B移除旧表query后再做fresh install；readiness与各owner变更串行合并 |
| D：消费者及fixture／文档 | stores测试adapter、旧playground去向、独立fixture重写、docs/config/nginx的指定子集 | 与A/B/C确认current合同后处理低冲突文件，逐条回填R01–R16去向 | 当前report/台账由root统一；禁止多人重写总清单 |
| Root：集成与审查 | 总台账、阶段总报告、固定源码快照、跨仓fresh PG/TLS流程 | 可在分组实现期间独立审查，只在全部合入后冻结候选执行最终gate | 单一环境owner管理task DB/process/secret/cache/cleanup；不能用改名前报告代替改后验证 |

顺序：先等待18/21/23各自完成并固定合同 → 并行A/B/D → C串行收敛schema与ACL → 清除残留query／fixture → 固定源码、从空库执行owner/runtime/readiness → 重新运行current协议、BFF、私有资源、通知和key/recovery回归 → current源码/config/docs静态检查 → 清理本阶段资源并独立审查。平台和identity可并行删旧实现，`0001`、共享helper、receipt路径、测试制备和总环境不得并行无协调地修改。

## 正式执行需要产生的证据

- 为R01–R16逐条记录最终去向、变更文件和新执行验证，不只记录`rg`命中数。
- 两个生产router的旧入口缺席、平台无法配置成第二issuer；canonical discovery和标准客户端仍通过。
- 新库catalog无旧oauth/oidc注册／grant／token表，schema清单、owner ACL、runtime readiness和初始化工具一致；明确允许保留的当前key/settings/receipt对象及真实名称。
- 内部 `app_session`、BFF／portal sessions、private resource/directory/notification合同仍有用途、audience、source和failure-closed证据。
- 账号／应用／成员／同意收缩及CURRENT_APP/GLOBAL登出仍使原来源失效；后台统计不依赖被删表。
- Phase23最终密钥／备份／恢复工具按退役后的当前基线运行其最小充分矩阵，已撤销／已注销／tombstone来源不复活；未发生接口或语义变更的部分不无意义重复重做。
- 对全八仓 current source/config/test/docs运行旧表、旧路由、旧凭据和checksum兼容搜索，逐项解释保留的历史report、负向测试、版本工件；current guide相对链接有效。
- 从所有Cargo/Node脚本预查是否间接创建容器。优先复用task原生PG与cache；若需要容器，预先按AGENTS列明项目／预算／context／归属并完整清理。没有执行上述验证前不得宣称Phase24 PASS。

## 本轮准备自检

| 检查 | 结果 | 说明 |
| --- | --- | --- |
| 上级／根规则、台账24、Phase1调用清单和Phase2退役清单已读 | PASS | 静态盘点输入 |
| 八仓旧路径与current用途分类 | PASS | R01–R16与八仓表；目录名／字段名不替代调用链判断 |
| 生产实现／数据库／服务修改 | 未执行 | 只新增本准备文档；不计为Phase24实施或验收 |
| 测试与fresh-install证明 | 未执行 | 需最终合同和正式Phase24依赖满足后运行 |
| 本次容器和临时运行资源 | 0 | 未创建或复用，无清理操作；不声称共享资源已释放或WSL磁盘已缩小 |

