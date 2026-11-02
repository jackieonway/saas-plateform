# Phase 25 部署目标盘点与 Phase 26 切换候选

状态：`PREPARED_WITH_PHASE26_GATES`。本文件是当前候选的参数、操作和回退清单，不是正式切换记录，也不表示 Phase 25 全部测试通过。机器可读盘点、时间、来源摘要和未满足门禁见[JSON](phase-25-deployment-plan.json)。后续实际候选摘要由 Phase 25 candidate 报告补齐，批准前必须绑定同一最终版本。

## 1. 已核实的本机事实

本次只读检查 Windows 监听和进程元数据、hosts、公开证书、`.local-run` 配置的非秘密字段，以及 WSL Ubuntu 的监听和 PostgreSQL 5432 的数据库名/owner。没有启动或停止服务，没有查询业务行，没有改动数据库、HBA、证书信任或网关。

| 项目 | 观察结果 | 解释 |
| --- | --- | --- |
| 主机范围 | 当前 Windows + WSL Ubuntu，本机回环 | 没有远端部署目标或公网 DNS 授权 |
| hosts | `127.0.0.1 auth.saas.test console.saas.test customer.saas.test` | 用户已确认前两域名；客户域名是现存本机配置，仍需纳入本次切换范围 |
| Windows 443/4443 和应用端口 | 盘点时没有监听 | 不证明端口随后持续空闲；执行前必须重查 |
| 旧 `.local-run` PID | 20 份记录的 PID 均没有对应 Windows 进程 | 不能凭旧 PID 文件停止进程或声称服务仍健康 |
| WSL PostgreSQL 5432 | `127.0.0.1:5432`，postgres PID 273 | 既有业务引擎，未改动 |
| WSL PostgreSQL 55494 | `127.0.0.1:55494`，postgres PID 1317 | 任务验证引擎，不能把它当正式目标 |
| 本机证书 | CA 和 leaf 有效至 2027-10-26；leaf SAN 包含三个 `.saas.test` 和 localhost；链校验成功 | 只证明公开证书内容与链，不证明秘密保管、未来可用性或正式证书获准 |
| Windows 信任 | 该 CA 在当前用户 Root 中，在 LocalMachine Root 中未找到 | 不能据此推断所有用户、WSL 服务、外部客户端均信任它 |

既有数据库必须保留：

| 数据库 | 观察到的 owner | 当前配置中的调用方 |
| --- | --- | --- |
| `saas_platform_authux_dev` | `saas_local26_migration_owner` | 平台、身份、通知使用不同账号 |
| `stores_codex_local` | `saas_local26_stores` | 门店 |
| `work_orders_dev` | `saas_local26_orders` | 工单 |
| `saas_customer_portal_dev` | `saas_local26_portal` | 客户门户 |

这些是目录盘点，未认证各库的当前 schema、业务内容、备份或实际可用性。以前清库许可不扩展为本次删除许可。

## 2. 旧本机启动配置不能直接重启

`.local-run` 是既有本机资料，不属于本轮临时文件；本次不删除或改写它。当前源码与旧资料存在以下具体差异：

1. 平台旧 `IDENTITY_ADMIN_SERVICE_CLIENT_ID/SECRET` 已被当前配置明确拒绝；KEY_ADMIN、AUTH_ADMIN、APPLICATION_PROJECTION、AUTH_REVIEW、AUTH_INVITATION 等用途应独立登记和配置。
2. 门户旧配置缺少启用当前 CUSTOMER 来源必需的 `IDENTITY_SERVICE_URL`、`CUSTOMER_SOURCE_SERVICE_CLIENT_ID/SECRET` 与私有身份 CA；不能借用员工 RP、资源凭据或公开 issuer。
3. 门店/工单旧 `PLATFORM_API_ORIGIN` 指向公开控制台 443，未配置当前外部 Access Token 的准确 identity issuer 和审核 scope 映射。新私有入口必须同时路由准确授权 POST 与目录/提醒 POST。
4. 旧 gateway 仍转发 oauth-playground，并仅为 console 的 `/internal/` 提供 4443；这不能满足当前身份私网、业务授权 POST 和通知 context-proof 的完整最小路由。
5. 旧通知配置使用 development 回环 HTTP，未证明生产私有 TLS/CA、指标凭据和受限 DB 权限。旧身份配置也未证明当前投递、TOTP、限流、监控和密钥材料全部齐备。

平台 `OIDC_ISSUER` 仍表示远程身份的固定 issuer 校验，与 `BFF_OIDC_ISSUER` 必须一致；它没有在平台重新启用 provider。门店当前模式名 `combined` 和工单 `canonical` 均只使用当前 `/api/apps/{code}/authorize` 合同，不能机械交换这两个配置值。

## 3. 可审阅的新库方案

建议在既有 5432 引擎中新建以下四库，保留第 1 节所有旧库及资料。以下名字和角色前缀在盘点时均不存在，**尚未创建，尚未获实际切换授权**。执行前再次检查不存在、引擎身份与归属；任何冲突停止，不复用或覆盖。

| 服务 | 提议数据库 | 提议角色/归属 |
| --- | --- | --- |
| 平台 + 身份 + 通知 | `saas_unified_auth_release_20261029` | `saas_release_20261029` 加当前六个共享库角色后缀；通知另用最小通知行读取 LOGIN |
| 门店 | `saas_stores_release_20261029` | 独立门店 owner/runtime，按当前启动模式和最小必要权限验证 |
| 工单 | `saas_work_orders_release_20261029` | 独立工单 owner/runtime，按当前启动模式和最小必要权限验证 |
| 客户门户 | `saas_customer_portal_release_20261029` | 独立 NOLOGIN owner + LOGIN runtime，无共享库 CONNECT |

服务建议仍使用 auth/console/customer 三个本机主机名；公开端口 443，私有 TLS 端口候选 4443。私有入口的实际监听实现、SNI、仅回环/允许来源、上游端口和服务管理方式必须在批准清单中明确；任务 `24443/24444` 端口不能原样冒充正式配置。运行文件与秘密应置于单独受保护目录，发布目录只读；实际路径/Windows ACL/WSL 权限由环境 owner 核实后写入最终非秘密清单。

角色提案进一步固定如下；这些都是尚未创建的候选名称，不能作为已批准对象。共享库六角色由当前 `create-roles.sql` 创建，其余独立库 owner/runtime 和通知 runtime 需要单独受控创建及最小权限交接，不将它们加入共享库六角色闭包。

| 范围 | 完整候选角色名 | 模式 |
| --- | --- | --- |
| 共享库 | `saas_release_20261029_migration_owner`、`saas_release_20261029_identity_owner`、`saas_release_20261029_platform_owner`、`saas_release_20261029_bridge_owner` | NOLOGIN / NOINHERIT |
| 身份、平台 | `saas_release_20261029_identity_runtime`、`saas_release_20261029_platform_runtime` | 独立 LOGIN / NOINHERIT |
| 通知 | `saas_release_20261029_notification_runtime` | LOGIN / NOINHERIT，仅当前通知查询所需权限 |
| 门店 | `saas_release_20261029_stores_owner`、`saas_release_20261029_stores_runtime` | owner 为 NOLOGIN，runtime 为 LOGIN；均 NOINHERIT |
| 工单 | `saas_release_20261029_orders_owner`、`saas_release_20261029_orders_runtime` | owner 为 NOLOGIN，runtime 为 LOGIN；均 NOINHERIT |
| 门户 | `saas_release_20261029_portal_owner`、`saas_release_20261029_portal_runtime` | owner 为 NOLOGIN，runtime 为 LOGIN；均 NOINHERIT |

以上共 13 个角色，均禁止 SUPERUSER、CREATEDB、CREATEROLE、REPLICATION 和 BYPASSRLS。迁移、owner 操作只经已批准的管理连接执行；runtime 不持有 owner 成员资格，不执行 DDL。创建库/角色、迁移与 ACL 交接是三个独立步骤，必须逐个记录结果；不能因为 migration 二进制只操作空库便漏掉创建动作的授权与归属。

仅新建库方案不包含 DROP/清空旧库、改远端、改公网 DNS、停止 WSL/Docker、移动 VHDX、清理共享资源、发送邀请给真实第三方或公开开放注册。这些动作不能从执行全部阶段的总体目标中推导具体对象。

## 4. 部署参数与受保护材料

| 部署单元 | 非秘密参数必须绑定 | 秘密仅使用受保护引用 |
| --- | --- | --- |
| 身份 | `APP_ENV`、`BIND_ADDR`、`SSO_ENABLED`、`OIDC_ENABLED`、精确 `SSO_ORIGIN/OIDC_ISSUER`、`TRUSTED_PROXY_IPS`；DB/限流/密码/连接预算 | 独立 identity DB 密码、`OIDC_KEYSET_PATH`、`SSO_TOTP_ENCRYPTION_KEY`、context keyset、invitation keyset、`IDENTITY_METRICS_TOKEN` |
| 平台 | SSO-only；`IDENTITY_REMOTE_ENABLED=true`、公开 `OIDC_ISSUER` 与 `BFF_OIDC_ISSUER`、精确 console callback；私有 `IDENTITY_SERVICE_URL/CA`；resource budgets；数据授权策略 | 平台 DB 密码、BFF RP secret、BFF AEAD、JWT_SECRET、METRICS_TOKEN；ACCOUNT_COMMAND、KEY_ADMIN、APPLICATION_PROJECTION、AUTH_ADMIN、AUTH_REVIEW、AUTH_INVITATION、RESOURCE_ACCESS_PROOF、员工 source 等独立凭据 |
| 门户 | 独立 CUSTOMER_PORTAL / USER / PLATFORM_CONTEXT RP、独立主机及 `/auth/callback`、私有 identity/资源 origin 和 CA、选择 consumer | 门户 DB 密码、RP secret、CUSTOMER source secret、CUSTOMER_PROFILE secret、consumer secret、独立 portal AEAD |
| 门店 | `STORES_DATABASE_URL` 的主机/端口/库/账号；准确 allowed origin；私有平台 origin/CA；`PLATFORM_AUTHORIZATION_MODE=combined`；identity issuer；审核 scope map | 门店 DB 密码、仅绑定 stores 的独立 APP_RESOURCE 凭据、STORES_METRICS_TOKEN |
| 工单 | `WORK_ORDERS_DATABASE_URL` 的主机/端口/库/账号；准确 allowed origin；私有平台 origin/CA；`PLATFORM_AUTHORIZATION_MODE=canonical`；identity issuer；审核 scope map | 工单 DB 密码、仅绑定 work-orders 的独立 APP_RESOURCE 凭据、WORK_ORDERS_METRICS_TOKEN |
| 通知 | `NOTIFICATION_ENV`、受限 DB 账号、私有 `PLATFORM_API_BASE_URL`、`NOTIFICATION_PLATFORM_CA_FILE`、准确 allowed origins、Socket.IO path 与有界队列 | 通知 DB 密码、独立 METRICS_TOKEN；不复用 RP/资源服务 secret |
| 标准 RP / MACHINE | 独立已批准 client/environment、精确 callback/issuer/CA、resource audience 与批准 scope；登出支持与本地会话期限 | RP/MACHINE 各自凭据，与内部服务用途分离 |

CA、TLS、数据库 TLS 是三项独立核验。正式数据库连接应明确选定并验证 TLS 模式；旧 URL 未写 sslmode，不能据 HTTPS 推断 PG 已强制 verify-full。配置清单只记引用、存在/权限/用途核验结果与公开指纹；不记录密码、Cookie、token、PHC、恢复码、私钥或完整连接串。

## 5. 全新安装及接流前顺序

1. 绑定 Phase 25 最终源码清单、全部应用/迁移二进制、前端/SDK/示例摘要、工具链及配置模板。重查 Windows/WSL 监听、5432 引擎身份、四库及角色不存在，保存只读 preflight；经批准的管理连接创建准确四个空库，记录管理会话身份及每库归属。不得连接旧库运行迁移。
2. 在批准的四个空库运行当前 `platform-migrate`、`stores-migrate`、`work-orders-migrate`、`portal-migrate`，迁移作业使用明确管理连接或已获授权的 owner 会话，完成后交接最终 owner/runtime。门店/工单子进程仅注入专用 URL，拒绝所有继承的 `PG*`；无参数迁移不 seed、不开 HTTP、不创建库/角色，不使用 `--import` 表达数据迁移。NOLOGIN owner 不设置可登录密码；不为运行迁移给 runtime 临时扩权。
3. 只在新共享库显式 bootstrap 受控管理员、租户/组织、应用/菜单/授权、两个独立人员 RP、资源与 scope、MACHINE、各私有用途服务和精确 callback。管理员通过真实主认证/TOTP 绑定；不种正向 MFA 时间。
4. 执行当前 identity-deployment create-roles/provision/verify，绑定精确 expected_database、UUIDv7 deployment、issuer、role_prefix 和数据库 COMMENT；单独建立并交接第 3 节所列业务/门户 owner/runtime 及通知受限角色。确认受限 runtime、owner/ACL/readiness、无跨库 CONNECT 和无秘密复用；保存真实角色允许/拒绝矩阵，不能只以管理连接成功代替 runtime 验证。
5. 在入口关闭状态启动候选服务/worker，检查实际 schema/issuer/角色 readiness、公开发现/JWKS、私有 HTTPS 的 CA/主机名负例、资源授权和通知 context-proof。公共入口固定拒绝旧 issuer/playground、`/internal/*`、服务授权 POST，只有准确浏览器/RP路由可达。
6. 在本机候选入口完成员工/客户/标准第三方/MACHINE 冒烟；业务服务实际 DB/HTTP、上下文/成员隔离、撤销/登出、503失败关闭、未知原操作恢复、通知重试和双 RP Cookie 隔离均有当前版本证据。不能以健康200或旧报告代替。
7. 确认第 6 节全部外部门禁后，保存现状和批准清单；以单一明确动作启用唯一 issuer 对应的候选入口，禁止同时启动旧 authority。若当前没有旧进程，也必须按版本及归属核对，不能凭过期 PID 执行 stop。
8. 从实际获准客户端访问三域名及业务路径，确认 TLS、固定 issuer/回调、敏感管理 TOTP、本人/员工/客户范围和标准 RP。观察告警真实送达、错误率、p95、撤销/登出延迟及 outbox；保存实际起止/样本/预算，不能预写99.9%或立即第三方退出承诺。
9. 交接所有者、值班/升级、支持/邀请核验、备份恢复和密钥管理，记录当前版本与非秘密配置引用。旧库/旧材料只按明确保留策略保存，不在本轮擅自删除。
10. 依据资源归属清理本轮临时进程/日志/故障文件/验证库/角色/目录；正式新库、正式报告和共享资源不在临时清理范围。若有 Docker，先后记录 df 且仅定向清理；不把逻辑字节删除称为 Windows VHDX 缩小。

上述步骤采用[全新安装](../../../fresh-installation.md)、[角色交接](../../../../ops/identity-deployment/README.md)、[当前部署合同](../../../deployment.md)和[canonical 恢复](../../../../ops/identity-recovery/README.md)。本文件没有执行步骤 1 之后的任何正式部署动作。

## 6. Phase 26 的硬前置

| 门禁 | 需要的真实事实或批准 | 当前状态 |
| --- | --- | --- |
| Q3 | 实际 reviewer/publisher 账号、责任和应用委派；申请人与审核分离；测试验收引用 | 缺失；`platform_auth_release_bindings` 必须保持默认关闭，fixture绑定不可移植 |
| Q5/Q6 | 可信邀请投递/身份核验渠道、支持流程、真实管理员 TOTP、两人恢复的独立授权人员 | 政策已定；正式人员/渠道及可用性未核实 |
| Q8 | 实际运营主体、法域/留境、合同/隐私与资料处置引用、审计/日志/备份保留责任 | 缺失；180/30天只是已采纳默认值，不能当作合规结论 |
| Q9 | 当前版本容量/故障/密钥/恢复证据；运维主责、安全升级和产品支持的实际联系渠道；正式备份/WAL与恢复作业 | 本计划不代填；须引用最终Phase25结果与真实值班表，达不到的承诺必须收紧或保持关闭 |
| Q10 | 本机/WSL/5432实际引擎、四个提议新库及角色范围、三域名/443/私有4443、服务管理/秘密引用、受影响服务及准确切换窗口 | 仅盘点与提案，尚未获针对实际对象的授权 |
| 候选版本 | 全部最终摘要、18类矩阵结果、未验证项及明确阻断、清理记录 | 由Phase25主报告提供；未完成前禁止正式切换 |

Q3绑定字段为 reviewer_user_ids、publisher_user_ids、policy_reference、operations_reference、verified_at。当前 runtime 只有 SELECT，不能从普通管理 UI自行启用。没有真实事实时保持关闭，不将环境管理员、测试账号或本机域名自动当作负责人。

最终针对性批准应只请求用户确认以下具体对象与事实；技术实现选择由执行器先写成可审阅清单，不把未定路径、端口、阈值或恢复命令留给用户自行设计。

- **部署范围**：当前 Windows + WSL Ubuntu 的 5432 引擎，准确四个新库、13 个新角色、三个本机域名、443/4443 监听及受影响的六个服务；旧库保留。最终发布目录、受保护秘密目录、服务管理和上游端口在请求批准前补齐。
- **版本与操作**：绑定最终 Phase 25 候选摘要及当前矩阵/清理报告，列明创建、迁移、权限交接、启动、唯一入口启用、观察窗口和失败关闭动作。任一项仍为待定时，不请求笼统“上线全部”许可。
- **运营事实**：真实审核/发布账号、独立恢复人员、邀请核验渠道、运维/安全/支持联系方式、政策与运营引用、正式备份/WAL 和业务库恢复责任。无法提供的门禁继续关闭，不复制 fixture 中的账号或政策绑定。

本次复核保留原始盘点时间，不将其改写成持续监控结论。当前 14 项来源文件 SHA-256 与 JSON 中记录一致，本文 6 个相对文件链接均可解析。最终候选仍未绑定；Phase 25 的正式报告、当前容量/恢复/告警结果及完整清理结论必须由唯一环境 owner 补齐。

## 7. 回退触发与整体恢复

任一错误引擎/库/owner/摘要、TLS/issuer/callback不一致、readiness失败、scope/成员越权、撤销来源复活、secret外泄、身份不可达时仍放行、MFA门禁绕过或标准客户端失败，均停止接流。正式观察窗口的延迟/错误率/撤销/登出/告警阈值必须在切换前绑定已测预算和责任人；本文件不凭空生成可用性承诺。

接流前失败时，入口保持关闭，候选新库与日志先隔离留证，修复后重新验收；旧库仍保留。接流后失败时，先关闭准确新入口/排空本轮进程并确认无双issuer，再由授权责任人选择修复当前候选或执行批准的整体恢复。不得启用已退役协议、旧token/checksum兼容、另一issuer、旧密钥终态复活或未经核实的旧进程。

共享认证库与门户必须使用当前 identity-recovery 的 capture/capture-deny、独立witness/floor、restore-new、reconcile、明确rearm与gate。恢复到新隔离目标，保留owner/ACL、墓碑/receipt/subject/撤销与签发高水位，轮换旧认证来源/凭据，再验证旧载体拒绝和新双RP业务；gate不自动开流量。不得执行通用旧 restore，它已在I/O前关闭。门店/工单业务库的恢复需要独立批准、当前基线与业务一致性流程；当前canonical身份工具不覆盖它们，缺少实际业务恢复方案仍是正式切换阻断项。

新库安装不删除旧库，不能据旧库存在就声称能安全回退旧authority。若最终整体回退方案或可恢复备份没有经过当前版本演练，Phase26保持未完成。

## 8. 本次交付与资源范围

本次产生本Markdown、对应脱敏JSON、[业务环境准备代码](../../../../tests/unified-auth-https/phase25-business-fixture.py)与一处密钥手册说明修正。准备代码的13项纯组件/归属负测通过，包括陌生集群/库/角色、缺manifest有未知资源、未标记同名角色、运行lease、损坏symlink和策略保留；它们没有调用真实DB或启动服务。5个临时组件测试目录及其文件/链接已由上下文结束清理。

准备代码支持 `prepare`、`cleanup-ui`、`cleanup`，仅供唯一环境owner在相同冻结版本中运行。它尚未在本子任务实际执行迁移或应用验收；实际结果由Phase25主报告记录。没有创建或复用容器，没有创建DB/角色、服务进程、日志、私钥、备份或服务运行目录；没有执行Docker清理，也没有Windows磁盘回收声明。Phase25实际验证资源由唯一环境owner记录和清理，不能由本计划代签。
