# Project Task Ledger

统一 OAuth / OIDC / SSO 与第三方接入任务台账。

版本：v2.5。创建日期：2026-10-27。已采纳开放问题建议；Phase 1–24 已完成。各阶段结论仅适用于其报告记录的输入与环境；旧阶段报告不代替当前全版本验收。Phase24退役、当前空库/权限、identity141+main1、平台432唯一分层覆盖及Nginx门禁与专属清理通过，见[Phase24报告](unified-auth/validation/phase-24.md)。当前主执行指针Phase25，正式切换仍属Phase26。

## 1. Project Overview

### Project Name

统一认证中心与客户端体系重构。

### Project Goal

将业务 OAuth 和统一登录 OIDC 合并为一套客户端登记及认证协议，由 `saas-identity` 提供唯一 OAuth/OIDC/SSO 权威；平台、自有应用和第三方应用使用同一体系，支持纯账号登录、平台业务身份登录和机器 API 调用。

### Background

唯一方案输入为[总体方案](unified-oauth-oidc-sso-architecture.md)，特别是第 4–11 节模型／协议合同、第 16 节全新安装约束、第 18 节验收矩阵与第 19 节待确认事项。

台账创建时，业务 `oauth_clients` 与 `/oauth/*` 主要由平台提供，登录 `oidc_clients` 与 `/oidc/*` 由身份服务提供。管理页面目前分两个标签，已有持久授权、单候选自动选择等局部能力；这些不是底层统一已经完成的证据。此前构建、测试、本机账号、数据库与截图不能自动计入本台账的新版本验收。

本台账是后续执行状态的唯一来源。总体方案提供设计依据，聊天历史只作背景；用户明确的新指令可覆盖台账，但执行者必须先更新受影响的约束、决策和验收要求。全部阶段初始为 `NOT_STARTED`，不得把既有局部能力直接标记为完成。

采用 26 个阶段，超过一般项目建议的 5–12 个：本项目涉及八个独立仓库、三类授权流程、两个 BFF 和独立资源服务，需要分别形成可审阅、可测试的终点。每阶段按一个执行代理一次连续工作划定；禁止通过“完成后端／完成集成”等大阶段掩盖多个交付周期。

### Success Criteria

- [ ] 只有一个有效客户端登记权威和对外 OAuth/OIDC 签发入口；旧两套运行路径已退役。
- [ ] 自有／第三方应用、一应用多客户端、人员／机器、纯账号／业务上下文维度分别生效。
- [ ] 无平台成员的有效账号能完成第三方 ACCOUNT_ONLY 登录；第三方不依赖内部 UUID 或平台数据库。
- [ ] code＋S256、ID Token、UserInfo、Access Token、refresh、introspection、revoke 和受控登出符合已声明合同。
- [ ] 持久同意按明确组／资源／scope 复用，权限扩大重新确认；撤销与删除不能复活旧凭据。
- [ ] 平台和门户保持独立会话及员工／客户隔离，成员、应用和资源权限的收缩能够使相应能力失效。
- [ ] 门店、工单、通知及宿主微应用完成目标凭据／上下文接入。
- [ ] 管理权限、第三方审核、凭据轮换、限流、审计、恢复及故障处理有当前证据。
- [ ] 完成全新安装、正式切换与外部接入验证；发布事实由 Phase 26 记录，候选版本不冒充已上线。
- [ ] 方案验收矩阵全部映射到具体测试，未通过项、受限项和清理情况如实记录。

---

## 2. Global Constraints

### Technical Stack

- 八个独立仓库，不改造为顶层 Cargo workspace：`saas-identity`、`saas-plateform`、`saas-customer-portal`、`saas-stores`、`saas-work-orders`、`saas-notification`、`saas-micro-app-sdk`、`saas-examples`。
- Rust 1.97.0、Axum、SQLx、PostgreSQL；React 19、TypeScript、Vite 6；Node 24.22.1、npm 11.16.0，以实际仓库锁文件和工具链配置核验。
- qiankun 2.10.16 与 SDK 3.0.0 为当前基线；改变契约须有显式版本及消费者验证，不借重构顺手升级依赖。
- 平台与身份共享物理 PostgreSQL、独立 owner/runtime；门店、工单、门户各自独立数据库。

### Architecture Constraints

- 最终由身份服务管理唯一客户端与标准 OAuth/OIDC；平台管理命令经受认证私有协议调用，不直读身份私有表。
- 平台继续拥有租户、组织、成员、业务上下文和资源授权；身份服务不复制角色菜单或直接查成员全集。
- 应用、客户端、合作方、资源、用户同意和业务权限是不同实体。合作方不等于业务租户，FIRST_PARTY 不等于自动全权限。
- 内部 ID 为 UUIDv7；对外 sub 为受审核 sector 下稳定 opaque 标识，不允许应用自行指定 sector 关联别的合作方用户。
- 单 issuer、显式 HTTPS 配置；公开网关拒绝私有接口。服务凭据按方向、用途和范围隔离，验证 CA，不关闭 TLS 校验。
- 阶段性新模块可以在隔离测试中逐步替换旧消费者；不能把旧签发路径作为目标环境的自动回退。临时保留的旧代码必须登记在 Phase 24 退役清单，不视为统一完成。
- `0001` 为 canonical 全新基线；不添加历史 checksum 接管、旧 UUID 搬迁、双读写兼容或运行时自动 DDL。

### Business Constraints

- 人员 USER 与机器 MACHINE 分客户端登记，不延续 `LEGACY_MIXED`。
- ACCOUNT_ONLY 不要求业务成员或组织选择；PLATFORM_CONTEXT 显式限定员工／客户、应用与资源资格。
- 单租户跳过租户页；选定租户内单个真实可用身份自动经同一服务端校验继续；搜索过滤不触发自动选择。
- 日常登录复用有效同意；新增资源／scope／实质用途需重新确认；标准显式 `prompt=consent` 与默认复用分开处理。
- 在线 refresh 受原 SSO／业务来源上限限制；真正离线 grant 首版限明确批准的 ACCOUNT_ONLY，不把浏览器自然到期等同于离线授权撤销。
- 首版以 openid/profile 和受控账号来源为最小接入范围；未确认的 email/phone、自助注册、MFA 强度和业务离线委托不自动开放。
- 不默认提供动态开放注册、SAML/SCIM、外部 IdP 代理、password/implicit grant、设备授权或 private_key_jwt；未来功能只有完成实施与验收后才能在元数据中声明。

### Security / Compliance Constraints

- ID Token 不调用业务 API；Access Token 必须有正确 audience、scope、主体、原授权与业务来源。
- 对外默认 opaque Access Token＋在线内省；内部 BFF 派生能力使用独立 token_use、受众及验证器。
- secret 摘要保存，仅创建／轮换时展示；不把真实秘密、token、Cookie、授权码、verifier、PHC、私钥或连接串写入报告／日志。
- code 一次性、S256、state/nonce/issuer 校验；refresh 轮换、重用 family 撤销、绝对期限和权限不扩张。
- 同意撤销、code 签发、refresh 与删除使用明确锁序／代际；活动记录不能直接删除，删除不恢复旧 grant，旧标签页不得覆盖新授权。
- 全局退出、当前应用退出、撤销同意、客户端停用、账号处置分别定义；不承诺第三方本地页面瞬间退出。
- 所有敏感写入保护来源、CSRF、管理权限、乐观版本、操作幂等和事务审计；未知提交结果查原 operation，不用新 ID 重放。
- 留存、账号注销、合作方资料删除义务、SLA/RTO/RPO 等由明确责任人确认，执行代理不得编造已批准政策。

### Code Style / Engineering Rules

- 每阶段先读取适用的工作区／上级／项目 `AGENTS.md`、README、Cargo/package 配置与锁文件，记录各受影响仓库 HEAD 和已有脏文件；不覆盖他人改动。
- 复用既有错误类型、协议、分页和命令 receipt；Rust 入口保持薄，避免生产路径 panic、无界队列或跨服务依赖业务实现 crate。
- 每阶段在所列验证文档记录：输入版本、变更文件、执行命令、结果、验证层级、未验证边界、风险、临时资源和清理；文档相对链接须可解析。
- 库／协议／并发修复需有效回归。Rust 定向 fmt、check、clippy、测试按修改范围执行；前端类型／构建、相关组件与真实浏览器检查按范围执行。只读、文档阶段不运行无关部署测试。
- HTTP fixtures、真实数据库、跨服务 HTTPS、浏览器、第三方客户端分别标明，不能互相冒充。既有报告不能当新阶段的通过证据。
- 默认使用本地工具和适用既有服务；测试前检查是否间接创建 Docker 资源。创建前记录项目、预算、context、已有资源和标识；依赖容器也计入预算。
- 必须在成功／失败／中断时清理本阶段临时集群、测试库、角色、容器、日志、卷、网络、专用镜像和无用缓存。仅按归属定向清理，保留业务数据和正式报告。
- Docker 操作清理前后核对 `docker system df`。删除 Docker 资源或 Linux 文件不等于 Windows VHDX 已缩小；停止 WSL/Docker 或压缩磁盘需核实影响及授权。
- `NOT_STARTED`、`IN_PROGRESS`、`BLOCKED`、`COMPLETED` 是本台账允许的状态。完成阶段后同步摘要、详情、指针、决策、问题和变更日志；没有验收证据不能标记完成。

### Forbidden Actions

- 创建台账不等于执行 Phase 1；执行指定阶段不等于授权自动完成所有后续阶段。
- 总体方案、台账及此前本机清库授权不构成未来任意数据库删除授权。在线服务变更、已有库重置、远端发布须有对应任务的明确授权与目标核对。
- 不引入本地密码登录回退、旧认证入口 fallback、共享万能 secret、未登记回调／资源、任意 issuer 或关闭 TLS。
- 不以合并两张前端列表代替合并登记与协议；不以标准登录成功代替资源／成员权限验收。
- 不在正式环境验证半成品，不让两套实现争用同一 issuer，不使用全局 Docker prune 或删除虚拟磁盘代替定向清理。
- 不将建议参数、临时 fixture、示例域名或当前本机凭据当作生产决策。

### Execution and Evidence Convention

以下均为未来预期产物路径，本次没有创建这些实施文件：

- 合同目录：`saas-plateform/backend/docs/designs/unified-auth/`。
- 每阶段报告：上述目录下 `validation/phase-NN.md`，NN 为两位阶段号；报告中引用实际提交／文件与可复现命令。
- 专属 fixture／运行秘密放任务私有目录并在结束清理，不提交仓库。
- 开始阶段先核验前置阶段报告；缺少证据的前置不因状态表写了完成就被自动接受。只执行本次明确指定的阶段，完成后更新指针而不自动启动下一阶段。
- 如某阶段实际仍超出一个连续执行单元，在写实现前拆分并记录变更；不得先实施一半再请求用户反复“继续”才补齐该阶段。

---

## 3. Phase Status Summary

| Phase | Name | Status | Depends On | Output Summary |
|---|---|---|---|---|
| 1 | 冻结统一实施合同与调用清单 | COMPLETED | - | [合同](unified-auth/implementation-contract.md)、[调用清单](unified-auth/callsite-inventory.md)、[Phase 01 报告](unified-auth/validation/phase-01.md) |
| 2 | 建立统一空库模型与角色隔离 | COMPLETED | 1 | [canonical 结构与 Phase 24 退役清单](unified-auth/phase-24-retirement-list.md)、[Phase 02 报告](unified-auth/validation/phase-02.md) |
| 3 | 实现应用／合作方认证投影协议 | COMPLETED | 2 | [最小投影与 Phase 03 报告](unified-auth/validation/phase-03.md) |
| 4 | 实现统一客户端管理与凭据操作 API | COMPLETED | 3 | [统一 API 与 Phase 04 报告](unified-auth/validation/phase-04.md) |
| 5 | 实现统一客户端管理页面 | COMPLETED | 4 | [统一注册簿页面与浏览器回归](unified-auth/validation/phase-05.md) |
| 6 | 实现 pairwise subject 与资料投影 | COMPLETED | 2 | [subject 映射、最小声明与 PostgreSQL 回归](unified-auth/validation/phase-06.md) |
| 7 | 实现同意组与用户授权管理 | COMPLETED | 4, 6 | [组级持久同意、撤销/删除与 PostgreSQL 回归](unified-auth/validation/phase-07.md) |
| 8 | 统一人员授权码与 OIDC 签发 | COMPLETED | 4, 6, 7 | [人员 OAuth/OIDC、TOTP 与 PostgreSQL 回归报告](unified-auth/validation/phase-08.md) |
| 9 | 实现资源限定 Access Token 与内省撤销 | COMPLETED | 8 | [资源 scope、内省、撤销与 UserInfo PostgreSQL 回归报告](unified-auth/validation/phase-09.md) |
| 10 | 实现在线与离线 refresh 生命周期 | COMPLETED | 9 | [轮换、重放、期限与来源 PostgreSQL 回归报告](unified-auth/validation/phase-10.md) |
| 11 | 接入机器客户端凭证流程 | COMPLETED | 9 | [MACHINE grant 与资源范围](unified-auth/validation/phase-11.md) |
| 12 | 接入统一客户端的业务上下文交互 | COMPLETED | 8, 9, 10 | [纯账号分流、业务选择与一次性结果](unified-auth/validation/phase-12.md) |
| 13 | 适配管理平台 BFF | COMPLETED | 10, 12 | [实际 TLS 1.3 浏览器 8/8、初始账号代际与员工来源回归](unified-auth/validation/phase-13.md) |
| 14 | 适配客户门户 BFF | COMPLETED | 10, 12 | [三服务真实 HTTPS 10/10、原能力恢复、续期及员工/客户隔离](unified-auth/validation/phase-14.md) |
| 15 | 固定平台资源与内部派生能力边界 | COMPLETED | 9, 12, 13 | [独立内部/外部验证器、真实角色及资源/目录 HTTP 矩阵](unified-auth/validation/phase-15.md) |
| 16 | 适配门店 API | COMPLETED | 15 | [严格 USER/issuer/scope、私有 STORE 目录、92 普通及 5/5 PostgreSQL 验收](unified-auth/validation/phase-16.md) |
| 17 | 适配工单 API | COMPLETED | 15 | [81普通/1文档/58真实PG、定向目录14/14；独立USER/MACHINE与私有目录](unified-auth/validation/phase-17.md) |
| 18 | 贯通宿主、SDK 与通知上下文变化 | COMPLETED | 13, 14, 16, 17 | 两类P2修复终审；宿主476/浏览器12、通知PG1/工单15，47文件摘要及清理通过 |
| 19 | 落地合作方审核与生产接入流程 | COMPLETED | 3, 4, 5, 7, 8 | [真实 TOTP/审核/邀请/受控生产 HTTPS 13/13；组件与清理均通过](unified-auth/validation/phase-19.md)，实际运营责任未核实仍关闭 |
| 20 | 实现登出 outbox 与 back-channel logout | COMPLETED | 7, 9, 10, 13, 14, 19 | 真实TLS13/13；INFO同套日志门禁通过、永久失败告警可见且无秘密；隔离资源清理0残留 |
| 21 | 交付独立第三方 OIDC 接入示例 | COMPLETED | 8, 9, 10, 11, 19, 20 | 追加容量P2修复：46组件+独立复审+最终快照实际TLS15/15，专属资源0残留 |
| 22 | 完成认证运行监控与有界故障处理 | COMPLETED | 8, 9, 10, 11, 20 | lib87/独立PG6/观测PG2/Node26/实际HTTPS8，容量实测及6告警送达，秘密0/资源0残留 |
| 23 | 验证密钥生命周期与备份恢复 | COMPLETED | 22 | [同构建实际密钥组＋最终恢复8/8、组件387、RPO26.463秒/RTO98.942秒及清理](unified-auth/validation/phase-23.md) |
| 24 | 退役旧客户端与旧协议运行路径 | COMPLETED | 5, 13, 14, 15, 16, 17, 18, 19, 20, 21, 23 | [单权威/新空库103、identity141+1、平台432分层覆盖、Nginx及专属清理](unified-auth/validation/phase-24.md) |
| 25 | 完成发布候选全新安装与验收矩阵 | IN_PROGRESS | 24 | 端到端验收、部署与回退候选清单 |
| 26 | 执行获授权的正式切换与外部开放 | NOT_STARTED | 25 | 真实发布、切换／回退证据及运营交接 |

允许状态：`NOT_STARTED`、`IN_PROGRESS`、`BLOCKED`、`COMPLETED`。依赖是完成门禁，不是并行代理授权。

---

## 4. Phase Details

### Phase 1: 冻结统一实施合同与调用清单
#### Status
COMPLETED
#### Goal
把总体方案转为可直接指导下一阶段的技术合同，明确当前调用者与目标替换点。
#### Scope
盘点两套登记／路由／令牌消费者；固定应用、客户端、资源、subject、同意、凭据 DTO、状态机、管理权限、scope 和撤销语义。
#### Out of Scope
不修改运行代码、schema、在线配置或数据库，不部署服务。
#### Inputs
总体方案全文；本台账全局约束；八仓库当前源码、README、锁文件及适用 AGENTS；第 7 节问题表。
#### Execution Requirements
记录真实 HEAD／脏文件及当前调用点；逐项映射方案差异。未确认的产品政策写明默认建议、责任人和阻塞阶段，不伪造批准；技术合同可排除未批准能力后形成可实施最小范围。
#### Expected Outputs
`unified-auth/implementation-contract.md`、`unified-auth/callsite-inventory.md`、Phase 01 报告；更新问题与决策记录。
#### Acceptance Criteria
- [x] 所有当前客户端表、协议入口、BFF／业务／通知消费者均有源码定位和替换责任阶段。
- [x] USER/MACHINE、ACCOUNT_ONLY/PLATFORM_CONTEXT、subject sector、同意组与在线／离线期限不相互混淆。
- [x] DTO、错误码、锁序、版本、幂等／未知结果恢复合同有示例；无真实秘密。
- [x] 每个待确认项已有影响与阻塞门禁，后续阶段无需凭聊天猜测。
#### Risks
把总体设计建议误当政策批准；遗漏依赖旧 UUID-sub 或旧 OAuth secret 的消费者。
#### Dependencies
无前置阶段；读取和编写合同不依赖生产凭据。
#### Notes
这是首个可执行阶段；台账创建阶段未执行，后续已按授权完成。

#### Completion Record
2026-10-27：完成源码与八仓库 HEAD/脏状态盘点，冻结 DTO、命令收据、错误、版本、锁序、权限与调用替换合同。详见 `unified-auth/validation/phase-01.md`。

### Phase 2: 建立统一空库模型与角色隔离
#### Status
COMPLETED
#### Goal
使空库能建立统一身份模型并通过最小权限验证。
#### Scope
统一客户端、凭据、资源／scope、应用投影、同意组、subject 和目标 grant 所需 schema；canonical `0001`、provision／verify、readiness。
#### Out of Scope
不接管旧库、不搬迁数据、不修改正式运行环境；不提前宣称旧表已退役。
#### Inputs
Phase 1 合同与清单；平台 migrations、身份部署脚本和当前 readiness／数据库测试。
#### Execution Requirements
按域分 owner/runtime；唯一基线初始化。为尚未替换的源码所需暂存结构登记 Phase 24 删除条目，仅用于阶段开发验证；不建立新旧双写。
#### Expected Outputs
基线与权限脚本变更、空库／约束／双角色允许和拒绝测试、Phase 02 报告。
#### Acceptance Criteria
- [x] 专属空库成功初始化，重复校验同一版本不重写历史。
- [x] 所有目标实体及关键唯一键／版本／引用约束有真实 PostgreSQL 测试。
- [x] 平台不能读写身份秘密，身份不能读取业务成员／角色；runtime 不能 DDL 或切换 owner。
- [x] schema 与角色验证命令、临时结构退役清单及失败清理可复现。
#### Risks
跨域 FK／ACL 扩权；增加 schema 后旧消费者仍需替换。
#### Dependencies
Phase 1；后续测试使用任务专属空库和明确管理连接。
#### Notes
只完成数据基础，不完成对外协议；不清理未授权的既有库。

#### Completion Record
2026-10-28：canonical `0001` 加入目标 `auth_*` 模型；identity owner/runtime 获得必要权限，platform runtime 被拒绝访问；真实 PostgreSQL 空库、重复迁移、约束与双 LOGIN-role 验证通过。旧消费者结构登记于 Phase 24 退役清单。详见 `unified-auth/validation/phase-02.md`。

### Phase 3: 实现应用／合作方认证投影协议
#### Status
COMPLETED
#### Goal
让认证中心取得经管理权限批准的最小应用与责任主体信息。
#### Scope
合作方／应用最小认证投影、审核状态、版本同步、operation ID／receipt／结果查询及停用传播。
#### Out of Scope
不实现完整合作方运营界面，不复制业务角色或租户数据。
#### Inputs
Phase 1 投影合同、Phase 2 模型；当前平台应用管理和私有管理命令模式。
#### Execution Requirements
通过认证私有 API 操作身份域；固定目标应用、责任主体、环境及权限范围。未知提交结果查询原操作；收缩未确认时失败关闭。
#### Expected Outputs
投影生产者／消费者、幂等与收缩恢复测试、Phase 03 报告。
#### Acceptance Criteria
- [x] 创建、更新、停用和版本冲突有可复现合同测试。
- [x] 超时后结果查询不重复执行，不借新 operation 绕过绑定。
- [x] 错误服务用途、应用范围及旧版本请求拒绝。
- [x] 合作方归属不会产生任何业务租户管理权限。
#### Risks
跨服务状态不一致被误报为停用完成。
#### Dependencies
Phase 2；管理责任划分由 Phase 1 固定。
#### Notes
人工审核流程的数据通道在此完成，运营页面在 Phase 19。

#### Completion Record
2026-10-28：完成身份私有投影 API、平台受权限约束的 first-party 投影适配、独立 `APPLICATION_PROJECTION` 服务凭据、版本/收据/原 operation 恢复和停用传播测试。第三方投影只能保持 `PENDING_REVIEW`；审核账号与运营流程仍由 Phase 19 落地，当前不宣称生产第三方接入已开放。详见 `unified-auth/validation/phase-03.md`。

### Phase 4: 实现统一客户端管理与凭据操作 API
#### Status
COMPLETED
#### Goal
用一个管理服务登记所有自有和第三方客户端。
#### Scope
统一创建、读取、编辑、状态变更、稳定分页／筛选；客户端类型组合校验、回调、资源权限及版本；凭据创建、双版本轮换、撤销。
#### Out of Scope
不实现公开动态注册，不更改应用归属或把 USER 原地改为 MACHINE。
#### Inputs
Phase 3 应用投影；Phase 1 客户端与权限合同；Phase 2 schema；当前命令审计／receipt 实现。
#### Execution Requirements
复用私有管理模式与平台权限代理。列表不返回秘密；普通修改不越过应用审批上限；轮换结果仅交付一次并有明确过渡窗口。
#### Expected Outputs
统一管理 API／代理、查询与写入回归、凭据生命周期测试、Phase 04 报告。
#### Acceptance Criteria
- [x] 自有／第三方及人员／机器使用同一客户端权威，非法组合拒绝。
- [x] 分页元数据与稳定排序正确，权限相同，越界页处理明确。
- [x] 凭据旧／新窗口和紧急撤销生效；重放不泄漏或生成额外 secret。
- [x] 越权、CSRF／操作绑定、版本冲突及审计失败不会留下半完成写入。
#### Risks
用户侧公开类型持有秘密；凭据轮换误伤其他客户端。
#### Dependencies
Phase 3；轮换窗口未确认时采用测试参数，生产能力受 Q7 门禁限制。
#### Notes
通过 API 交付的凭据功能在 Phase 5／19 呈现，不能重复建设第二套 secret 系统。

#### Completion Record
2026-10-28：完成身份统一客户端 API、独立 `AUTH_ADMIN` 平台代理、稳定分页/筛选、应用 scope ceiling、版本冲突、凭据 24 小时重叠与紧急撤销；真实 PostgreSQL 回归证明 operation 查询/重放及审计失败回滚。详见 `unified-auth/validation/phase-04.md`。

### Phase 5: 实现统一客户端管理页面
#### Status
COMPLETED
#### Goal
用单一列表和适用字段表单管理统一登记。
#### Scope
应用／合作方／环境／类型／状态筛选、服务端分页、详情、合法类型创建与编辑、版本冲突、一次性凭据展示与轮换操作。
#### Out of Scope
不再保留业务／登录两套永久列表；不替代合作方申请审批页面。
#### Inputs
Phase 4 API；平台 ClientsPage、应用管理、公共 Dialog／分页和权限组件。
#### Execution Requirements
按运行类型展示字段；敏感修改解释影响；不能回填秘密。沿用固定操作栏、视口约束、键盘与移动端滚动规则。
#### Expected Outputs
统一列表／表单／API 类型、组件与真实浏览器回归、Phase 05 报告。
#### Acceptance Criteria
- [x] 管理平台、门户和第三方登记在同一列表可查，筛选与页码恢复正确。
- [x] PUBLIC／CONFIDENTIAL、USER／MACHINE／上下文选项无法提交非法组合。
- [x] 读失败不清空旧配置，重复提交／并发版本冲突有明确处理。
- [x] 桌面和 390px 窗口能访问全部表单及操作；无秘密进入持久浏览器存储。
#### Risks
旧 UI DTO 或缓存仍请求两套 API；失败回填覆盖正确配置。
#### Dependencies
Phase 4。
#### Notes
这阶段只完成管理产品的统一，协议统一仍依赖后续阶段。

#### Completion Record
2026-10-28：统一客户端管理页切换到 Phase 4 身份 API，覆盖所有者／环境／类型筛选、URL 服务端分页、类型化表单、版本冲突重载、失败读取保留、原 operation 恢复及一次性 secret 展示。Chromium 实测含 390px 窗口；全量前端测试、生产构建与 Phase 5 浏览器回归通过。详见 `unified-auth/validation/phase-05.md`。

### Phase 6: 实现 pairwise subject 与资料投影
#### Status
COMPLETED
#### Goal
为第三方提供稳定、隔离且不暴露内部 UUID 的账号标识。
#### Scope
受审核 sector、subject 映射、资料 scope 投影、UserInfo／ID Token 共用的声明构造边界。
#### Out of Scope
不开放跨合作方自动账号关联，不实现邮箱匹配自动合并或未经批准的资料 scope。
#### Inputs
Phase 1 subject／资料合同；Phase 2 schema；当前账号资料与认证证明。
#### Execution Requirements
持久映射保证更名／凭据和签名密钥轮换不改变 sub；原账号注销后不复用。声明构造接受已验证账号证明，不接受浏览器声明内部 ID。
#### Expected Outputs
subject 映射服务、最小资料投影、隔离／稳定性测试、Phase 06 报告。
#### Acceptance Criteria
- [x] 同 sector 稳定、不同 sector 隔离；未审核共享请求拒绝。
- [x] ID Token／UserInfo 投影得到一致 sub，内部用户 ID 仅在私有证明存在。
- [x] 未同意／未实现字段不返回，验证状态不伪造。
- [x] 账号注销后映射不被新账号继承。
#### Risks
现有 BFF 把 sub 解析为 UUID；须由 Phase 13／14 替换，不加隐式回退。
#### Dependencies
Phase 2；Q2／Q4 限制跨应用共享与敏感字段范围。
#### Notes
此处可用可信证明 fixture 测试投影，标准 HTTP UserInfo 在 Phase 8／9 接通。

#### Completion Record
2026-10-28：新增可信 `AccountSessionProof` 驱动的 subject resolver；仅批准的 USER 客户端可建立 app/environment sector，subject 持久且随机，账号退役事务封存映射。ID Token 与 UserInfo 使用同一最小声明 builder；真实 PostgreSQL 回归覆盖隔离、并发首次创建、重命名／签名密钥轮换稳定性、scope 投影和注销后不继承。详见 `unified-auth/validation/phase-06.md`。

### Phase 7: 实现同意组与用户授权管理
#### Status
COMPLETED
#### Goal
使应用同意按明确组、资源与 scope 持久保存并可安全撤销／删除。
#### Scope
同意覆盖、组成员登记、授权代际、本人管理 API 与页面、撤销／活动删除拒绝、审计与失效事件。
#### Out of Scope
不把管理员扩权当用户已同意，不实现登出网络投递。
#### Inputs
Phase 4 客户端及权限登记、Phase 6 账号标识合同；现有持久授权／撤销页面及并发测试。
#### Execution Requirements
按账号与同意组固定锁序。撤销同意、相关 grant／refresh／未完成请求与失效 outbox 事务一致；真正投递由 Phase 20 完成。
#### Expected Outputs
同意服务与页面、scope／组／版本／撤销并发回归、Phase 07 报告。
#### Acceptance Criteria
- [x] 首次同意、后续复用、新增范围重确认、组外不能继承均通过。
- [x] 明确组内所有适用客户端来源被撤销，其他应用不受影响。
- [x] 活动记录及其他账号记录不可删，删除不复活 grant；重新授权后旧删除请求冲突。
- [x] 退出登录与组织切换保留同意，敏感写入满足来源／CSRF／账号锁校验。
#### Risks
错误共享同意；并发兑换在撤销后提交旧代际。
#### Dependencies
Phase 4、6；Q1 已确定显式同应用共享，隔离／共享条件按政策决策验收。
#### Notes
不能沿用“每个 RP 永远独立”旧实现而宣称已满足组级撤销。

#### Completion Record
2026-10-28：完成 canonical consent group context/membership version、用户同意覆盖服务、组级撤销与 outbox、软删除及原操作 receipt；平台注册簿支持创建组和显式登记成员。成员变化推进 group version，旧 consent 在用户再次确认前不适用于新成员。真实 PostgreSQL 路由回归、canonical `0001`/owner/runtime 交接验证、431 个前端测试和 7 个 Chromium 场景通过。详见 [Phase 07 报告](unified-auth/validation/phase-07.md)。执行指针移至 Phase 8。

### Phase 8: 统一人员授权码与 OIDC 签发
#### Status
COMPLETED
#### Goal
在认证中心完成统一的人员登录标准协议链路。
#### Scope
单 issuer 发现文档、authorize、code＋S256、客户端认证、ID Token、UserInfo 接口与声明、标准提示和错误语义；实现 TOTP、二次认证、一次性恢复码及受控管理员恢复。公共自助注册保持关闭，账户仍由受权限保护的平台账号管理路径创建。
#### Out of Scope
不实现机器 grant 或 refresh family；不发布未实现的设备／签名客户端能力。Q5 的一次性 24 小时邀请签发/授权与已验证渠道投递分别列入 Phase 19、20，Phase 8 不宣称邀请链接已可用。
#### Inputs
Phase 4 客户端、Phase 6 声明、Phase 7 同意；当前 SSO 和 OIDC code 实现。
#### Execution Requirements
复用成熟 SSO 和密钥机制，合并协议而非新开第二授权服务器；校验精确回调、state/nonce/issuer/PKCE，固定标准提示语义。
#### Expected Outputs
统一公开路由、元数据、人员 code／ID Token／UserInfo 测试、Phase 08 报告。
#### Acceptance Criteria
- [x] ACCOUNT_ONLY 的无业务成员账号完成标准登录，无组织查询。
- [x] 错误回调／verifier／nonce／issuer、code 重放和并发双兑换拒绝。
- [x] PUBLIC 不持 secret，CONFIDENTIAL 正确认证；ID Token／UserInfo sub 一致。
- [x] prompt=login/none/consent、max_age 与持久同意语义分别有测试，元数据仅声明已实现能力。
- [x] TOTP 绑定、重放／猜测防护、加密密钥、恢复码一次使用及管理员恢复双人核验通过；账号恢复撤销旧凭据，未实现强度不声明。
#### Risks
混淆标准强制同意与默认授权复用；错误验证 opaque sub。
#### Dependencies
Phase 4、6、7。
#### Notes
后续 Phase 9 完成资源 token 的最终校验语义，Phase 24 退役旧公开入口。

#### Completion Record
2026-10-28：完成 canonical `/oauth/*` discovery、authorize/code、S256、ID Token 和 UserInfo；协议已消费 Phase 6 pairwise subject 与 Phase 7 group-version-aware consent。实现 TOTP 加密存储、重放/猜测防护、恢复码及双人管理员恢复。Phase 8 PostgreSQL HTTP 回归 2/2 通过；canonical `0001`/owner-runtime 边界回归 1/1 通过。详见 [Phase 08 报告](unified-auth/validation/phase-08.md)。Q5 一次性邀请签发授权已列入 Phase 19，验证渠道投递列入 Phase 20；本阶段仍关闭公开注册。执行指针移至 Phase 9。

### Phase 9: 实现资源限定 Access Token 与内省撤销
#### Status
COMPLETED
#### Goal
使访问令牌绑定正确资源、scope、主体及持久授权来源。
#### Scope
opaque Access Token、资源／scope 上限、受认证 introspection、标准 revoke 与 UserInfo token 校验。
#### Out of Scope
不增加第二种对外 JWT Access Token profile，不代替业务资源授权。
#### Inputs
Phase 8 人员 grant、Phase 1 资源合同、已登记应用／客户端权限与同意代际。
#### Execution Requirements
摘要存储；内省受调用方资源范围限制；校验账号、客户端、同意及原授权代际。注销／撤销与 token 的自然到期分开。
#### Expected Outputs
签发／内省／撤销接口、资源混用与失效回归、Phase 09 报告。
#### Acceptance Criteria
- [x] 错误 audience、越界 scope、无关内省者、ID Token 调 API 被拒绝。
- [x] 撤销后在线校验失效，来源不可验证返回约定失败而不放行。
- [x] 记录／日志无 token 明文，撤销一个来源不误伤无关授权。
- [x] UserInfo 仅在合法 token 的同意范围内返回资料。
#### Risks
将裸 read/write 解释为全部资源权限；以 token 签名代替来源校验。
#### Dependencies
Phase 8。
#### Notes
业务成员与数据范围由 Phase 12／15–17 继续执行。

#### Completion Record
2026-10-28：扩展 canonical `/oauth/token` 对已登记资源 scope 的授权，签发仍为 opaque digest 并持久绑定 resource/audience；加入 Basic 认证的 `/oauth/introspect`、客户端自有 token 的 `/oauth/revoke`，并使 UserInfo 重验完整授权来源。PostgreSQL HTTP 回归验证 scope/audience 隔离、pairwise `sub`、撤销及会话来源失效。复跑发现并修复同意只读检查中的账号锁升级死锁；P8 并发兑换回归修复后完整通过并另重复 3 次。详见 [Phase 09 报告](unified-auth/validation/phase-09.md)。机器 `client_credentials` 签发仍由 Phase 11 完成；执行指针移至 Phase 10。

### Phase 10: 实现在线与离线 refresh 生命周期
#### Status
COMPLETED
#### Goal
在不扩大原授权的前提下，正确区分在线会话续期和真正离线授权。
#### Scope
refresh family、轮换、重用检测、并发刷新、scope 收缩、绝对／空闲期限、在线／离线来源分支。
#### Out of Scope
首版不开放 PLATFORM_CONTEXT 的离线用户委托。
#### Inputs
Phase 9 grant/token、总体方案 10.2／10.4、Phase 7 撤销代际与 Phase 1 期限合同。
#### Execution Requirements
在线续期不超过 SSO／业务来源；仅明确批准的 ACCOUNT_ONLY offline grant 可独立到期。浏览器自然到期不等于撤销离线 grant，显式全局注销及安全代际变化仍须拒绝。
#### Expected Outputs
refresh handler／存储、并发与期限矩阵、Phase 10 报告。
#### Acceptance Criteria
- [x] 轮换和重用 family 撤销、并发、过期、绝对上限均有真实数据库测试。
- [x] refresh 不能增加 scope、替换失效成员或修复旧版本。
- [x] 在线 BFF 不靠错误申请 offline_access 获得越界续期。
- [x] 离线 ACCOUNT_ONLY 不错误依赖原浏览器始终未到期，撤销事实仍生效。
#### Risks
7 天 family 与 12 小时 SSO 被混成同一时钟，导致无限续期或伪离线能力。
#### Dependencies
Phase 9；生产期限依赖 Q7，测试必须显式设定边界。
#### Notes
参数建议不视为生产配置批准。仅注册、应用批准且用户明确同意 `offline_access` 的 ACCOUNT_ONLY grant 可使用离线期限；当前不接收 PLATFORM_CONTEXT refresh。

### Phase 11: 接入机器客户端凭证流程
#### Status
COMPLETED
#### Goal
使 MACHINE 客户端经统一 token endpoint 获取受控 API 能力。
#### Scope
client_credentials、凭据状态／版本、机器主体、audience/scope 和明确的数据范围引用。
#### Out of Scope
不伪造用户、发放 ID Token／用户 refresh，或隐式实现代表用户委托。
#### Inputs
Phase 9 令牌与资源校验、Phase 4 凭据管理、应用审核上限。
#### Execution Requirements
与人员客户端共享登记和签发基础，保持主体与授权方式互斥；服务账号范围从批准记录取得而非客户端临时声明。
#### Expected Outputs
MACHINE token 分支、类型／凭据／资源隔离测试、Phase 11 报告。
#### Acceptance Criteria
- [x] 合法机密客户端获得正确机器受众与 scope。
- [x] PUBLIC、USER、停用凭据、越界资源、未批准数据范围均拒绝。
- [x] 机器 token 无人员业务身份，不能被员工接口当人使用。
- [x] 凭据紧急撤销及版本来源校验实际生效。
#### Risks
延续 LEGACY_MIXED 或用机器请求参数冒充账号。
#### Dependencies
Phase 9。
#### Notes
设备授权不是本阶段范围。

#### Execution Record
2026-10-28：完成 `client_credentials` MACHINE 分支、凭据版本绑定与在线复验；scope 同时受应用审批上限和客户端资源权限约束。PostgreSQL HTTP 回归覆盖正确 audience/scope、PUBLIC/USER 拒绝、跨资源权限拒绝、未批准 scope 拒绝、UserInfo 拒绝、无人员声明、凭据轮换重叠、版本来源与紧急撤销；canonical `0001` 回归验证机器 grant 必须绑定已登记凭据版本。详见[Phase 11 报告](unified-auth/validation/phase-11.md)。执行指针移至 Phase 12。

### Phase 12: 接入统一客户端的业务上下文交互
#### Status
COMPLETED
#### Goal
把平台身份选择接到统一授权流程，保持纯账号与业务身份明确分流。
#### Scope
上下文事务绑定、私有来源验证、候选／确认、一次性结果、code 兑换扩展及原结果恢复合同。
#### Out of Scope
不替两个 BFF 建本地会话，不在浏览器传递选择凭据。
#### Inputs
Phase 8–10 统一 grant 与来源；当前 context-selection 模块及合同。
#### Execution Requirements
绑定 RP／回调／state／nonce／PKCE／原会话；候选按业务应用与成员用途过滤。无成员拒绝，单项自动仍走同一后端校验，refresh 不重复交付一次性结果。
#### Expected Outputs
新客户端上下文适配、来源／候选／确认／消费合同与并发回归、Phase 12 报告。
#### Acceptance Criteria
- [x] ACCOUNT_ONLY 不调用平台成员服务，PLATFORM_CONTEXT 无法绕过业务资格。
- [x] 零／一／多租户组织、员工客户、搜索不自动确认均验证。
- [x] 交互替换、来源撤销、重复签发／消费、错客户端／用途拒绝。
- [x] 超时恢复原结果，不重新签发；提交前校验原始来源期限与版本。
#### Risks
私有双向调用形成重入锁死，或错误拿当前授权修复原来源。
#### Dependencies
Phase 8、9、10。
#### Notes
复用专用服务身份和私网 HTTPS；不得用公开控制台 origin 代替私有入口。

#### Execution Record
2026-10-28：canonical `/oauth/*` 上下文交互、`ACCOUNT_ONLY` 零成员服务调用、业务资格与用途过滤、来源/交互版本栅栏、一次性 code 交付及加密超时恢复完成。身份 Chromium 11 个场景、身份 PostgreSQL HTTP 回归、平台上下文 HTTP/事务、员工回调、客户跨库恢复和真实角色隔离回归通过；`cargo fmt --check`、两服务 `cargo check --all-targets` 与 `git diff --check` 通过。详见[Phase 12 报告](unified-auth/validation/phase-12.md)。临时 PostgreSQL 集群已停止并删除，Docker 未创建资源。执行指针移至 Phase 13。

### Phase 13: 适配管理平台 BFF
#### Status
COMPLETED
#### Goal
管理平台通过统一认证协议恢复真实员工会话。
#### Scope
发现文档与端点、code 校验、opaque sub／私有账号证明、服务端凭据、上下文事务消费、在线 refresh 与员工会话。
#### Out of Scope
不改客户门户 Cookie，不增加本地密码登录。
#### Inputs
Phase 10／12 合同；平台 bff、session、前端登录恢复和会话版本模块。
#### Execution Requirements
内部 UUID 只来自受认证证明；选择消费与员工会话同事务；异步响应带逻辑会话边界，不能覆盖更新的账号或组织。
#### Expected Outputs
平台 BFF／前端适配、真实 HTTPS 登录／续期／切换测试、Phase 13 报告。
#### Acceptance Criteria
- [x] 当前管理员和普通员工在专属新环境完成登录、会话恢复和在线续期。
- [x] code／选择失败不留下半完成员工会话，客户身份不能建立员工会话。
- [x] 旧标签页／延迟响应不覆盖新版本，来源撤销后会话拒绝。
- [x] 浏览器不持上游 refresh，部署配置不依赖旧入口 fallback。
#### Risks
旧 sub 解析／token 字段造成表面登录成功但业务会话无效。
#### Dependencies
Phase 10、12。
#### Notes
业务内部能力面向资源的完整合同在 Phase 15 验证。

#### Completion Record
2026-10-28：固定源码快照 `17d9fa7399571282d44f3d3ff5dd5e354773753bdebf514044b4d6f9cea0341a`，实际 Linux 身份/平台与真实 owner/runtime 新库经 Chromium/TLS 1.3 完成 8/8 验收，含真实 API 新建账号 `0→1` 后登录、Cookie/页面恢复、refresh family 轮换、中央切换、客户拒绝和原来源撤销。修复 AUTH 私有目标遗漏及 SELECT-only 服务登记的行锁权限；没有放宽 runtime 写凭据权限。测试边界状态已合法清理，浏览器已退出；共享本次 PG/网关供并行阶段复用，最终清理仍待执行。详见 [Phase 13 报告](unified-auth/validation/phase-13.md)。

### Phase 14: 适配客户门户 BFF
#### Status
COMPLETED
#### Goal
客户门户保持独立数据库和会话，通过统一认证取得本人的客户身份。
#### Scope
新协议与 subject 适配、独立 Cookie、客户选择消费、跨库 pending／结果恢复、在线续期和本人资料。
#### Out of Scope
不共享员工 token、Cookie 或业务权限，不实现门户管理员功能。
#### Inputs
Phase 10／12 合同；门户 backend/frontend、独立基线和客户资源证明。
#### Execution Requirements
原操作在服务端加密持久化；未知结果只查询相同原结果。客户 proof 与本地 BFF ID／版本绑定。
#### Expected Outputs
门户前后端适配、跨库故障恢复／客户隔离／真实浏览器报告、Phase 14 报告。
#### Acceptance Criteria
- [x] 客户登录、单候选自动进入、切换及本人资料完成真实 HTTPS 验证。
- [x] 平台已提交而本地写失败可恢复同一能力，不重复消费或建立新上下文。
- [x] 错会话、错版本、员工身份及已撤销来源被拒绝。
- [x] 门户与员工会话互不覆盖，跨库故障触发器及临时身份状态已清理；任务环境保留供后续阶段复用并统一清理。
#### Risks
把 401 当服务不可用继续恢复；使用新的 token／会话代替原始来源。
#### Dependencies
Phase 10、12。
#### Notes
本阶段不得通过读平台账号／成员表解决隔离问题。

#### Completion Record
2026-10-28：固定快照 `1ce73c2a960bc2b9acfcb760a1fea86d04b671a0dab3b5f5b3c944986872edd7` 经实际 Chromium/TLS 1.3 三服务验收 10/10；未忽略证书错误，未使用 HTTP fixture。平台已提交/本地失败恢复保持原能力、消费次数 1 和原期限；在线续期保持原成员，切换后旧来源拒绝。独立库权限、负向绑定、员工客户 Cookie 与 SSO 隔离、原来源撤销均通过，浏览器及临时故障对象清理完成。见 `unified-auth/validation/phase-14.md`。

### Phase 15: 固定平台资源与内部派生能力边界
#### Status
COMPLETED
#### Goal
明确统一 Access Token 与平台内部业务会话派生凭据的验证职责。
#### Scope
平台资源入口、scope/audience、员工／客户来源证明、BFF 内部 token_use 与短期受众能力合同。
#### Out of Scope
不在平台恢复第二个 OAuth token endpoint，不替资源服务做全部业务数据判断。
#### Inputs
Phase 9、12、13；当前平台 session-token、context、userinfo／proof 与资源授权模块。
#### Execution Requirements
所有接受凭据的入口列明 issuer、audience、token_use、主体、scope、原版本及业务权限；拒绝 ID Token／机器伪装／跨资源混用。
#### Expected Outputs
资源验证器与内部能力合同、接口分类清单和拒绝矩阵、Phase 15 报告。
#### Acceptance Criteria
- [x] 外部 token 和内部能力各有准确验证器，不靠字符串前缀隐式混用。
- [x] 资源范围／成员／应用收缩能拒绝相应旧能力。
- [x] 无业务成员第三方只使用账号资料，不能访问平台管理资源。
- [x] 门店与工单可按明确合同独立接入而不依赖平台实现 crate。
#### Risks
签名正确被当作业务授权，派生 token 成为永续旁路。
#### Dependencies
Phase 9、12、13。
#### Notes
资源 API 中可共享稳定协议，但不能共享平台业务实现。

#### Completion Record
2026-10-28：固定运行源码/结构快照 `605ac9ef8aba39fbeed7037287380559bab9a0c075bbc61a49362f71fc1d87f3`；真实 LOGIN 角色矩阵 3/3、上下文 HTTP 1/1、身份资源 proof 1/1、平台资源及新增私有目录 HTTP 综合矩阵 1/1、客户注册表及续期回归 9/9、聚焦单元 9/9 均通过。格式、all-target check/strict Clippy 及独立安全审查通过。公共 opaque sub 不解析内部 UUID；已撤销内部摘要不回退外部验证。门店/工单的独立业务验证由 Phase 16/17 负责。详见 `unified-auth/validation/phase-15.md`。

### Phase 16: 适配门店 API
#### Status
COMPLETED
#### Goal
门店服务按统一资源合同验证调用来源和 STORE 业务范围。
#### Scope
门店受众／scope、平台上下文证明、人员与机器允许范围、拒绝／故障语义与现有业务读写回归。
#### Out of Scope
不改工单，不重建既有门店业务数据，不直读平台库。
#### Inputs
Phase 15 合同；门店独立 API／数据库、平台投影与当前客户端配置。
#### Execution Requirements
通过明确 HTTP 协议取得身份与组织依据；数据库与来源严格限定，失联不能跳过验证。
#### Expected Outputs
门店适配、STORE／身份／scope 及真实读写测试、Phase 16 报告。
#### Acceptance Criteria
- [x] 合法 STORE 身份可完成授权范围内的资料／日报请求。
- [x] 错资源、错组织、停用成员、机器冒充人和来源不可用被拒绝。
- [x] 业务数据边界与现有功能回归通过。
- [x] 仅使用专属测试数据并完成资源清理。
#### Risks
兼容旧 token 导致越权；测试误连已有业务库。
#### Dependencies
Phase 15。
#### Notes
与工单分阶段交付，避免一次运行跨两个独立业务项目。

#### Completion Record
2026-10-28：门店生产只使用 canonical combined 授权与私有 POST 目录，严格 USER principal、issuer、scope 和 STORE 证据；MACHINE 未批准业务范围明确拒绝。92 个普通测试、真实独立 PostgreSQL 5/5（20 次负矩阵、缺 scope 绑定、提交前撤销/故障回滚及锁等待期限）与 fmt/check/strict Clippy 通过。8 个临时新库均删除，剩余 0；无容器。实际全版本跨服务 TLS 仍由 Phase 25 验收。见 `unified-auth/validation/phase-16.md`。

### Phase 17: 适配工单 API
#### Status
COMPLETED
#### Goal
工单服务按统一资源合同执行上下文、动作与数据范围授权。
#### Scope
工单受众／scope、来源校验、查询／处理／导出等授权和业务回归。
#### Out of Scope
不改门店功能，不用 token scope 替代工单实际可见范围。
#### Inputs
Phase 15 合同；工单独立 API／数据库、现有平台 proof 与动作权限。
#### Execution Requirements
请求必须同时满足应用、客户端、同意、受众、动作与业务范围；客户端声明的用户／组织不是权威。
#### Expected Outputs
工单适配、读写／导出与拒绝矩阵、Phase 17 报告。
#### Acceptance Criteria
- [x] 合法员工来源在原范围内完成代表性业务流程。
- [x] 跨组织、scope 扩大、失效来源、错误 token_use／audience 均拒绝。
- [x] 机器授权只使用明确批准的数据范围。
- [x] 独立库、连接与临时测试资源无残留。
#### Risks
读取／导出路径校验不一致，或以当前成员代替原授权代际。
#### Dependencies
Phase 15。
#### Notes
门店阶段的通过结果不代替本阶段验收。

#### Completion Record
2026-10-28：工单 canonical-only typed USER/MACHINE、显式 issuer/scope 和 Basic/Form 私有目录完成；MACHINE 仅批准的 read/export，人员写入/SELF/目录/提醒均拒绝。普通 81/81、文档 1/1、显式 PostgreSQL/HTTP 58/58 与最新目录定向 14/14、fmt/check/strict Clippy 全通过。源复验不持业务连接或行锁，四前缀测试库/会话均为 0，无容器。外部 USER 提醒当前明确拒绝且不造成功意图，由 Phase 18 私有通知发布闭合；全版本 TLS 由 Phase 25 验收。见 `unified-auth/validation/phase-17.md`。

### Phase 18: 贯通宿主、SDK 与通知上下文变化
#### Status
COMPLETED
#### Goal
让一次业务身份切换或撤销可靠传播到标签页、微应用和通知连接。
#### Scope
宿主冻结、逻辑会话边界、微应用卸载／重挂载、SDK 上下文更新、通知重订阅／断开及客户隔离。
#### Out of Scope
不重写 qiankun 或引入新的微应用平台，不因账号 SSO 有效向客户发送员工通知。
#### Inputs
Phase 13、14、16、17 会话／业务证据；宿主、SDK、通知的当前契约。
#### Execution Requirements
以服务端会话／身份版本为依据；切换前冻结旧写入，提交后失效旧派生能力，失败按原来源规则恢复或拒绝。
#### Expected Outputs
贯通适配与多标签页／微应用／通知浏览器测试、Phase 18 报告。
#### Acceptance Criteria
- [x] 切换后无旧组织数据或写入覆盖，微应用和通知使用新来源。
- [x] 延迟响应和旧连接被丢弃或断开，无跨账号／员工客户泄漏。
- [x] 取消切换只恢复仍有效的原会话。
- [x] SDK 契约版本、消费者及必要发布说明一致。
#### Risks
客户端表面更新而旧 socket／业务能力继续生效。
#### Dependencies
Phase 13、14、16、17。
#### Notes
本阶段是同一上下文变化链路，报告分别列出所有实际参与仓库与资源。


#### Completion Record
2026-10-28：两类独立审查P2均修复并终审无剩余P1/P2。切换128bit关联、pending32/settled128、并行/逐版本/overflow及queued-renew统一guard通过；宿主476普通、浏览器12、build/407交付/固定SDK通过。私有publication不可变原recipientProofs使公开DELETE不能抹去原收件人复验，真实LOGIN/HTTP1/1（4.88s）+普通3+strictClippy通过；工单15数据库/HTTP、通知3进程/PG/SocketIO、SDK30等组件通过。47输入SHA manifest `1553bd1da4fb2ddc8958b1d74b6822611bc73d1c301b53b7dcf8b37eb47c1036` 已核实，专属资源0残留、Docker0，共享PG/cache保留，最终全版本TLS仍需25。见 `unified-auth/validation/phase-18.md` 及components JSON。

### Phase 19: 落地合作方审核与生产接入流程
#### Status
COMPLETED
#### Goal
使第三方从申请到测试／生产登记有可操作的管理流程。
#### Scope
合作方资料、域名／责任人验证、应用与 scope 申请、审核／退回／停用、测试生产隔离、授权分工及接入清单；实现受权限保护的账号邀请签发／撤销，限制为 24 小时、绑定收件人与用途、只存 token 摘要且成功消费一次。
#### Out of Scope
不擅自开放自助注册、动态客户端注册或未经确认的敏感资料。
#### Inputs
Phase 3、4、5、7、8；Q3／Q4 的审核责任和数据范围决策，Q5 的受控账号邀请政策。
#### Execution Requirements
页面使用已有统一登记和凭据操作，不再另建客户端系统。角色权限拆分查看／登记／审核／轮换／停用，合作方管理员只能访问明确应用；邀请签发必须经过平台管理员授权并记录收件人与用途，不能从公开入口创建。
#### Expected Outputs
审核与接入界面、权限／状态测试、账号邀请管理入口与 token 生命周期、合作方接入清单、Phase 19 报告。
#### Acceptance Criteria
- [x] 生产管理员和凭据／scope／生产审核等敏感写入强制校验 Phase 8 的 TOTP 与最近 5 分钟二次认证；缺失或过期拒绝，不能仅隐藏按钮。
- [x] 申请、退回、批准、生产登记及暂停有明确操作者与审计。
- [x] 未批准 scope、回调域、跨合作方资源和测试凭据调用生产被拒绝。
- [x] 轮换入口复用 Phase 4 且不会二次泄露 secret。
- [x] 只有获授权的平台管理员可签发／撤销邀请；收件人或用途变化会产生新邀请，token 摘要持久化，过期、撤销或消费后不能再次使用。
- [x] 未确认的运营责任或数据政策对应功能保持受控，不伪造审批完成。
#### Risks
业务租户管理员被误当合作方接入管理员。
#### Dependencies
Phase 3、4、5、7、8；Q3/Q4 是生产审核上线的硬前置。
#### Notes
可以在人工审核角色／流程获确认后交付，不要求建设自动开发者门户。

#### Completion Record
2026-10-28：实际 Chromium/TLS 1.3 验收 13/13，三名独立人员真实绑定 TOTP、密码与二次认证、新 BFF/原来源隔离、测试审核/身份准入、邀请签发/撤销、陈旧及恢复码拒绝、生产默认关闭及明确 fixture 责任下的批准/发布/暂停均通过。修复内部 guard 误拒合法客户端分页 query，并以真实 PG/HTTP 回归复现及验证。组合运行时/二进制/脚本来源与三份 RED 均留档；两个任务库/角色及三个服务、namespace、gateway/proxy、12 个私有文件和日志已清理，残留 0。实际 Q3/Q8 发布责任事实仍未提供，生产保持关闭。见 `unified-auth/validation/phase-19.md` 与 `phase-19-cleanup.json`。

### Phase 20: 实现登出 outbox 与 back-channel logout
#### Status
COMPLETED
#### Goal
按准确范围退出并可靠通知接入方，不把通知成功等同于 token 撤销本身。
#### Scope
当前应用／全局退出、同意撤销通知、事务 outbox、签名 logout token、投递／重试／告警和接收示例；通过已验证渠道投递 Phase 19 的账号邀请，投递内容不写入日志且链接仅一次有效。
#### Out of Scope
不承诺第三方本地会话即时全部退出，不依赖 iframe 作为唯一手段。
#### Inputs
Phase 7、9、10、13、14、19；登记的登出地址与应用会话映射、Phase 19 的邀请记录和 Q5 已验证渠道合同。
#### Execution Requirements
验证 aud/sid/sub/events/重放；接收地址固定且有 SSRF 网络约束。撤销事务不等待外部网络，投递有界并有幂等追踪。
#### Expected Outputs
登出服务／outbox／消费者、邀请投递与一次性消费服务、重试和范围隔离测试、Phase 20 报告。
#### Acceptance Criteria
- [x] 当前应用与全局退出影响不同且同意保留规则正确。
- [x] 错签名、错受众、重放、未登记地址及危险内网目的被拒绝。
- [x] 对方故障不延迟本地撤销，恢复后受控重试，永久失败可告警。
- [x] 不支持通知的合作方有明确短会话／在线检查降级合同。
- [x] 邀请仅通过已验证的收件渠道投递；失败受控重试，token 不进入日志或可复用的投递记录，成功接受后重复访问被拒绝。
#### Risks
outbox 丢失、无界重试、通知成为 SSRF，或范围过大误踢其他应用。
#### Dependencies
Phase 7、9、10、13、14、19。
#### Notes
账号停用与安全事件也必须引用相同来源失效合同。

#### Completion Record
2026-10-28：固定组合快照的真实三服务 TLS 1.3/Chromium 13/13；同一 harness 以 INFO 配置重跑13/13仅验证日志门禁，不重复计作26场景。7份日志3,014,692字节及6条实际邀请消息在内存比对无秘密，永久失败固定告警1条。双库/角色、服务、namespace、gateway、proxy、collector及93,475字节私有文件/日志已清理，残留0；既有443 PID5044不变，共享任务PG/cache保留。见 `unified-auth/validation/phase-20.md`、`phase-20-https-browser.json`、`phase-20-log-redaction.json`、`phase-20-cleanup.json`。

### Phase 21: 交付独立第三方 OIDC 接入示例
#### Status
COMPLETED
#### Goal
证明外部系统能够仅依靠公开标准协议接入。
#### Scope
独立 Web BFF 示例、标准发现／code／ID Token／UserInfo／refresh／登出、账号绑定说明和机器调用示例。
#### Out of Scope
不读取平台／身份数据库，不依赖内部 Rust 实现或强制 third-party sub 为 UUID。
#### Inputs
Phase 8–11、19、20 的协议、生产申请和登出合同；`saas-examples`。
#### Execution Requirements
至少一个通用标准 OIDC 客户端库完成互操作；示例仅使用外部可获得配置，秘密由实际部署注入，文档含错误与撤销处理。
#### Expected Outputs
可运行示例、第三方接入指南、互操作记录、Phase 21 报告。
#### Acceptance Criteria
- [x] 无业务成员账号完成纯账号登录，ID Token 与 UserInfo 身份一致。
- [x] state/nonce/issuer/PKCE 错误、撤销／离线期限／登出场景有实际证据。追加登出容量P2修复和新快照完整实际复测通过。
- [x] 第三方已有账号关联要求证明双方控制权，不按相同邮箱自动合并。
- [x] 示例无法越过受众／scope 或访问未批准业务资源。
#### Risks
示例隐式借助内网证明或本机特殊配置，失去外部可用性。
#### Dependencies
Phase 8、9、10、11、19、20。
#### Notes
通过标准客户端测试不等于已获 OpenID 官方认证。


#### Completion Record
2026-10-28：独立 `openid-client 6.8.8` BFF/机器CLI，严格TLS组件35/35、实际身份权威/平台资源/无成员账号的TLS1.3浏览器15/15通过。实际state/issuer/nonce/PKCE、UserInfo、refresh/replay/RFC7009、受众及准确登出范围均验证；第三方默认离线/MACHINE仍拒绝，正向为独立获批FIRST_PARTY fixture。双控制权关联指南、不自动邮箱合并、4份日志脱敏、37链接及运行摘要通过；专属库/6角色/服务/lease/私密材料全部清理，原服务不动。纯公开Node/锁依赖/tools保留给23/25，最终goal收尾清理。见 `unified-auth/validation/phase-21.md`、`phase-21-https-browser.json`、`phase-21-cleanup.json`。


#### Review Reopen Record
2026-10-28：首轮完成后追加独立审查复现SOURCE_CAPACITY：unknown sid tombstones达到容量上限时，closeSource在本地清token之前抛503；logout已pending但仍authenticated，旧UserInfo可继续。重新打开Phase21，先补HTTP RED，再使容量/关闭失败也关闭全部原本地source并阻止迟到callback/refresh复活，不扩容、不淘汰未过期事实。此前真实15/15及791bb5...快照保留为原版本证据，修后源码不得冒称已用它重跑。修复/独立复核与最小充分动态门禁完成前保持IN_PROGRESS；不接管Phase22当前环境。


#### Repair Acceptance Record
2026-10-28：容量P2经HTTP RED2失败复现；原source所有本地session先关闭并抬generation，满来源/replay账本保持有界overflow拒绝新挂载且不淘汰未过期事实，迟到callback/refresh不可复活。focused11/11、full46/46和独立复审无剩余P1/P2；最终RP源码SHA `72e685c26f4aae783c8b0f16e9c5e0ce692cd86fa0782f912ca5250a13d3fc0a` 的完整实际TLS1.3 15/15重新执行通过，新结果SHA `9d804a473d7ea26b6016e8b8b8bb22f8dfe61991b6706698d6996d050d5bda95`。旧15/旧源码保持历史，不混总数；本轮4日志无秘密、49链接有效，17,495,743字节专属库/6角色/私密材料/进程/leases已清理，TLS归还22，Docker0。见 `phase-21.md` 及 `phase-21-capacity-{components,runtime-attestation,https-browser,log-redaction,cleanup}.json`。

### Phase 22: 完成认证运行监控与有界故障处理
#### Status
COMPLETED
#### Goal
让统一认证入口在依赖故障或容量受压时可观察且受控。
#### Scope
账号／客户端／IP 限流、密码队列、数据库／私有调用超时、登录与内省指标、outbox 积压、告警及容量边界测试。
#### Out of Scope
不编造运营 SLA，不用拒绝记录增长或无限缓存维持表面可用性。
#### Inputs
Phase 8–11、20；当前 telemetry／限流／队列／健康探针；Q9 容量与运营目标。
#### Execution Requirements
明确单节点和全局限制；客户端网络、队列、连接与重试有上限；脱敏可验证；关键授权事务失败关闭。
#### Expected Outputs
指标／告警／运行说明、受压和故障测试、Phase 22 报告。
#### Acceptance Criteria
- [x] 认证／平台／数据库失联、队列满、超时及连接池饱和有确定语义。
- [x] 多实例下不以进程锁保证 code／refresh／撤销正确性。
- [x] 压测在明确限额和专属环境执行，报告实际容量与瓶颈。
- [x] 日志无秘密，异常路径与 outbox 永久失败能追踪和告警。
#### Risks
测试影响其他工作负载；故障时错误清空用户状态或放行缓存授权。
#### Dependencies
Phase 8、9、10、11、20；Q9 未确认时仅报告实测能力，不承诺服务等级。
#### Notes
涉及 Docker 的预算与完整清理必须遵守全局规则。


#### Completion Record
2026-10-28：固定identity源码03878de6...、binary7e3bda90...；lib87/54显式PGignored，binary subscriber1、independent-state PG6、观测PG2、Node26及strictClippy通过，独立审查WARN父target P2已修。实际HTTPS8/8、CODE18.65/REFRESH24.05/内省87.62有效RPS及p95/p99/CPU/RSS/pool实测通过；未达Q9目标和30min不外推承诺。故障保原cookie/flow，实际collector收到DB/queue FIRING+RESOLVED、TERMINAL FIRING+ACK共6事件，终态保留；固定条件管道不冒称PromQL/Alertmanager。容量与故障分fixture明确来源；最终库/6角色/进程/lease/24私密文件日志已清理，canary泄漏0，原服务不动、Docker0，共享PG/cache保留。见 `unified-auth/validation/phase-22.md`、runtime-attestation、self-check与cleanup JSON。

### Phase 23: 验证密钥生命周期与备份恢复
#### Status
COMPLETED
#### Goal
使统一认证可以在密钥轮换和灾难恢复后继续验证合法凭据并保留撤销事实。
#### Scope
签名公钥发布／激活／验证窗口／退役／紧急撤销，密钥与数据库备份、隔离恢复和失败恢复流程。
#### Out of Scope
不删除仍需验证的公钥，不在实际生产环境演练破坏性恢复。
#### Inputs
Phase 22 运行边界；当前密钥／恢复工具；目标 grant／同意／撤销记录；Q9 的 RTO/RPO 目标。
#### Execution Requirements
用独立 fixture 密钥与任务专属备份；恢复验证版本、同意及撤销代际，不能把已撤销 token 恢复成有效。报告密钥保管和操作权限。
#### Expected Outputs
轮换／恢复 runbook、实际演练与时间证据、Phase 23 报告。
#### Acceptance Criteria
- [x] 发布、激活、窗口保留及紧急撤销有完整校验。
- [x] 并存密钥窗口内合法 token 可验证，错误／已撤销密钥拒绝。
- [x] 恢复后旧撤销事实不丢失，已注销／失效来源不复活。
- [x] 实测恢复结果和目标差距明确，fixture 私钥与备份临时资源清理。
#### Risks
数据库快照与密钥不一致；恢复回滚授权代际。
#### Dependencies
Phase 22；正式恢复目标依赖 Q9。
#### Notes
本阶段验证 runbook，不是获准恢复正式数据库。


#### Execution Record
2026-10-28：Phase22全部依赖验收与清理完成。按已授权并行策略，密钥生命周期组件和canonical恢复工具分别实施，native实际HTTPS与恢复由单一环境owner协调；root独占任何task-owned HBA变更。依据 `phase-23-preparation.md`（含4.1精确BFF/墓碑/UNKNOWN约束及7.1窄SCRAM方案）；不得复用旧oidc/oauth inventory作为新体系恢复证明，不操作真实5432库或原443。

#### Completion Record
2026-10-29：apps-r3完整构建及6个binary摘要固定；r19实际完整密钥组通过（该轮整体因后续邀请恢复失败仍保留FAIL），r21显式recovery模式最终8/8通过，不冒称重复执行完整密钥组。修复append-only同意receipt误用FOR UPDATE、历史邀请验证材料丢失及恢复后context登记代际不一致；最终纯组件387/387、runtime LOGIN-role PG1/1、lib88/58显式PGignored、strictClippy通过。真实TLS1.3、双实例签发、原code拒绝后仍余5.039秒、11项旧/新service HTTP、严格恢复不变量和新双BFF/标准RP/MACHINE业务通过。实测RPO26.463秒、完整RTO98.942秒，符合本机fixture内部候选目标，不外推生产承诺。4库/8角色/78私有文件161,473bytes及进程/lease已清，HBA原字节/stat恢复且journal删除；Docker0、df四类0B。共享任务PG/公开快照/编译缓存保留给后续阶段。见 `unified-auth/validation/phase-23.md` 及其分组原始证据。

### Phase 24: 退役旧客户端与旧协议运行路径
#### Status
COMPLETED
#### Goal
消除中间阶段保留的双轨，形成只用统一体系的新安装版本。
#### Scope
删除旧登记／签发代码、旧表及旧配置／路由／文档引用；收敛 canonical `0001` 与初始化清单；保留明确非 OAuth 的内部会话边界。
#### Out of Scope
不搬迁旧库、不发布线上、不把旧实现包成自动 fallback。
#### Inputs
Phase 1 调用清单、Phase 2 暂存结构清单及所有消费者／运营／示例阶段证据。
#### Execution Requirements
逐项证明调用者已替换再删除；检查源码、网关、配置、测试、SDK 和文档引用。统一服务不可达时拒绝，不回到平台旧签发。
#### Expected Outputs
退役差异清单、单基线／单入口静态与空库检查、Phase 24 报告。
#### Acceptance Criteria
- [x] 无独立运行的业务／登录两套客户端和 OAuth 签发链路。
- [x] 当前空库仅按目标基线建立，owner／readiness／初始化流程一致。
- [x] 保留的 BFF／内部能力均有明确用途，不是改名的第二授权服务器。
- [x] 原切换清单每项有去向或删除证据，无旧 token／checksum 兼容旁路。
#### Risks
误删仍使用的应用登记、业务授权或内部会话；残留路由继续签发。
#### Dependencies
Phase 5、13、14、15、16、17、18、19、20、21、23。
#### Notes
保留旧版本恢复工件不等于目标版本可同时运行旧入口。

#### Completion Record
2026-10-29：按R01–R16完成旧登记/签发/配置/fixture退役及canonical保留边界。最终新集群空库103、identity141库测试+main1、平台432唯一用例分层覆盖（382输入等价+50当前重验，191原ignored实际执行）、strictClippy/fmt和实际Nginx14顶层门禁通过；旧FAIL不改写。修复成员capability FK与应用策略失效遗漏，R15旧restore在I/O前关闭。4专属private目录/76文件309,448,977逻辑字节、测试DB/roles及进程清理PASS，Docker0；WSL重启/tmpfs丢失与未删旧ignored工件独立如实记录，共享PG/cache供下一阶段复用。详见 `unified-auth/validation/phase-24.md` 及各分层JSON。

### Phase 25: 完成发布候选全新安装与验收矩阵
#### Status
IN_PROGRESS
#### Goal
对已收敛的目标版本形成可审阅、可切换的完整发布候选。
#### Scope
任务专属环境全新安装，重跑方案第 18 节矩阵；端到端自有／第三方／机器流程；配置、备份、回退、上线及清理清单。
#### Out of Scope
不修改正式 DNS／网关、重置已有库或对外宣告上线。
#### Inputs
Phase 24 当前源码；全部阶段报告；第 7 节问题处置与 Q9/Q10 运营发布资料。
#### Execution Requirements
逐条链接当前测试和实际版本，明确单元／fixture／数据库／HTTPS／浏览器／外部客户端证据层级；没有执行的项标记未验证并保留发布门禁。
#### Expected Outputs
验收矩阵结果、部署参数清单（不含秘密）、切换／回退候选 runbook、Phase 25 报告。
#### Acceptance Criteria
- [ ] 方案验收矩阵所有条目有可追溯结果，无以旧报告顶替的新版本空缺。
- [ ] 从空库完成全部参与服务和外部示例；失败与恢复路径验证。
- [ ] 风险、容量、密钥、告警、运维责任及外部接入说明齐备。
- [ ] 真实部署目标、需授权操作和回退触发条件已列明；资源已按归属完整清理。
#### Risks
发布候选被误当正式上线；外部条件不明时写出虚假全部通过。
#### Dependencies
Phase 24；未确认运营事项须在报告列为 Phase 26 的硬前置。
#### Notes
若门禁未满足，不得以“基本可用”把整个项目标为完成。

#### Execution Record
2026-10-29：Phase24四项依赖验收与专属清理完成。按全部阶段授权继续当前完整版本候选：固定八仓源码、配置和二进制，任务专属新安装并逐项重跑十八类矩阵；源码/组件、实际PG、跨服务HTTPS、浏览器和独立标准客户端分层记录。复用已绑定的本次原生PG与cache，单一owner管理实际环境，Docker预算0；不把任务fixture/旧阶段结果冒充正式目标或当前全版本通过。

### Phase 26: 执行获授权的正式切换与外部开放
#### Status
NOT_STARTED
#### Goal
在明确授权的真实环境完成单一认证体系切换及运营交接。
#### Scope
核对目标、准备受保护凭据、准确 issuer／CA／回调／资源配置，执行批准的新安装或目标重置、角色交接、网关切换、外部冒烟和观察。
#### Out of Scope
不扩大清库范围，不自动操作未纳入清单的远端或其他项目，不让新旧系统争用同一 issuer。
#### Inputs
Phase 25 候选与 runbook；用户针对实际部署／数据操作的授权；Q3/Q5/Q6/Q8/Q9/Q10 最终运营与安全决策。
#### Execution Requirements
执行前核对本机／远端、库名、归属与受影响服务。切换失败按批准的整体恢复方案处理，不启动协议 fallback。敏感参数经保护通道提供。
#### Expected Outputs
真实上线／回退记录、外部客户端检查、监控观察与责任交接、最终配置引用及 Phase 26 报告。
#### Acceptance Criteria
- [ ] 实际环境与授权范围完全一致，单 issuer 对应唯一签发权威。
- [ ] 自有应用、第三方标准客户端及资源 API 的上线冒烟通过。
- [ ] 运行指标、撤销／登出时效、支持人员及应急流程可用。
- [ ] 备份／旧环境处置和临时资源清理按批准清单完成；若回退，记录失败并保持阶段未完成。
#### Risks
遗漏客户端配置导致登录中断；回退恢复错误数据／密钥；误称所有第三方本地会话即时退出。
#### Dependencies
Phase 25；真实环境、运营门禁及相应任务授权必需，缺失时不能执行发布相关操作。
#### Notes
Phase 26 完成且全局成功条件满足后，才可将整个统一项目认定完成。

---

## 5. Current Execution Pointer

Current Phase: Phase 25

Current Objective:
冻结当前完整发布候选，按当前canonical 0001完成所有参与服务的任务专属全新安装，逐条重跑总体方案第18节十八类验收矩阵和标准外部客户端；形成配置/备份/回退/切换候选清单、精确真实目标及待授权操作。Phase25尚未计完成；Phase24组件与分层证据按输入比对引用，不替代当前全版本互通与浏览器结果。

Parallel Execution:
用户于2026-10-28明确要求“能并行的任务就并行执行”。Phase1–24已完成，Phase25依赖已满足。后续可按构建/前端/矩阵映射/部署清单等独立文件组并行；共享DDL、冻结ENV/bin/manifest及任何实际环境/HBA写窗口仍由单一owner协调。Phase25实际环境、私密配置和编译/前端输入须一致；Phase26真实切换门禁继续保留，不以组件/既有报告代替新版本验收。

Current Restrictions:

- 用户已明确授权按依赖顺序执行全部阶段；逐阶段核对前置报告和验收证据。
- Phase1–24已完成；Phase25为当前执行阶段。Phase24退役源、最终schema/owner/readiness和当前组件已验收，不能替代Phase25完整新版本矩阵。Phase 9 opaque access token 与 Phase 10 refresh family 绑定精确 resource/scope、用户 grant、consent generation、group version、原始 SSO session 与客户端／应用版本；Phase 11 MACHINE grant 绑定确切凭据版本、应用审批 scope、客户端资源权限及 audience；Phase 12 一次性业务上下文结果与原来源、成员、RP/应用策略版本绑定；Phase 13 员工 BFF 及 Phase 14 客户门户的实际 HTTPS 会话、来源撤销和相互隔离已验收；Phase 15 资源与内部能力合同为各独立消费者唯一目标入口，Phase 16/17 独立业务组件及真实数据库边界均已验收；Phase23只证明其冻结应用与分组密钥/恢复证据，最终当前版本互通仍需 Phase25重跑。
- 不接管/删除既有库，不改动正式在线服务、网关或密钥；临时验证资源须按 AGENTS 定向清理。
- Q1–Q10 按已采纳决策执行；Q3／Q8／Q9／Q10 仅真实发布事实待核实，按问题表设置门禁。
- 用户已确认 Phase 26 目标为本机，域名为 `auth.saas.test`、`console.saas.test`；实际目标库名与归属、运行责任人及具体切换清单仍待 Phase 25 核实，不把隔离验证库或临时网关当作正式部署。
- 用户已明确授权按依赖顺序推进全部阶段；后续阶段仍须逐项满足依赖与真实环境门禁。

---

## 6. Decision Log

| Date | Decision | Reason | Impact |
|---|---|---|---|
| 2026-10-29 | Phase24按当前输入和分层执行完成，命令/签名对象中性化且KEY_ADMIN用途独立 | 单一canonical权威、旧恢复入口关闭，103空库/142 identity/432平台唯一覆盖/Nginx与专属清理通过；保留失败历史及输入等价证明 | Phase24 COMPLETED；指针25 IN_PROGRESS；Q3/Q8/Q9/Q10及26真实授权门禁不变 |
| 2026-10-29 | Phase23按同应用构建的密钥与恢复分组证据完成 | r19密钥组PASS且保留该次整体FAIL；r21恢复8/8、组件387、旧载体/11service/新业务及清理PASS | Phase23 COMPLETED；指针24 NOT_STARTED；Q9生产SLO/人员及25当前全版本、26实际切换门禁不变 |
| 2026-10-28 | Phase22技术门禁完成，启动23密钥与恢复并行 | 当前实测容量/故障8/告警6/日志0泄漏与资源0残留，独立终审通过 | Phase22 COMPLETED；Phase23 IN_PROGRESS；Q9未达到目标继续列真实发布门禁 |
| 2026-10-28 | Phase21容量P2修复并完成新快照实际复测 | 46组件/独立复审/新TLS15及4日志脱敏通过，新库/6角色/进程/私密文件已清理 | Phase21恢复COMPLETED；主指针22实际运行门禁；旧结果独立保留，不混新旧证据 |
| 2026-10-28 | Phase18两类P2修复和独立终审完成；Phase21追加容量P2重开 | 宿主476/浏览器12/后端PG及47文件摘要全通过；标准RP容量满登出可残留旧source已复现 | Phase18 COMPLETED；Phase21 IN_PROGRESS主指针，与22并行；历史15TLS不冒充修后通过 |
| 2026-10-28 | 标准第三方OIDC示例实际互操作验收完成 | 组件35/35与实际TLS15/15分层通过，日志无秘密、专属环境清理0残留 | Phase21 COMPLETED；Phase18审查修复最终门禁及Phase22继续并行；默认第三方权限不扩张 |
| 2026-10-28 | Phase20真实HTTPS、INFO告警/脱敏及清理验收；启动21/22并行 | 原13场景及同套日志门禁通过，告警1/泄露0/资源残留0 | Phase20 COMPLETED；Phase21/22 IN_PROGRESS；Phase18 P2修复仍待当前回归 |
| 2026-10-28 | 工单验收完成并启动宿主/SDK/通知链 | 工单81普通、1文档、58实际数据库/HTTP及新目录14/14通过，库/会话残留0 | Phase 17 COMPLETED；Phase 18/20并行，外部USER提醒在18按私有服务合同闭合；不把组件证据冒充最终TLS互通 |
| 2026-10-28 | 门店与合作方流程验收完成，并行启动 canonical 登出/可靠投递 | Phase 16 普通92/真实PG5/5及清理通过；Phase 19 实际TOTP/审核/受控生产13/13及隔离资源清理通过 | Phase 16/19 COMPLETED；Phase 17、20并行；Phase 18等待17完成；真实运营事实不由fixture替代 |
| 2026-10-28 | Phase 15 资源与私有目录验收完成，并行启动门店/工单 | 真实角色、canonical proof、私有服务认证及原来源/权限收缩矩阵通过；独立审查无 P1/P2 | Phase 15 COMPLETED；Phase 16/17 按稳定合同独立适配，Phase 19 继续固定快照真实 HTTPS；不以组件 fixture 代替 Phase 25 全版本验收 |
| 2026-10-28 | Phase 14 实际 HTTPS 完成；记录本机发布目标 | 固定快照经三服务实际浏览器 10/10，原能力恢复/续期/切换/员工客户隔离均通过；用户确认 auth.saas.test、console.saas.test | Phase 14 COMPLETED；Phase 15、19 继续并行；实际目标库/归属与具体切换清单保留 Phase 25/26 门禁 |
| 2026-10-28 | Phase 13 实际 HTTPS 完成并启动 Phase 15 | 当前固定源码/新库/真实 LOGIN角色的 TLS 1.3 浏览器 8/8，code/refresh/来源与事务回归通过 | Phase 13 COMPLETED；Phase 15 与 Phase 14、19 并行，后续仍需当前版本独立验收 |
| 2026-10-28 | 按已完成依赖并行推进 Phase 13、14、19 | 用户明确要求能并行即并行；Phase 14、19 不依赖 Phase 13，前置已复核 | 分离门户、员工 BFF、合作方管理及身份/数据库文件所有权；共享本次隔离工具与资源，逐阶段独立验收，不越过未完成依赖 |
| 2026-10-27 | 目标为底层客户端与标准认证协议统一，不仅合并页面 | 用户明确统一登录也是 OAuth 客户端，自有／第三方最终接入 SSO | Phase 2–24 必须收敛到唯一权威 |
| 2026-10-27 | 应用归属与客户端类型／主体／上下文独立 | 第三方可能需要纯登录或业务 API，自有应用也可能是机器 | DTO 和表单不得仅按自有／第三方推断权限 |
| 2026-10-27 | 纯账号登录允许无业务成员 | 对外系统可只使用账号身份 | ACCOUNT_ONLY 不进入租户组织选择 |
| 2026-10-27 | 持久授权复用；单候选减少交互；撤销记录可删 | 用户已明确提出的产品行为 | Phase 7／12／13／14 保留这些行为与服务端校验 |
| 2026-10-27 | 采用全新 canonical 基线，不建设历史兼容链 | 当前工作区 AGENTS 和总体方案约束 | 新安装验证；任何已有数据删除需新任务明确授权 |
| 2026-10-27 | pairwise sub、同意组、opaque token、在线／离线分支作为方案设计基线 | 来自总体方案的建议架构，尚非已实现能力 | Phase 1 固定合同；开放政策按已采纳决策执行，真实发布事实按问题表核实 |
| 2026-10-27 | 拆为 26 个独立交付阶段 | 八仓库及多个协议／消费者里程碑不能合并成单次“完成集成” | 每阶段独立验收，不自动跨阶段执行 |
| 2026-10-27 | 用户授权按建议采纳开放问题，政策依决策文档执行 | 不再重复询问已决定偏好；真实环境事实不能编造 | Q1／2／4／5／6／7 决策完成，其余保留发布事实门禁 |
| 2026-10-27 | 初次只创建台账 | 用户显式调用 task-ledger-creator | 当前执行指针 Phase 1，未开始实施 |
| 2026-10-27 | Phase 1 冻结合同与调用清单 | 用户明确要求执行全部阶段；源码盘点确认两套表／入口及消费者边界 | Phase 1 COMPLETED；执行指针移至 Phase 2；Phase 26 仍受 Q3/Q8/Q9/Q10 与具体授权门禁约束 |
| 2026-10-28 | 启动 Phase 2 空库模型与角色隔离 | Phase 1 验收通过；本阶段依赖合同及调用清单 | 仅建立目标结构和隔离验证；不触碰既有业务库 |
| 2026-10-28 | Phase 2 空库模型与角色隔离完成 | 空库迁移、约束、双 LOGIN-role ACL 与 verifier 经 PostgreSQL 实测通过 | Phase 2 COMPLETED；新 `auth_*` 结构加入 canonical `0001`；旧运行结构登记 Phase 24 退役；指针移至 Phase 3 |
| 2026-10-28 | 启动 Phase 3 应用／合作方认证投影协议 | Phase 2 通过并交付模型与 ACL | 仅实现私有最小投影与幂等收据，不实现合作方运营 UI 或公开客户端注册 |
| 2026-10-28 | Phase 3 应用／合作方认证投影协议完成 | 身份 PostgreSQL 路由回归与平台远程管理定向测试通过，含版本冲突、停用、错误服务用途和原 operation 恢复 | Phase 3 COMPLETED；第三方保持待审核；指针移至 Phase 4 |
| 2026-10-28 | 启动 Phase 4 统一客户端管理与凭据操作 API | Phase 3 完成并有当前投影／operation 恢复证据 | 保留旧 OIDC 管理 facade 作过渡；新增管理 API 直接使用 `auth_*` 权威，不返回持久 secret |
| 2026-10-28 | Phase 4 统一客户端管理与凭据操作 API 完成 | 身份真实 PostgreSQL 生命周期测试、平台固定目标测试与当前 canonical 0001/ACL 回归通过 | Phase 4 COMPLETED；第三方生产审核仍受 Q3/Phase 19 限制；指针移至 Phase 5 |
| 2026-10-28 | 启动 Phase 5 统一客户端管理页面 | Phase 4 API 与 DTO 已通过当前回归，前端仅消费统一 API | 一张列表按应用/合作方/环境/类型筛选，持久状态不保存任何 client secret |
| 2026-10-28 | Phase 5 统一客户端管理页面完成 | 六个 Chromium 浏览器场景、431 个 Node 测试、全量 typecheck 与生产构建通过 | 统一注册簿替代业务／OIDC 双列表；真实第三方审核仍由 Q3／Phase 19 门禁约束；指针移至 Phase 6 |
| 2026-10-28 | 启动 Phase 6 pairwise subject 与资料投影 | Phase 2 schema 与 Phase 1 subject／资料合同已完成；Q2／Q4 已采纳 | 只接受可信账号证明与批准的 sector／scope；不提供跨应用自动关联 |
| 2026-10-28 | Phase 6 pairwise subject 与资料投影完成 | PostgreSQL 回归覆盖 app/environment 隔离、并发首次映射、账户退休、最小声明与签名密钥轮换稳定性 | 账号 tombstone 同事务封存 subject links；旧 HTTP OIDC 路径与 BFF 暂不切换；指针移至 Phase 7 |
| 2026-10-28 | 启动 Phase 7 同意组与用户授权管理 | Phase 4 客户端、Phase 6 subject/profile 与 Q1 显式同应用共享政策已通过 | 按同意组与资源 scope 管理本人授权；真实协议与第三方发布门禁按后续阶段处理 |
| 2026-10-28 | 组成员变化推进同意组版本，旧用户同意需再次确认 | 防止管理员把新客户端加入旧组后自动继承既有用户授权，落实 Q1 显式共享与“管理员扩权不能替代用户同意” | 新客户端必须经显式组成员登记；Phase 8 sufficient-consent 检查必须比较用户确认的 group version |
| 2026-10-28 | Phase 7 同意组与用户授权管理完成 | PostgreSQL 路由回归、撤销并发/软删除/重授权、canonical 0001/角色边界、平台管理代理、431 个前端测试和 7 个 Chromium 场景通过 | consent 记录确认的 group version；成员变化要求用户再次确认；撤销事件仅本地写入 Phase 20 outbox；指针移至 Phase 8 |
| 2026-10-28 | 将 Q5 邀请签发与投递分配至 Phase 19/20 | 签发需要已确认的平台管理角色，投递需要已验证渠道与有界通知 outbox；Phase 8 保持禁止公开自助注册 | Q5 政策不变；Phase 19 验收 24 小时、收件人/用途绑定和一次性摘要 token，Phase 20 验收验证渠道投递与日志脱敏 |
| 2026-10-28 | Phase 8 统一人员授权码、OIDC 签发与 TOTP 完成 | PostgreSQL 协议与 MFA 回归 2/2、canonical schema/ACL 回归 1/1、fmt/check/clippy/JS 语法检查通过 | 人员 OAuth 只声明实现能力；暂存 `/oidc/*` 至 Phase 24；执行指针移至 Phase 9 |
| 2026-10-28 | Phase 9 资源限定 Access Token、内省与撤销完成 | PostgreSQL HTTP 回归覆盖资源 scope 上限、audience 隔离、pairwise 内省、ID Token 拒绝、逐 token 撤销及失效 session；身份测试 54 通过 | Discovery 声明 introspection/revoke；refresh 与 MACHINE token issuance 继续由 Phase 10/11 实施；执行指针移至 Phase 10 |
| 2026-10-28 | 同意只读校验使用账号共享锁，显式批准继续排他锁 | 并发 code 兑换重现账号行锁升级导致的 `40P01`；读校验无需写锁，保持共享锁可避免死锁并保留版本 fence | Phase 8/9 code redemption、UserInfo 和内省共享当前账号/同意代际来源；授权批准仍串行写入 |
| 2026-10-28 | Phase 10 在线与离线 refresh 生命周期完成 | 临时 PostgreSQL 18.6 回归验证 family 轮换/重放、并发、scope 收缩、空闲与绝对期限、在线来源失效、离线自然过期与显式撤销；P8/P9 协议回归及身份测试通过 | 注册 `refresh_token`；`offline_access` 仅由明确配置和用户同意启用；`/oauth/revoke` 可撤销所属 refresh family；执行指针移至 Phase 11 |
| 2026-10-28 | Phase 11 MACHINE client_credentials 完成 | PostgreSQL HTTP 回归验证机密服务客户端签发、应用审批与客户端资源权限双重 scope 上限、跨资源拒绝、人员身份隔离、凭据轮换版本和紧急撤销；canonical `0001` 回归验证 MACHINE grant 凭据版本约束 | Discovery 声明 `client_credentials`；MACHINE grant 绑定 `credential_version` 与精确 audience/scope；执行指针移至 Phase 12 |
| 2026-10-28 | Phase 12 统一业务上下文交互完成并启动 Phase 13 | canonical OAuth 上下文、账号/业务分流、来源代际栅栏、一次性结果与加密恢复通过 PostgreSQL、BFF 与浏览器回归；临时数据库集群定向清理 | Phase 12 COMPLETED；员工与客户实际 HTTPS 会话建立继续分别由 Phase 13/14 验收；当前执行指针移至 Phase 13 |

---

## 7. Open Questions

政策以[开放问题决策](unified-oauth-oidc-sso-policy-decisions.md)为准，用户已授权按建议采纳。COMPLETED 仅表示决策已确定，不表示功能已实现；IN_PROGRESS 表示政策已定，但还有实际发布事实待核实，不再重复征询产品偏好。

| ID | Question | Impact | Owner | Status |
|---|---|---|---|---|
| Q1 | 同应用、同环境和责任主体内显式组共享，跨用途／公司隔离 | Phase 4／5／7 实现与测试共享、扩权及撤销 | 产品／实施负责人 | COMPLETED |
| Q2 | 按应用和环境隔离 pairwise sub，首版无跨应用关联 | Phase 6 拒绝未批准 sector 和自动账号合并 | 安全／实施负责人 | COMPLETED |
| Q3 | 人工审核、申请人不可自批、受控启用；真实审核与发布账号待绑定 | Phase 19／26 绑定责任人，缺失不能生产启用 | 产品／运营／安全负责人 | IN_PROGRESS |
| Q4 | 第三方默认 openid/profile；email／phone／业务资源不开放 | Phase 6／9／19／21 拒绝未批准资料与 scope | 产品／数据负责人 | COMPLETED |
| Q5 | 受控创建／邀请，无公开注册；恢复需可信核验与撤销旧会话 | Phase 1／8 固定账号与恢复合同；Phase 19 实现邀请签发授权，Phase 20 实现已验证渠道投递，Phase 21 对外说明 | 产品／实施负责人 | COMPLETED |
| Q6 | 生产管理员 TOTP，敏感操作最近 5 分钟二次认证 | Phase 8／19 实现及强制；Phase 25／26 验收门禁 | 安全／实施负责人 | COMPLETED |
| Q7 | 采用决策文档具体期限，在线来源上限与离线 7 天分开 | Phase 10／22／25 验证；不冒充当前配置 | 产品／安全负责人 | COMPLETED |
| Q8 | 180 天安全审计、30 天日志／备份和注销清理默认值；实际法域与合同待核实 | Phase 7／19／23 实现；Phase 26 核实运营与法律适用事实 | 运营／合规负责人 | IN_PROGRESS |
| Q9 | 受邀试点及容量、99.9% 内部目标、RPO 15 分钟／RTO 4 小时；实测及值班人员待落实 | Phase 22／23／25 验证，未验证不得对外承诺 | 运维／安全负责人 | IN_PROGRESS |
| Q10 | 默认隔离新环境新库；用户已确认本机及 auth.saas.test、console.saas.test；目标库名、归属、秘密及具体切换授权待核对 | Phase 25 形成具体清单；Phase 26 获针对性授权后执行 | 环境负责人／User | IN_PROGRESS |

未核实发布事实不阻止无关盘点及实现；不以建议政策替代真实环境、容量报告、合规核实或具体操作授权。

---

## 8. Change Log

| Version | Date | Change |
|---|---|---|
| v2.5 | 2026-10-29 | Phase24退役与全部四项验收完成：103最终空库、identity141+main1、平台432唯一分层覆盖、Nginx14顶层实际验证及专属清理；保留WSL重启/旧FAIL与原生浏览器子测边界，指针25 |
| v2.4 | 2026-10-29 | Phase23完整密钥组与最终恢复8/8分层验收，RPO26.463秒/RTO98.942秒、秘密0泄漏/私密资源0残留；四项验收完成，指针移至Phase24 |
| v2.3 | 2026-10-28 | Phase22限流/监控/故障/容量/实际告警与清理全部通过；启动Phase23密钥与canonical恢复组件并行 |
| v2.2 | 2026-10-28 | Phase21容量P2修复、46组件和最终新快照TLS15/15及清理完成；主执行指针22 |
| v2.1 | 2026-10-28 | Phase18修复后完整验收/终审完成；追加审查发现Phase21登出SOURCE_CAPACITY P2并重开，主指针21，与22并行 |
| v2.0 | 2026-10-28 | Phase21标准库独立示例、实际TLS15/15与专属资源清理完成；Phase18/22继续并行 |
| v1.9 | 2026-10-28 | Phase20实际TLS、INFO日志门禁和定向清理完成；启动21/22并行；Phase18独立审查P2保持未完成 |
| v1.8 | 2026-10-28 | Phase 17 工单全部组件、真实数据库/协议58组与最新目录14组验收及清理完成；启动 Phase 18，与 Phase 20 并行 |
| v1.7 | 2026-10-28 | Phase 16 门店组件/真实数据库验收及 Phase 19 真实 HTTPS 13/13 与完整隔离资源清理完成；继续 Phase 17，并行启动 Phase 20 |
| v1.6 | 2026-10-28 | Phase 15 内部/外部资源验证器、真实角色及私有目录矩阵完成；并行启动 Phase 16 门店与 Phase 17 工单，继续 Phase 19 真实 HTTPS |
| v1.5 | 2026-10-28 | Phase 14 三服务真实 TLS 1.3 浏览器 10/10 验收完成；原提交恢复、续期、切换与员工客户隔离均通过；记录用户确认本机及 auth/console 域名；继续 Phase 15、19 |
| v1.4 | 2026-10-28 | Phase 13 真实 TLS 1.3 8/8 验收、源锁 ACL 与新账号代际修复完成；主指针移至 Phase 15，继续并行 Phase 14、19 |
| v1.3 | 2026-10-28 | 记录已完成阶段本轮审查与安全修复；按用户要求并行推进依赖已满足的 Phase 14、19；Phase 13/14 真实 HTTPS 及 Phase 19 各项验收尚未完成 |
| v1.2 | 2026-10-28 | 完成 Phase 12 统一业务上下文交互、来源绑定与一次性恢复回归；指针移至 Phase 13 |
| v1.1 | 2026-10-28 | 完成 Phase 11 MACHINE `client_credentials`、凭据版本、scope/audience 隔离与紧急撤销回归；指针移至 Phase 12 |
| v1.0 | 2026-10-28 | 完成 Phase 10 在线/离线 refresh 轮换、重放、期限、撤销与 PostgreSQL 回归；指针移至 Phase 11 |
| v0.9 | 2026-10-28 | 完成 Phase 9 资源 scope token、认证内省与逐 token 撤销；修复 consent read-lock deadlock；指针移至 Phase 10 |
| v0.8 | 2026-10-28 | 完成 Phase 8 人员 OAuth/OIDC、TOTP、恢复及 PostgreSQL 回归；将 Q5 邀请签发/投递分配至 Phase 19/20；指针移至 Phase 9 |
| v0.7 | 2026-10-28 | 完成 Phase 7 同意组、组成员版本 fence、用户授权管理 API/页面和回归；执行指针移至 Phase 8 |
| v0.6 | 2026-10-28 | 完成 Phase 6 pairwise subject 与资料投影及账户注销封存；执行指针移至 Phase 7 |
| v0.5 | 2026-10-28 | 完成 Phase 5 统一客户端管理页面与浏览器验收；执行指针移至 Phase 6 |
| v0.2 | 2026-10-27 | 按用户授权采纳 Q1–Q10 建议，新增政策决策，更新 MFA／账号恢复／期限与发布门禁；未执行实施阶段 |
| v0.4 | 2026-10-28 | 完成 Phase 4 统一客户端 API 与凭据生命周期；将执行指针移至 Phase 5 |
| v0.3 | 2026-10-28 | 完成 Phase 3 投影协议并记录当前代码／数据库验证证据；将指针移至 Phase 4 |
| v0.1 | 2026-10-27 | 根据总体方案创建任务台账，拆分 26 阶段，记录全局约束、可量化验收、依赖、当前执行指针、决策与 10 个待确认事项；未执行任何阶段 |

创建说明：沿用当前全新安装、SSO-only 和独立仓库边界；开放政策按[已采纳决策](unified-oauth-oidc-sso-policy-decisions.md)执行；待核实发布事实见 Q3／Q8／Q9／Q10。Phase1–24已完成；当前主执行指针为Phase25（IN_PROGRESS）；Phase26仍需要真实目标核对、对应决策事实和专项操作授权。
