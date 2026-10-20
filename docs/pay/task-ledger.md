# 支付应用接入与 Rust 重构任务台账

版本：v0.1。创建日期：2026-10-19（Asia/Shanghai）。项目实施状态：NOT_STARTED。当前执行指针：Phase 1，NOT_STARTED。

本台账按 task-ledger-creator 建立，是支付项目后续分阶段执行的状态、约束、决策、依赖和下一步来源。创建台账不代表授权执行任何实现、数据库迁移、渠道请求或生产部署。所有支付执行阶段均未开始。

本台账以[完整方案](README.md)为设计输入，并纳入后续讨论的“同租户根组织创建支付账户，显式授权当前组织及整棵下级子树使用”。后续明确用户指令优先；本台账新增或细化的执行边界优先于方案里较粗的 P0—P10 描述。不能仅靠聊天记录恢复关键要求。

本台账独立于[全仓架构优化台账](../architecture-optimization-ledger.md)，不复用其 Phase 编号，不修改其状态、历史证据或执行指针。

## 1. Project Overview（项目概况）

### Project Name

支付业务应用 payments：平台用户统一、租户组织隔离、根组织账户共享、UnifyPay Rust 重构与 QuickPay 接入。

### Project Goal

以一个支付应用承载独立管理微应用、Rust 支付 API/工作进程、公开 WAP 收银台和支付宝原生小程序。平台统一管理用户与组织权限；支付域保存商户、账户、交易、退款、事件和业务授权。先完成普通支付一期，再逐项接管旧系统实际使用的扩展资金能力与未结清义务。

### Background

- 当前平台已有用户、租户、组织、角色、两级应用/菜单授权、独立 data grant、应用 token、微应用 SDK 和独立 Rust 业务案例。合并授权与显式 legacy 均已存在。
- UnifyPay 有真实普通支付、退款、严格关单、重复收款裁决、渠道通知、余额/转账/付款/免押等实现，但没有租户/组织隔离，旧管理账号不能直接转为平台管理员。
- QuickPay 只有 WAP、支付宝小程序和发布工具，所需五类 /channel/open/* 服务在给定 UnifyPay 中缺失；订单权威、链接生成和业务来源必须核实。
- 根组织账户共享目前只有方案讨论，没有已实现的支付授权模型。现有 ORG_SUBTREE 向下限定业务范围，不自动赋予子组织读取或使用祖先账户的能力。
- 当前 organizationScopeIds 包含当前组织及其后代；organization.path 是展示名称路径，不能作身份关系证明。组织 tree 使用管理 session；组织更新目前不支持 parent_id 移动。
- 本次只创建本文件；未创建业务工程、运行数据库、调用渠道或执行任何 Phase。源码/生产事实仍需在 Phase 1 重新核对，不继承其他阶段测试通过结论。

### Success Criteria

**一期普通支付交付完成**：Phase 1—45 在明确冻结的范围内实际完成，具备同源证据；平台管理用户、账户子树授权、机器准入、所选普通支付场景、严格查关单、回调/退款、可靠任务、客户端、交付及存量责任全部闭环。根账户可被获准下级使用，子组织仍不能取得祖先管理详情/秘密或兄弟组织交易。

**旧支付域全量 Rust 替代完成**：一期完成后，E01—E09 中实际使用的能力已逐项拆成可执行阶段、迁移并验收；不存在无人接管的余额、冻结、转账、付款、免押、退款、通知或查补义务；旧引擎退役有独立批准与核对证据。Phase 45 完成不等于该全量目标已完成。

本版按 QuickPay 当前真实使用的四种候选场景规划：WECHAT_JSAPI、WECHAT_H5、ALIPAY_WAP、ALIPAY_LITE。当前主线依赖按四者均需完成编排；若业务明确缩小一期范围，先修改范围、阶段依赖和验收清单，未纳入阶段保留 NOT_STARTED，不用 SKIPPED 或虚假 COMPLETED 消除依赖。

### Inputs and Evidence（输入与依据）

| ID | 现有输入 | 用途 |
| --- | --- | --- |
| S1 | [支付接入与 Rust 重构方案](README.md) | 已有完整方案；P0—P10是里程碑，不等于本台账执行编号。 |
| S2 | [应用授权与受控成员协议](../application-authorization-contract.md) | combined/legacy、memberLookups、身份和范围合同；最终以当前源码核对。 |
| S3 | [合并授权路由](../../backend/src/routes/app_authorize.rs)、[授权投影](../../backend/src/app_authorization/store.rs)、[策略](../../backend/src/data_grants/policy.rs) | 精确resource:action、应用绑定、当前membership grant和组织硬上限。 |
| S4 | [门店后端](../../business-apps/stores/backend/README.md)、[工单基线](../work-orders-rust-baseline.md) | 独立服务/库、精确金额、成员见证、幂等和未知结果参考。 |
| S5 | [组织管理路由](../../backend/src/routes/organizations.rs)、[组织策略](../../backend/src/authorization_policy.rs)、[用户管理](../../backend/src/routes/users.rs) | session型tree、当前无parent_id移动能力、全局用户与成员管理边界。 |
| S6 | [微应用 SDK](../../examples/micro-app-sdk/README.md) | 登记版本、生命周期、请求、下载及opaque iframe合同。 |
| S7 | [业务交付](../../deploy/microapps/README.md)、[网关生成](../../deploy/gateway/render.mjs)、[管理 CLI](../../tools/platform-admin/src/contract.rs) | URL/镜像/版本发布、当前固定应用入口及后续支付扩展。 |
| S8 | S1第23章外部源仓库定位表；外部项目名 `pfs-unifypay-monorepo`、`pfs-quickpay-monorepo` | 由执行环境显式挂载或定位源仓库；缺源记录为输入缺口，不把聊天记忆当源码，也不搜索无关盘符。 |

外部源仓库只按显式提供/定位的根目录读取，原项目名和目录合同见 S1 第23章。当前没有提供的生产库、凭据、域名、订单服务、映射文件均是待补输入，不视为已经具备。未来输出路径仅作规划，除本台账之外不表示文件已生成。

### Milestones and Executable Phases（里程碑与执行阶段）

原方案 P0—P10 保留为设计里程碑。为满足一个执行阶段能独立交付和验收的要求，拆为45个一期执行阶段；微信/支付宝协议核心、每种支付场景、退款审批/执行、inbox/outbox/通知及迁移接管分别交付。阶段数超过技能常见5—12的偏好，是因为这些有不同契约、资金副作用和验证终态，不能把整个后端或全部渠道压成一次执行。

| 原方案里程碑 | 本台账映射 |
| --- | --- |
| P0 契约与归属 | Phase 1；实际存量明细复核为Phase 37 |
| P1 库与隔离 | Phase 2、5；后续业务schema随对应阶段追加 |
| P2 身份权限 | Phase 3—7、12—13 |
| P3 普通支付 | Phase 14—22 |
| P4 回调与可靠任务 | Phase 23—25；旧新分发为Phase 39 |
| P5 退款与收款裁决 | Phase 26—29 |
| P6 收银台 | Phase 30—33 |
| P7 管理端与交付 | Phase 8—11、34—36、40—43 |
| P8 存量灰度 | Phase 37—45 |
| P9/P10 扩展资金与渠道 | E01—E09覆盖清单；先细分再执行，不作为一个超大阶段启动 |

## 2. Global Constraints（全局约束）

### Technical Stack

- G01：生产业务后端 Rust，优先现有 Axum/Tokio/SQLx/Reqwest/Serde/Tracing 经验；前端独立 Vite/React/TypeScript，支付宝客户端保留原生 TypeScript。
- 新支付库建议 PostgreSQL 17，是否保留MySQL双库必须在Q08冻结；旧Java维护期仍遵守原双库规范。数据库、HTTP与秘密策略独立验证。
- Node只用于前端/测试/构建发布工具；不引入Node生产业务后端、Bun、Caddy或Next.js。以实际仓库工具链和锁文件为基线，不顺手升级。

### Architecture Constraints

- G02：支付API、worker、数据库与静态前端独立部署；业务库账号不查平台身份表，不依赖平台/门店/工单业务实现crate。领域不依赖HTTP/SQL/渠道具体实现。
- 管理端使用HTML/opaque iframe与登记SDK。公开收银台独立入口，不能为调起付款给管理iframe增加同源、顶层导航或弹窗权限。
- 平台只管理应用登记、公开URL、预检和发布指针，不负责启动容器。KnownApp、网关和镜像入口的支付扩展是后续实际任务。
- G03：平台用户、密码、登录、全局账号/成员/角色是唯一管理身份来源；支付只保存平台UUID、最少快照和业务关联。用户管理使用宿主session与系统权限，应用token不冒充管理session。
- combined 使用 Basic + form(token/resource/action)；legacy显式配置，不失败自动回退。每请求重验并独立核对完整合同；不缓存正向授权。

平台permission严格是小写资源动作的 `resource:action`，例如建议 `pay-account:use` 对应账户使用，`pay-account:read/create/update` 对应管理动作，`pay-account-grant:read/create/update/revoke` 对应分享规则维护。USE/MANAGE/SHARE是本文业务能力术语，不直接作为大写平台action；最终资源清单在Phase1与原方案的pay-merchant等命名统一冻结，客户端/菜单/capabilities/接口使用同一清单。

### Business Constraints

- G04：tenant是硬隔离；订单来源组织不可伪造或普通编辑。支付商户、渠道账户、服务商应用、管理用户、渠道付款人分别建模。
- 一个逻辑业务订单一个意图、多个渠道尝试、一个有效收款。默认唯一键tenant/source/businessOrderId；源业务命名空间例外必须冻结，merchant/channel不是意图身份。
- 每attempt保存实际商户、原渠道账户/应用与版本；查关退与恢复沿原账户。配置版本与合法秘密轮换分开，未知历史版本不能由当前绑定补造。
- 金额用检查过的整数最小单位/明确十进制，费率和舍入规则固定；旧无时区时间先确认来源时区，大ID用字符串。
- G05：渠道超时、HTTP取消、数据库提交丢确认均可能已发生动作；沿稳定事件/渠道号查明，不生成新单号重复扣/退。不同退款号仍竞争原支付总额度，处理中/未知预占也扣减。
- G06：真实收款不能被异常回滚或乱序覆盖；多笔成功事实均保存。重复收款有唯一原路退款义务，未退回不得显示已退。普通支付与免押生命周期分开。

### Root Account Sharing Contract（根组织账户共享合同）

本节完整保存最新需求，后续执行不依赖聊天记录。

1. G07：支持同一租户内根组织或中间组织拥有账户，并显式授权本组织或某个组织子树USE。账户owner不变，渠道秘密留服务端，账户共享不复制账户。
2. USE只表示按获准商户/场景等约束使用收款资源；MANAGE维护允许的账户配置；SHARE维护授权受众。三者分别授权。USE/MANAGE均不能自动推导SHARE或向下继续转授。
3. 建议默认创建只给本组织使用；可选择“本组织及全部下级”或“指定子树”。产品如要求创建即全树可用，也必须在同事务显式生成可见、可撤销、可审计规则。Q03尚待冻结，不能把建议当已批准默认。
4. 子树规则包含锚点本组织与当前/未来有效后代，使用权动态计算；不按授权时静态展开ID列表冒充动态继承。首版默认不实现下级转授或复杂排除规则；有新要求先冻结合同。
5. G08：子组织得到的是当前业务组织的账户使用关联，而不是祖先管理数据范围。可用账户列表返回最小字段；账户管理列表仍按owner/管理范围。不能向context加入父组织来解决可见性。
6. 可信关系由平台窄用途合同或经批准的等价机制提供，不直查平台库、不相信前端parent/path。用户关系证明与机器关系证明分别认证；全局应用Basic不能自行访问任意租户组织。
7. 每次新受理同时满足平台身份/功能permission/data grant、组织硬上限、账户使用规则、账户状态及交易约束。业务API则满足获准机器租户/来源/组织和同一账户策略；付款会话只引用已受理的归属。
8. 至少一条完整有效USE路径覆盖目标组织、动作/场景、期限和限额；不能从不同规则各取宽松条件拼接放行。系统硬限额继续收窄。撤销一条规则后不能再借该规则放行；另有独立有效授权时按其判断，无有效规则时拒绝。
9. G09：新增后代在下次权威检查时自动继承；移出/移入按当前关系重算。当前平台没有组织移动API，本期不新增该功能，未来重挂只定义语义并用受控fixture验证；新增/启停等现有操作走正式API。
10. 规则/账户撤销或关系链停用阻止后续新受理；已受理交易继续原账户回调、查补及批准意图恢复。平台核验到支付库COMMIT仍有跨库竞态，默认不承诺全局原子强撤销。
11. G10：交易归实际来源组织。多个组织共用账户不互查订单；总部读下级交易也需对应交易permission/grant。资金仍按渠道账户/子商户真实合同结算，不自动分账、不自动给每个组织独立余额/银行入账。
12. 只有明确批准的同租户共享属于本期；平台根跨租户共享/运营是新能力，不以admin/super标志关闭tenant过滤。

账户规则的ORG/ORG_SUBTREE表示受众覆盖方式；平台data grant的scope表示当前用户对某资源动作的数据范围。两者是独立授权条件。账户使用用途按业务发起组织/使用关联核验平台范围，账户管理用途按owner/管理范围核验，不能要求下级拥有祖先账户的通用管理读取权限才能使用。

### Security / Compliance Constraints

- G11：四类非管理身份分别为商户/业务客户端、付款会话、渠道回调、worker。使用自己的认证与准入，不保存用户token供后台长期执行，不把openid当管理userId。
- 商户签名含明确版本/编码/时间窗/nonce，多副本防重放与资金幂等分开。通知目标、订单provider及跳转地址由服务端批准，拒绝任意URL与无控制redirect。
- 回调保留验签必需原字节，先验证、可靠收件再按渠道合同ACK；inbox/outbox/任务及其状态可恢复。共享账户加密回调先可信提取稳定单号，再查owner，不广播两个引擎。
- 导入或隔离恢复时保留原业务状态，另设全部可执行任务的迁移禁用门禁；jobs/outbox/商户通知/查补/退款均不能自动领取。演练出口仅受控fixture，owner和允许操作核对后才显式激活，不能误投旧生产notifyUrl。
- G12：秘密不进代码、文档、日志或前端产物；原文按必要性加密限期保存。审计与业务写同事务，actor区分管理用户/client/渠道/worker，不随外部用户删除级联交易。
- 真实资金、生产发布或第三方上线必须有具体获准操作域；创建台账只授权当前文档交付，不推导后续动作已批准。

### Code Style / Engineering Rules

- G13：遵守适用AGENTS，优先单crate内模块；薄入口厚库、依赖单向、公共API收敛，不提前创建无稳定复用的公共crate/workspace。
- 每阶段使用实际manifest进行fmt/check/clippy/test/doc及相称数据库/协议/浏览器验证；可执行的根命令要以真实工程形态为准。不能把未运行、失败或必需skip写成通过。
- 不改已执行迁移checksum；新schema用追加迁移，Flyway/SQLx历史不可直接互认。恢复需兼容构建，数据库回退不撤销渠道事实。
- 每阶段保留源码/产物摘要、环境/工具版本、执行命令、通过/失败/未运行、限制、清理与恢复记录。建议证据为docs/pay/validation/phase-NN.md，均为未来拟产出。

### Forbidden Actions

- G14：本次不执行Phase1，不创建业务代码/数据库，不请求渠道，不发布镜像/小程序，不修改全仓架构台账。
- 不按旧admin、重名、相同邮箱/电话自动合并或授权；不把当前导入人补为历史owner。
- 不关闭tenant/组织过滤来支持账户共享；不复制父账户密钥给下级；不让USE拥有转授、退款或转账权。
- 不在渠道HTTP时持长数据库锁，不依靠进程Mutex保证多副本安全，不以lease撤销已发请求。
- 不让旧新引擎同时写同单，不自动换渠道号恢复未知资金事件，不无条件广播回调，不以快照覆盖已发生资金事实。
- 不启动E01—E09整包；先根据实际范围细分成满足本台账格式的可执行阶段，并更新依赖/指针。未迁移能力不能悄悄停用其旧处理方。

### Status and Evidence Rules（状态与证据规则）

阶段只使用 NOT_STARTED、IN_PROGRESS、BLOCKED、COMPLETED。当前全为NOT_STARTED；文档创建不是某个支付实现阶段完成的证据。Q表的OPEN是问题状态，不是第五种执行阶段状态。

未来只有在收到执行请求且本阶段前置终态/外部输入满足后才进入IN_PROGRESS。遇到真实必需外部阻碍，记录已完成准备、具体阻碍和恢复条件；未执行的当前创建阶段不预先标BLOCKED。必需验收未通过不得COMPLETED，预算或输出文件存在不替代终态。

阶段完成时同步汇总、详情状态、当前指针、决策/变更和证据。当前默认按编号前进；只有依赖满足且用户授权范围允许时才并行。不得为追赶数量改变合同、删除必需验收或跨阶段偷偷实现后续内容。

## 3. Phase Status Summary（阶段状态汇总）

| Phase | Name | Status | Depends On | Output Summary |
| --- | --- | --- | --- | --- |
| 1 | 核对源码基线并冻结首期合同 | NOT_STARTED | 无 | docs/pay/contracts.md（拟建）：资源、身份、接口、状态及Q项冻结结果。 |
| 2 | 建立独立 Rust 服务与支付库迁移基线 | NOT_STARTED | 1 | business-apps/payments/backend（拟建）及自己的Cargo.lock、迁移和配置示例。 |
| 3 | 登记支付应用并接入平台授权 | NOT_STARTED | 2 | 支付authorization模块、资源动作映射与无秘密的平台登记/授权模板及正式API核对步骤。 |
| 4 | 实现用户用途的可信组织关系核验合同 | NOT_STARTED | 3 | 平台窄接口、支付关系客户端和合同文档。 |
| 5 | 实现支付账户聚合与版本化渠道配置 | NOT_STARTED | 2、3 | 账户/渠道/路由schema及管理API、审计、配置安全视图。 |
| 6 | 实现账户子树 USE 授权维护与撤销 | NOT_STARTED | 4、5 | 账户授权schema、创建/读取/版本更新/撤销API与规则解析器。 |
| 7 | 实现可用账户投影与统一使用门禁 | NOT_STARTED | 3、4、6 | 可用账户接口、UsageWitness/等价对象、后续支付用例统一入口。 |
| 8 | 接入支付管理微应用生命周期与平台登录态 | NOT_STARTED | 3 | business-apps/payments/frontend（拟建）骨架、路由和SDK登记消费。 |
| 9 | 实现账户与渠道配置管理页面 | NOT_STARTED | 5、8 | 账户/配置页面及对应API适配器。 |
| 10 | 实现根组织账户分享与可用账户选择界面 | NOT_STARTED | 6、7、8、9 | 账户授权与撤销页面、下级可用账户选择器。 |
| 11 | 交付隔离测试部署与支付入口路由 | NOT_STARTED | 2、8 | payments隔离部署与构建模板、路由清单和健康检查。 |
| 12 | 实现商户客户端签名与重放防护 | NOT_STARTED | 2、5 | 商户/业务客户端认证模块、凭据元数据和授权动作过滤。 |
| 13 | 实现机器租户准入与组织关系核验 | NOT_STARTED | 4、6、7、12 | 机器准入/关系服务契约及必要平台受控实现、支付适配器。 |
| 14 | 实现权威订单桥接与支付意图幂等 | NOT_STARTED | 5、12、13 | 意图schema、订单provider适配器、创建/查询和同键恢复API。 |
| 15 | 实现微信 HTTP 与密码协议适配核心 | NOT_STARTED | 2、5 | 微信协议client与回调验证适配器。 |
| 16 | 实现支付宝 HTTP 与密码协议适配核心 | NOT_STARTED | 2、5 | 支付宝协议client、验证器及黄金向量。 |
| 17 | 实现支付尝试与持久渠道调用意图 | NOT_STARTED | 7、13、14 | 支付用例、attempt/command schema及渠道port/fake。 |
| 18 | 接入微信 JSAPI 下单场景 | NOT_STARTED | 15、17 | JSAPI适配器、DTO和受控HTTP合同样本。 |
| 19 | 接入微信 H5 下单场景 | NOT_STARTED | 11、15、17 | H5适配与返回地址构造。 |
| 20 | 接入支付宝 WAP 下单场景 | NOT_STARTED | 11、16、17 | 支付宝WAP适配器与返回DTO。 |
| 21 | 接入支付宝小程序 LITE 下单场景 | NOT_STARTED | 16、17 | LITE适配器、DTO及渠道合同样本。 |
| 22 | 实现权威查单、严格关单与查补用例 | NOT_STARTED | 15、16、17、18、19、20、21 | query/close端口实现和查补用例、状态映射。 |
| 23 | 实现渠道回调可靠收件与状态推进 | NOT_STARTED | 15、16、17、22 | 回调路由、inbox schema与处理器。 |
| 24 | 实现事务 outbox 与可恢复工作进程 | NOT_STARTED | 22、23 | outbox/jobs schema、独立或同包worker入口和调度器。 |
| 25 | 实现签名商户通知与受控重放 | NOT_STARTED | 12、24 | 商户通知投递器、通知状态API和受控接收方fixture。 |
| 26 | 实现人工退款申请与复核状态机 | NOT_STARTED | 3、5、17 | 退款请求schema、申请复核API、审批审计和状态合同。 |
| 27 | 实现微信与支付宝渠道退款合同 | NOT_STARTED | 15、16、22 | 两渠道退款/退款查询适配器、通知解析和统一状态映射。 |
| 28 | 实现退款额度预占与持久执行 | NOT_STARTED | 13、24、25、26、27 | 退款执行schema/用例、预占不变量、查询和恢复接口。 |
| 29 | 实现重复成功收款裁决与自动退款义务 | NOT_STARTED | 17、23、25、28 | 重复收款裁决用例、系统退款批准依据及差异记录。 |
| 30 | 实现收银台支付会话与旧接口兼容层 | NOT_STARTED | 11、14、17、22、23、28、29 | cashier session API、旧路径兼容合同、受权结果查询。 |
| 31 | 实现付款人 OAuth 与一次性状态流 | NOT_STARTED | 15、16、30 | 付款OAuth端点及旧redirect/authorize兼容接线。 |
| 32 | 接入 QuickPay WAP 与服务端结果恢复 | NOT_STARTED | 18、19、20、30、31 | WAP迁入或受管工程、环境公开配置、结果恢复UI。 |
| 33 | 接入支付宝原生小程序与结果回查 | NOT_STARTED | 21、30、31 | 原生客户端适配、环境配置与结果状态UI。 |
| 34 | 实现交易管理查询接口与页面 | NOT_STARTED | 8、17、22、25、29 | 管理交易范围SQL、list/count/detail/export/close HTTP接口及页面/API客户端。 |
| 35 | 实现退款申请、复核及执行结果页面 | NOT_STARTED | 8、26、28、29、34 | 退款工作流页面、API适配、角色矩阵。 |
| 36 | 实现通知、补偿与支付审计运维页面 | NOT_STARTED | 8、24、25、28 | 支付运维页面和最小管理接口。 |
| 37 | 盘点旧数据、身份映射与原账户证据 | NOT_STARTED | 1、5、6、17、28 | 带批次/摘要的归属、用户、账户和实体映射文件规范与受控数据清单。 |
| 38 | 实现核心导入器与隔离迁移核对 | NOT_STARTED | 28、29、37 | 核心迁移工具、批次状态/校验报告、可恢复重跑流程。 |
| 39 | 实现旧新引擎 owner 路由与可信回调分发 | NOT_STARTED | 11、23、24、37、38 | 兼容路由/收件分发实现和部署合同。 |
| 40 | 固化全链路 CI 与授权隔离回归 | NOT_STARTED | 10、11、18、19、20、21、22、23、24、25、26、27、28、29、30、31、32、33、34、35、36、38、39 | 支付CI入口、用例清单及矩阵结果。 |
| 41 | 演练备份恢复与兼容回退 | NOT_STARTED | 38、39、40 | 恢复/回退运行手册和隔离演练记录。 |
| 42 | 验证容量预算与授权模式默认值 | NOT_STARTED | 40 | 容量报告、默认模式决定、预算配置与告警阈值。 |
| 43 | 完成获准渠道与真实客户端联合验收 | NOT_STARTED | 40、41、42 | 获准操作范围和各场景联合结果、资金核对报告。 |
| 44 | 执行获准灰度、切换核对与回退演练 | NOT_STARTED | 37、38、39、40、41、42、43 | 发布/停受理/回退执行清单、灰度前后owner和金额核对。 |
| 45 | 完成一期验收与未迁移义务交接 | NOT_STARTED | 1、2、3、4、5、6、7、8、9、10、11、12、13、14、15、16、17、18、19、20、21、22、23、24、25、26、27、28、29、30、31、32、33、34、35、36、37、38、39、40、41、42、43、44 | 一期验收与交付报告、运行手册索引、扩展阶段拆分入口。 |

### 后续能力覆盖矩阵（不是可直接执行的超大阶段）

以下为全量替代目标的明确未完成范围。所有项为NOT_STARTED；Phase1和37确认实际使用、在途义务和负责人，Phase45交接，之后先在本文件细分阶段。不能因为一期不做就标COMPLETED/BLOCKED，也不能自动丢弃原引擎。

| ID | 能力 | Status | 未来必须细分的范围 | 全量覆盖完成判据 | 责任角色 |
| --- | --- | --- | --- | --- | --- |
| E01 | 余额、冻结与账本 | NOT_STARTED | 账户余额/冻结不变量、追加分录、币种、费率舍入、历史余额/流水校验；先拆模型、入账、核对、接管阶段。 | 余额/冻结/流水逐笔可解释，重复事件不重复入账，恢复后金额一致。 | 财务/支付架构负责人 |
| E02 | 渠道转账 | NOT_STARTED | 收款身份、限额、渠道提交/查单/通知、未知结果与恢复；按渠道及用例拆分。 | 稳定转账号贯穿未知结果，成功/失败/关闭差异处理可核对。 | 支付产品/渠道负责人 |
| E03 | 充值 | NOT_STARTED | 充值意图、普通支付关联、到账幂等、余额入账、退款/冲销政策。 | 重复成功回调只入账一次，支付事实与充值账本核对一致。 | 财务/支付产品负责人 |
| E04 | 余额付款 | NOT_STARTED | 付款申请/复核、冻结、转账执行、成功扣减/失败解冻及异常义务。 | 未知转账不提前解冻，累计余额不变量和审批约束通过。 | 财务/支付产品负责人 |
| E05 | 商户进件与渠道授权 | NOT_STARTED | 资料与敏感数据、渠道授权、进件状态、查询回调及人员职责；与管理用户登录分离。 | 渠道身份、资料范围、状态和历史凭据接管可追溯。 | 商户运营/渠道负责人 |
| E06 | 微信发货与履约同步 | NOT_STARTED | 订单/渠道交易映射、履约权限、上传、查询、补偿与失败处置。 | 只同步获准来源订单，重试幂等，支付成功不被等同于发货成功。 | 履约系统负责人 |
| E07 | 支付宝免押 | NOT_STARTED | 授权冻结、查询/撤销、扣费、解冻/退款、服务单同步与在途接管，按生命周期切阶段。 | 冻结和扣费义务完整，原渠道身份可用，重复/未知动作不丢义务。 | 支付产品/渠道负责人 |
| E08 | 微信支付分 | NOT_STARTED | 创建/确认、查询/取消、完结异步扣费、回调/查补、服务商模式和不支持能力合同。 | 真实服务单、扣费与退款闭环；不支持多次部分扣费等限制显式保留。 | 支付产品/渠道负责人 |
| E09 | 完整对账与差异处置 | NOT_STARTED | 账单获取/解析、支付/退款/手续费核对、差异工单、复核处置、重跑与历史差异迁移。 | 原始账单可追溯，差异有明确终态及处理依据，不用流水页面冒充对账。 | 财务/对账负责人 |

每个实际使用扩展必须登记：旧系统支持证据、当前运行引擎、未结清清单及金额、运行责任人、后续阶段/依赖、获准渠道条件和退役门禁。缺少实际责任人时记录待确认，不能将角色名称当已完成交接。智能成本路由、银联、TCC等未核实旧实现的新功能不自动纳入等价迁移范围，新增需求另行立项。

## 4. Phase Details（阶段详情）

### Phase 1: 核对源码基线并冻结首期合同

#### Status

NOT_STARTED

#### Goal

形成可追溯的首期实现输入，先确定账户共享、订单权威与资金归属。

#### Scope

只读核对三个仓库；定义支付账户的业务对象、首期场景、业务唯一键、退款政策、机器准入与存量责任；整理Q表。

#### Out of Scope

不创建业务服务、迁移数据库、调用真实渠道或执行生产变更。

#### Inputs

S1支付方案、S2平台授权协议、S3—S8源码入口；Q01—Q21；外部源仓库需由执行环境显式定位。

输入门禁：Q01—Q21按影响逐项分流；进入后续阶段必须满足其列出的输入门禁。

#### Execution Requirements

- 记录实际源码版本或工作树摘要，区分现状、设计和外部未知。
- 冻结逻辑账户与支付商户/渠道账户关系、USE/MANAGE/SHARE、动态子树与结算合同；缺失信息写入Q表并阻止依赖动作。

#### Expected Outputs

- docs/pay/contracts.md（拟建）：资源、身份、接口、状态及Q项冻结结果。
- 来源能力清单、源文件定位、源数据/在途义务盘点模板及首期场景启用表。

#### Acceptance Criteria

- [ ] 合同明确订单来源、单号命名空间、真实业务组织及资金接收主体，或对未解决项设置对应阶段硬门禁。
- [ ] 账户默认分享策略、未来后代语义、转授限制、用户与机器关系核验分别有定义。
- [ ] 每个首期必需Q项有结论或具体阻碍、责任人和恢复条件；不能把未决核心输入当作已冻结。

#### Risks

源仓库与现网不一致、缺失收银台订单服务、口头建议被误写为已批准业务规则。

#### Dependencies

无前置实现阶段；先完成本阶段可进行的只读核对，外部输入按Q表处理。

#### Notes

可以先完成静态盘点并提出具体问题；外部输入不足时保留真实状态，不伪造合同。方案P0映射到本阶段。

证据：拟保存为 docs/pay/validation/phase-01.md；当前尚不存在，不代表已验证。

---

### Phase 2: 建立独立 Rust 服务与支付库迁移基线

#### Status

NOT_STARTED

#### Goal

交付可独立启动、隔离数据库且不开放资金操作的支付服务骨架。

#### Scope

独立crate/锁文件、薄入口厚库、配置校验、专属数据库、基础迁移、金额/ID类型、live/ready和安全错误。

#### Out of Scope

不实现下单、退款、账户分享或平台内部表读取。

#### Inputs

Phase1冻结的数据库/标识合同；S4门店与工单工程案例；Q08已解决。

输入门禁：Q08数据库合同；缺失时不开始schema实现。

#### Execution Requirements

- 按仓库实际工具链与依赖锁实现，领域类型不依赖HTTP/SQL实现。
- 启动前校验配置和schema；运行账号与平台/其他业务库隔离，生产迁移权限边界明确。

#### Expected Outputs

- business-apps/payments/backend（拟建）及自己的Cargo.lock、迁移和配置示例。
- 空库、重复启动、坏schema、金额精度/溢出、日志脱敏验证。

#### Acceptance Criteria

- [ ] 独立build/check/test/fmt/clippy/doc按实际manifest执行，结果逐项留证。
- [ ] 非超级业务账号不能查询平台身份表；迁移失败不开放业务监听。
- [ ] 金额使用检查过的最小货币单位，标识/时间/币种合同与Phase1一致。

#### Risks

照搬现有业务唯一约束、共享平台连接或将健康检查误当业务授权可用。

#### Dependencies

前置执行阶段：Phase 1。

#### Notes

所有新增路由先无资金副作用；所有示例秘密均为无效占位。

证据：拟保存为 docs/pay/validation/phase-02.md；当前尚不存在，不代表已验证。

---

### Phase 3: 登记支付应用并接入平台授权

#### Status

NOT_STARTED

#### Goal

支付管理API逐请求取得可信用户、租户、组织、permission和data grant。

#### Scope

combined授权、显式legacy兼容、合同验证、期限/并发/容量预算、安全身份对象与拒绝路径。；可复用的平台登记/授权清单、隔离初始化和普通用户fixture。

#### Out of Scope

不改平台用户密码体系、不缓存正向权限、不提供跨租户运营豁免。

#### Inputs

S2、S3；Phase1权限/路由清单；可用的真实隔离平台及有权执行正式管理API的测试管理员，不假定payments应用/客户端已经登记。

输入门禁：Q18跨租户运营默认不开放；Q14账号候选核对另在Phase37。

#### Execution Requirements

- combined使用Basic加form token/resource/action；legacy显式选择，失败不自动降级。
- 独立验证aud/app/client/身份/有效期、grant主体和组织硬上限；浏览器props不作权威。
- 在隔离环境通过正式API登记payments应用/实际机密客户端关联、MENU/BUTTON、精确permission；产出capabilities/memberLookups策略、租户/组织两级授权、角色菜单及membership grant清单。保留已有授权，不直写平台表；秘密只在受控配置。

#### Expected Outputs

- 支付authorization模块、资源动作映射与无秘密的平台登记/授权模板及正式API核对步骤。
- 两个模式的契约与真实平台拒绝测试、预算/未知结果说明。

#### Acceptance Criteria

- [ ] 错应用、错客户端、停用用户/成员、撤销会话、缺permission/grant、未支持SELF均被拒绝。
- [ ] 平台不可达、正文超限、并发满或证明过期不使用旧正向结果兜底。
- [ ] 新受保护请求重新验证，现有平台用户管理接口仍由session/system权限保护。
- [ ] 从未登记payments的隔离平台可按清单完成应用/客户端/菜单/策略/两级授权/角色/grant准备，普通测试用户只获得批准范围；后续阶段按清单增量维护，fixture临时注册不代替正式产物。

#### Risks

只验JWT签名、按角色名字放行、把DATA_GRANT_POLICY登记范围当作历史grant已清除。

#### Dependencies

前置执行阶段：Phase 2。

#### Notes

创建支付应用/测试授权只在获准隔离环境通过正式API执行，不在当前台账创建时执行。

证据：拟保存为 docs/pay/validation/phase-03.md；当前尚不存在，不代表已验证。

---

### Phase 4: 实现用户用途的可信组织关系核验合同

#### Status

NOT_STARTED

#### Goal

让下级使用祖先账户时可以验证真实从属关系，同时不扩大其context。

#### Scope

平台应用专用有界关系核验接口、客户端/应用/用途绑定、同租户祖先判断、证明期限和版本。；用于分享界面的用途限定、分页最小组织候选目录。

#### Out of Scope

不开放全量组织目录，不使用名称path，不新增组织移动功能，不把机器Basic视为用户上下文。

#### Inputs

Phase1组织关系合同；S2/S3；S5组织路由/授权投影。

输入门禁：Q03/Q04共享范围语义；Q10机器合同不在本阶段冒充完成。

#### Execution Requirements

- 建议新端点POST /api/apps/{appCode}/organization-relations/check；actorOrganizationId取自token，targetOrganizationId缺省为当前组织。指定目标时先验证用途grant/组织硬上限，再计算候选祖先到实际target的关系，而非只检查操作者组织。
- 关系响应和UsageWitness绑定app/tenant/user、actorOrg、targetOrg、resource/action、祖先锚点、期限和relationRevision；候选祖先UUID只作条件。组织硬上限不包含祖先也不因此扩张；限制候选数、正文和并发。
- 另提供或复用已核实可用的应用用途组织候选合同，候选只在当前管理用途grant/硬上限内返回id/name；不调用session型全树接口，不改变context wire。

#### Expected Outputs

- 平台窄接口、支付关系客户端和合同文档。
- 同租户祖先链、伪造path、越界/停用/过期、撤权与不可达测试。

#### Acceptance Criteria

- [ ] 下级可以验证自己属于获准祖先子树，响应不加入祖先到organizationScopeIds。
- [ ] 跨租户、假祖先、停用祖先链、过期证明和客户端错应用均拒绝。
- [ ] 新增后代真实API可验证；未来重挂语义仅用受控关系fixture测试，不能宣称已有移动API。
- [ ] 账户owner分享界面可取得允许目标组织候选，下级只获得必要关系判断，不开放全租户目录。
- [ ] 同一操作者可对目标A允许、目标B拒绝；交换targetOrg或用途复用关系证明失败，当前组织证明不能用于另一业务目标。

#### Risks

组织tree接口要求管理session；把缓存/名称路径当组织关系权威。

#### Dependencies

前置执行阶段：Phase 3。

#### Notes

当前平台不支持parent_id修改，本阶段仅为未来变化定义语义；机器关系另由Phase13受控实现。

证据：拟保存为 docs/pay/validation/phase-04.md；当前尚不存在，不代表已验证。

---

### Phase 5: 实现支付账户聚合与版本化渠道配置

#### Status

NOT_STARTED

#### Goal

建立同租户、明确owner组织的账户及安全可追溯配置。

#### Scope

逻辑PaymentAccount到支付商户/渠道账户的唯一映射；账户CRUD、owner、配置版本、脱敏读和路由默认。

#### Out of Scope

不实现子树USE、资金调用、跨租户共享或新账户自动向全部组织开放。

#### Inputs

Phase1账户与结算对象合同；Phase2库基线、Phase3管理鉴权；Q02/Q05/Q18已冻结。

输入门禁：Q02账户对象、Q05真实结算/法人、Q18共享技术配置。

#### Execution Requirements

- 账户与渠道技术身份分开建模；若复用pay_merchants作为账户存储，不再维护两套冲突owner。
- 保存不可变配置版本及秘密引用，公共默认与组织路由覆盖分开；更新需版本、管理范围与同事务审计。

#### Expected Outputs

- 账户/渠道/路由schema及管理API、审计、配置安全视图。
- 同租户复合约束、owner不可普通修改、跨域拒绝及配置轮换测试。

#### Acceptance Criteria

- [ ] 普通有权管理用户可维护其范围内账户；只有USE或其他组织用户不能取得管理详情/秘密。
- [ ] 配置版本可定位，密钥不进入HTTP响应、日志和前端产物。
- [ ] 同账户真实渠道商户/子商户和结算主体可从冻结配置确定；组织字段不被当成分账。

#### Risks

将平台租户、支付商户和渠道账户混成一个ID；组织覆盖改动影响所有组织。

#### Dependencies

前置执行阶段：Phase 2、Phase 3。

#### Notes

创建时默认分享选择须由Phase1决定，规则持久化接入Phase6后才启用相应选择。

证据：拟保存为 docs/pay/validation/phase-05.md；当前尚不存在，不代表已验证。

---

### Phase 6: 实现账户子树 USE 授权维护与撤销

#### Status

NOT_STARTED

#### Goal

账户归属方可以一次授予本组织或某组织子树使用权，禁止无权扩权。

#### Scope

统一账户组织授权表、ORG/ORG_SUBTREE覆盖、用途/场景/期限、SHARE权限、版本及审计；必要用户商户限制。

#### Out of Scope

默认不实现下级转授；不把USE自动升级为MANAGE/SHARE；不授予跨租户账户使用。

#### Inputs

Phase1授权政策；Phase4可信关系；Phase5账户owner；Q03/Q04已冻结。

输入门禁：Q03创建默认、未来组织；Q04转授/例外及强撤销要求。

#### Execution Requirements

- 方案中的固定组织USE关联作为ORG规则统一解析，不并行保留另一套可OR放行的权威。
- 授予范围受账户owner或明确委派边界约束，创建/撤销/启停与审计同事务；作用范围不可超过授权人的SHARE能力。
- 分享规则的目标组织与关系证明使用相同target语义；授予锚点/后代覆盖不得通过更换目标组织复用旧见证。

#### Expected Outputs

- 账户授权schema、创建/读取/版本更新/撤销API与规则解析器。
- 根含后代、指定分支、期限、撤权、越权转授及并发变更验证。

#### Acceptance Criteria

- [ ] 一条根子树规则覆盖根及当前/未来有效后代，不按授权时ID快照假装动态继承。
- [ ] 只有USE或只有MANAGE而无SHARE时不能新增受众；跨租户和平级越界请求失败。
- [ ] 撤销规则后不能再借该规则放行；没有其他有效授权时拒绝新受理，原交易回调和已批准意图仍按存量政策处理。
- [ ] 账户创建的分享选项显式落规则且留审计；缺省行为与Phase1一致。

#### Risks

无关联被当作全开放、多条规则相互扩大范围、撤权误删历史单。

#### Dependencies

前置执行阶段：Phase 4、Phase 5。

#### Notes

若未来允许转授/排除子树，先补充独立合同及验收；首版不得通过隐含继承引入。

证据：拟保存为 docs/pay/validation/phase-06.md；当前尚不存在，不代表已验证。

---

### Phase 7: 实现可用账户投影与统一使用门禁

#### Status

NOT_STARTED

#### Goal

下级能使用获准祖先账户，且账户列表和实际受理采用相同策略。

#### Scope

accessible账户查询、最小脱敏投影、目标组织验证、账户/规则版本见证及使用门禁。

#### Out of Scope

不放宽祖先管理数据范围，不通过账户共享授予兄弟组织交易读取权。

#### Inputs

Phase3身份、Phase4关系证明、Phase5—6账户与授权；Phase1业务组织合同。

输入门禁：Q04撤销语义；Q17预算上限在合同中冻结。

#### Execution Requirements

- 可用查询基于当前或获准目标组织的使用关联，不对根账户owner套普通子组织管理列表过滤。
- 后端实际受理必须再次执行同一门禁，不能只相信前端选项；关系检查后短事务复核账户/授权版本和期限。
- UsageWitness绑定实际业务targetOrg、actor/app/tenant/用途、账户和命中规则版本；名单查询、受理和持久意图使用相同target，交换目标或用途拒绝。

#### Expected Outputs

- 可用账户接口、UsageWitness/等价对象、后续支付用例统一入口。
- 名单/直传accountId一致性与撤权并发测试。

#### Acceptance Criteria

- [ ] 子组织看到名称/渠道/场景等必要字段但不能读根账户管理详情和秘密。
- [ ] 子树外、跨租户、缺平台权限/grant、停用账户/关系、证明过期均拒绝。
- [ ] 新建下级继承、未来移出/移入fixture和同商户不同订单组织隔离有证据。
- [ ] 文档承认平台检查到支付库COMMIT的跨库竞态，不承诺已发资金动作可立即撤销。

#### Risks

只过滤下拉框、使用owner范围拒绝合法子组织、旧正向关系缓存继续放行。

#### Dependencies

前置执行阶段：Phase 3、Phase 4、Phase 6。

#### Notes

此阶段只接用户用途；业务客户端接入同一账户策略的入口在Phase13完成。

证据：拟保存为 docs/pay/validation/phase-07.md；当前尚不存在，不代表已验证。

---

### Phase 8: 接入支付管理微应用生命周期与平台登录态

#### Status

NOT_STARTED

#### Goal

独立管理前端通过现有SDK运行，使用平台身份和组织上下文。

#### Scope

Vite入口、固定SDK、mount/update/unmount、内存路由、权限展示、请求客户端和身份分区。

#### Out of Scope

不保留旧独立登录/注册/密码和localStorage token，不实现全部业务页面。

#### Inputs

S6 SDK；旧管理前端；Phase3登记/授权模板、普通用户fixture和API身份合同；冻结appCode/basePath。

输入门禁：Q16测试宿主origin可先用隔离本地环境。

#### Execution Requirements

- 使用当前应用Bearer、credentials:omit、相对资源；组织/身份变化清理旧视图并拒绝迟到回填。
- 纯主题更新不触发资金请求；未知写入事件只保存必要键/正文摘要引用，等待服务端核对。

#### Expected Outputs

- business-apps/payments/frontend（拟建）骨架、路由和SDK登记消费。
- 生命周期、Origin:null、组织切换/撤权和异常恢复测试。

#### Acceptance Criteria

- [ ] 普通平台用户能在获准组织进入，直接打开入口没有上下文时不伪造登录。
- [ ] 无旧admin缺省回退；凭据不进URL、持久浏览器存储或构建产物。
- [ ] 切组织、token更新、403/401和卸载清理行为通过当前支持浏览器验证。

#### Risks

复用旧AuthProvider残留独立token、前端判断替代后端授权。

#### Dependencies

前置执行阶段：Phase 3。

#### Notes

该阶段先提供最小测试页面；业务页面按Phase9/10/34—36接入。

证据：拟保存为 docs/pay/validation/phase-08.md；当前尚不存在，不代表已验证。

---

### Phase 9: 实现账户与渠道配置管理页面

#### Status

NOT_STARTED

#### Goal

管理用户通过宿主身份维护获准账户及其配置版本。

#### Scope

账户列表/详情/创建/版本更新、脱敏渠道配置、默认路由与影响范围展示。

#### Out of Scope

不提供资金转账/余额操作，不复制全部旧通用用户管理页面。

#### Inputs

Phase5管理API、Phase8生命周期、原UnifyPay相关页面。

输入门禁：Q02/Q05配置及结算字段应已冻结。

#### Execution Requirements

- UI使用精确菜单/动作权限，保留409输入，未知更新先受权读回。
- 共享默认变更展示影响组织，秘密输入遵守服务端引用合同且不回显原秘密。

#### Expected Outputs

- 账户/配置页面及对应API适配器。
- 普通管理用户、只USE用户、版本冲突与秘密产物扫描证据。

#### Acceptance Criteria

- [ ] 有管理权限者仅在获准范围修改；只有使用权用户没有管理入口且后端拒绝直调。
- [ ] 更改配置生成新版本，旧尝试引用保持可用。
- [ ] 同身份草稿与组织切换隔离，响应中没有渠道秘密。

#### Risks

页面复用时保留全局商户列表或把管理账户作为可用账户选择器。

#### Dependencies

前置执行阶段：Phase 5、Phase 8。

#### Notes

账户分享范围的创建选项由Phase10接线，尚未完成时不展示可用的分享按钮。

证据：拟保存为 docs/pay/validation/phase-09.md；当前尚不存在，不代表已验证。

---

### Phase 10: 实现根组织账户分享与可用账户选择界面

#### Status

NOT_STARTED

#### Goal

账户owner能清晰授予子树USE，下级只选择其真实可用账户。

#### Scope

仅本组织/本组织及全部下级/指定组织子树、规则详情/撤销、动态未来后代提示、可用账户组件。

#### Out of Scope

不新增组织移动功能，不把组织树管理接口开放给普通支付token。

#### Inputs

Phase6—7授权/accessible API、Phase8—9界面；Phase4窄关系与可管理受众候选合同。

输入门禁：Q03默认分享；Q04例外/委派仅在明确纳入时实现。

#### Execution Requirements

- 授权范围选择从受控用途接口得到，不直接读取不具权限的全量平台tree。
- 明确USE/MANAGE/SHARE和真实结算主体差别；撤销需显示影响的新受理范围并遵循版本合同。

#### Expected Outputs

- 账户授权与撤销页面、下级可用账户选择器。
- 根/分支/门店/兄弟组织角色矩阵及新建子组织联动证据。

#### Acceptance Criteria

- [ ] 一条根子树规则可见、可撤销、可审计，不要求逐下级复制账户。
- [ ] 无SHARE用户不能修改受众，选择器不返回秘密或兄弟交易。
- [ ] 新建组织按动态规则取得资格；缺应用/用户权限者仍被拒绝。

#### Risks

组织选择器需要新增信息却绕用system:organization；默认全开没有明确产品决定。

#### Dependencies

前置执行阶段：Phase 6、Phase 7、Phase 8、Phase 9。

#### Notes

仅本组织作为当前建议默认，是否根创建即全树开放仍由Q03冻结，不把前轮分析当批准。

证据：拟保存为 docs/pay/validation/phase-10.md；当前尚不存在，不代表已验证。

---

### Phase 11: 交付隔离测试部署与支付入口路由

#### Status

NOT_STARTED

#### Goal

为后续OAuth与渠道联调提供真实且独立的HTTPS入口。

#### Scope

Rust/静态镜像、测试Compose、管理/公开/回调路由、CORS/CSP、精确origin、秘密注入和基础CI构建。

#### Out of Scope

不推送生产、不激活正式支付入口、不修改现有业务路由行为。

#### Inputs

S7部署/网关；Phase2/8产物；Phase1路径合同；Q16测试域名和证书。

输入门禁：Q16测试域名/证书；缺失时只可交付模板但不得标本阶段完成。

#### Execution Requirements

- 扩展当前固定KnownApp/镜像/网关入口或采用正式REST，不声称登记URL自动部署服务。
- 公开API分组独立中间件；iframe不增加同源/顶层导航能力，metrics保持私有。

#### Expected Outputs

- payments隔离部署与构建模板、路由清单和健康检查。
- 固定产物摘要、HTTPS/CORS/CSP/正文透传与秘密扫描证据。

#### Acceptance Criteria

- [ ] 独立镜像与服务可在私有测试环境运行，支付库与平台库账号隔离。
- [ ] 各路径落到正确身份链，管理路由不能因公开前缀被免鉴权。
- [ ] 后续渠道回调和OAuth测试地址有精确登记输入，未完成场景保持禁用。

#### Risks

把部署模板当实测、提前生产发布、代理改写签名原文。

#### Dependencies

前置执行阶段：Phase 2、Phase 8。

#### Notes

测试入口先行，完整CI及场景总验收在Phase40/43；生产发布另由Phase44明确授权。

证据：拟保存为 docs/pay/validation/phase-11.md；当前尚不存在，不代表已验证。

---

### Phase 12: 实现商户客户端签名与重放防护

#### Status

NOT_STARTED

#### Goal

业务系统以受限机器身份访问支付服务，凭据不继承管理员权限。

#### Scope

业务来源登记、租户组织商户/动作绑定、HMAC版本兼容、持久nonce、限流与密钥轮换。

#### Out of Scope

不创建真实支付、不把平台client_credentials当租户context。

#### Inputs

Phase1调用方/旧HMAC合同；旧ApiAuth实现；Phase2/5；Q12已冻结。

输入门禁：Q12调用方兼容；Q07稳定单号范围。

#### Execution Requirements

- 按版本冻结canonical request和原始字节，签名凭据与平台Basic secret分离。
- 认证nonce与资金幂等分开；稳定来源编码不因客户端/密钥轮换变化，跨副本共享防重放状态。

#### Expected Outputs

- 商户/业务客户端认证模块、凭据元数据和授权动作过滤。
- 签名黄金向量、nonce竞争、过期/重放、轮换及跨租户拒绝测试。

#### Acceptance Criteria

- [ ] 错签名/来源/动作/租户/商户均拒绝，未登记客户端默认不能调用。
- [ ] 使用新认证nonce恢复原事件时不重新生成资金幂等身份。
- [ ] 日志和错误不回显canonical敏感正文、密钥或签名。

#### Risks

兼容旧全局商户secret后丢失租户边界、客户端ID轮换变成新业务来源。

#### Dependencies

前置执行阶段：Phase 2、Phase 5。

#### Notes

动态组织和支付开通准入未通过Phase13前，不开放机器新资金受理。

证据：拟保存为 docs/pay/validation/phase-12.md；当前尚不存在，不代表已验证。

---

### Phase 13: 实现机器租户准入与组织关系核验

#### Status

NOT_STARTED

#### Goal

无人值守业务客户端也能安全使用获准祖先账户，无需保存用户token。

#### Scope

明确机器应用开通/租户状态准入、逐租户受限关系合同、支付客户端绑定及统一USE门禁接线。

#### Out of Scope

不让全局payments Basic查询任意租户，不借管理员session或用户token执行后台受理。

#### Inputs

Phase1机器准入合同；Phase4用户关系模式作参考；Phase12机器身份；Q10已解决。

输入门禁：Q10权威租户/应用准入和同步/故障策略。

#### Execution Requirements

- 平台机器接口需明确服务客户端、用途及批准租户/组织上限，支付业务客户端再收窄到已登记来源和目标组织。
- 每次新受理获取新鲜状态/关系见证或合同明确的同等机制，失联/过期/版本变化拒绝；已受理意图恢复另按存量政策。

#### Expected Outputs

- 机器准入/关系服务契约及必要平台受控实现、支付适配器。
- 租户/应用停用、错tenant/org、证明过期、平台不可达和共享账户机器使用测试。

#### Acceptance Criteria

- [ ] 没有用户在线时，已登记机器客户端仍能完成新鲜租户/组织准入、关系核验和USE判定，不保存或伪造用户token；本阶段不声称实际资金执行和持久意图恢复已完成。
- [ ] 应用Basic本身不足以越租户，任何机器关系查询均有批准的租户约束。
- [ ] 授权/组织变化对下一次新受理生效，跨库最后检查至提交的竞态有明确边界。

#### Risks

现有平台没有通用机器租户授权端点，不能靠未实现接口完成测试。

#### Dependencies

前置执行阶段：Phase 4、Phase 6、Phase 7、Phase 12。

#### Notes

若Q10合同未确定，本阶段保留未完成，后续机器支付/公开会话受理不得标可用。 已受理意图的实际恢复在Phase24/28/40验收。

证据：拟保存为 docs/pay/validation/phase-13.md；当前尚不存在，不代表已验证。

---

### Phase 14: 实现权威订单桥接与支付意图幂等

#### Status

NOT_STARTED

#### Goal

从可信订单来源建立唯一支付意图，金额和业务组织均由服务端确认。

#### Scope

订单provider合同、稳定source、订单修订/可付状态、归属/金额/币种、幂等receipt和规范正文摘要。

#### Out of Scope

不信任QuickPay自报tenant/org/amount，不凭空生成业务订单，不发送渠道支付。

#### Inputs

Phase1订单来源与唯一键；Phase13机器准入；Q06/Q07已解决。

输入门禁：Q06原订单/链接服务；Q07业务单命名空间与修订语义。

#### Execution Requirements

- 默认tenant/source/businessOrderId唯一；确为组织内局部单号才采用已冻结服务端命名空间，merchant/channel不进入意图身份。
- 同业务单换商户/组织/请求键不能绕过唯一性；订单变价/取消按合同冲突或有审计的状态转换处理。

#### Expected Outputs

- 意图schema、订单provider适配器、创建/查询和同键恢复API。
- 同键同体/异体、跨来源/组织、金额变更、provider失联及提交未知测试。

#### Acceptance Criteria

- [ ] 权威订单提供归属和金额，访问任意URL或浏览器Base64不能取得订单权限。
- [ ] 同一逻辑订单只有一个意图，重复请求得到原受理记录。
- [ ] COMMIT确认丢失可使用稳定键/受权查询恢复，不重复生成渠道号。

#### Risks

缺失原收银台后端，静态mock订单被当权威来源上线。

#### Dependencies

前置执行阶段：Phase 5、Phase 12、Phase 13。

#### Notes

测试fake只能验证合同；真实来源接入及凭据是本阶段终态输入。

证据：拟保存为 docs/pay/validation/phase-14.md；当前尚不存在，不代表已验证。

---

### Phase 15: 实现微信 HTTP 与密码协议适配核心

#### Status

NOT_STARTED

#### Goal

形成可验证的微信签名、响应/回调验证及账户版本选择基础。

#### Scope

HTTP预算、请求签名、证书/公钥模式、APIv3解密、序列号、轮换及安全错误。

#### Out of Scope

不宣称JSAPI/H5/支付分或退款已经完成，不调用未经授权的真实渠道。

#### Inputs

旧WechatClient与证书实现；Phase5秘密/配置版本；已选直连/服务商模式。

输入门禁：Q09渠道模式；Q16真实凭据仅在Phase43按授权使用。

#### Execution Requirements

- 领域只消费渠道trait；保留签名原文、换行和URL编码合同，限制响应和重定向。
- 使用受控测试密钥与脱敏样本验证证书/公钥轮换及解密，不复制生产凭据。

#### Expected Outputs

- 微信协议client与回调验证适配器。
- 黄金签名/验签/解密、错误码、超时和轮换测试向量。

#### Acceptance Criteria

- [ ] 合法样本通过，错body/签名/序列号/账户及超限报文拒绝。
- [ ] 配置版本可解析原账户并采用合法轮换材料。
- [ ] 测试明确渠道模式和支持范围，不以client存在声称场景已可用。

#### Risks

不同服务商/公钥模式、字节编码或证书轮换差异。

#### Dependencies

前置执行阶段：Phase 2、Phase 5。

#### Notes

后续Phase18/19/22/23/27分别接入场景、查关单、回调和退款。

证据：拟保存为 docs/pay/validation/phase-15.md；当前尚不存在，不代表已验证。

---

### Phase 16: 实现支付宝 HTTP 与密码协议适配核心

#### Status

NOT_STARTED

#### Goal

形成支付宝RSA2/证书、请求响应验证和账户版本基础。

#### Scope

请求编码、签名/验签、证书序列号、HTTP预算、错误分类与合法轮换。

#### Out of Scope

不将协议client视为WAP/LITE/免押或退款完成，不执行真实资金请求。

#### Inputs

旧AlipayClient、渠道签名样本；Phase5配置版本；Phase1渠道模式。

输入门禁：Q09渠道模式和实际接口版本。

#### Execution Requirements

- 使用受控样本验证表单/JSON与签名原文，域名/redirect/响应容量受控。
- 保留渠道原始错误分类和未知结果，不把HTTP超时判为未受理。

#### Expected Outputs

- 支付宝协议client、验证器及黄金向量。
- 合法/错签名/错账户、金额单位、编码、证书轮换和超限测试。

#### Acceptance Criteria

- [ ] 黄金样本字节级签名串和验签结果一致。
- [ ] 秘密与完整敏感报文不进日志；旧版本账户可正确解析。
- [ ] 支持的直连/服务商模式明确，未支持模式拒绝。

#### Risks

证书模式、表单编码和元转分规则差异。

#### Dependencies

前置执行阶段：Phase 2、Phase 5。

#### Notes

场景、退款和免押分别交付；复用client不等于业务接口支持。

证据：拟保存为 docs/pay/validation/phase-16.md；当前尚不存在，不代表已验证。

---

### Phase 17: 实现支付尝试与持久渠道调用意图

#### Status

NOT_STARTED

#### Goal

在渠道调用前保存稳定尝试和执行依据，崩溃后可恢复。

#### Scope

意图/尝试状态机、实际商户/账户/路由版本、稳定渠道号、准入见证与条件写回。

#### Out of Scope

本阶段用fake渠道，不开放真实下单；不以进程锁作为多副本资金防线。

#### Inputs

Phase7/13账户使用门禁、Phase14意图合同、S1支付不变量。

输入门禁：Q04撤销竞态；Q07唯一键。

#### Execution Requirements

- 持久命令提交后释放业务事务再调外部；结果使用版本条件写回。
- 每attempt保存实际收款商户，换商户重试需明确政策且收口旧尝试；未知状态不生成新请求号。

#### Expected Outputs

- 支付用例、attempt/command schema及渠道port/fake。
- 多副本领取/重复提交、提交丢确认、发前/发后崩溃和迟到结果测试。

#### Acceptance Criteria

- [ ] 一意图多尝试但有效收款裁决唯一；实际渠道请求号稳定可查。
- [ ] tenant/org与原意图一致，直接传accountId也经过统一授权。
- [ ] 外部HTTP期间不持有长业务锁；过期见证和本地规则变更安全拒绝新受理。

#### Risks

租约被当成撤销外部请求、同意图切商户覆盖旧账户。

#### Dependencies

前置执行阶段：Phase 7、Phase 13、Phase 14。

#### Notes

在回调、退款及重复收款尚未终验前，场景接线仅允许隔离测试。

证据：拟保存为 docs/pay/validation/phase-17.md；当前尚不存在，不代表已验证。

---

### Phase 18: 接入微信 JSAPI 下单场景

#### Status

NOT_STARTED

#### Goal

WECHAT_JSAPI生成正确渠道请求及客户端调起参数。

#### Scope

openid/应用绑定、直连或服务商参数、预下单响应、调起签名和待确认语义。

#### Out of Scope

不实现H5、小程序支付宝或支付分；不以bridge成功作为到账事实。

#### Inputs

Phase15微信核心、Phase17尝试、冻结JSAPI场景合同。

输入门禁：Q09是否上线JSAPI；未纳入需Phase1明确范围而非假通过。

#### Execution Requirements

- openid必须属于会话选定渠道应用，金额/商户/组织由意图与attempt决定。
- 回传必要调起字段，不回传秘密；未知预下单保存原请求身份。

#### Expected Outputs

- JSAPI适配器、DTO和受控HTTP合同样本。
- 模式参数、错openid应用、签名、成功/失败/未知响应测试。

#### Acceptance Criteria

- [ ] 选定模式请求及调起参数符合黄金合同，错误与超时保留待确认。
- [ ] 篡改金额、商户或付款人应用绑定被拒绝。
- [ ] 未完成真实渠道门禁前只登记实现候选，不标现网可用。

#### Risks

直连与服务商sub字段、openid所属应用不匹配。

#### Dependencies

前置执行阶段：Phase 15、Phase 17。

#### Notes

付款人OAuth在Phase31，浏览器真实联调在Phase43。

证据：拟保存为 docs/pay/validation/phase-18.md；当前尚不存在，不代表已验证。

---

### Phase 19: 接入微信 H5 下单场景

#### Status

NOT_STARTED

#### Goal

WECHAT_H5生成受控H5调起地址并保持服务端结果权威。

#### Scope

H5场景参数、可信客户端IP、URL与返回地址约束、渠道待确认响应。

#### Out of Scope

不扩大任意targetUrl、不信任浏览器payerClientIp、不实现JSAPI桥接。

#### Inputs

Phase15/17；Phase11测试HTTPS与信任代理；H5已冻结合同。

输入门禁：Q09/Q16 H5模式、域名与开通。

#### Execution Requirements

- 从真实peer/可信代理链取IP，精确校验渠道URL及返回origin/path。
- HTTP重试不创建新资金事件，h5_url仅用于该会话的允许场景。

#### Expected Outputs

- H5适配与返回地址构造。
- 代理IP伪造、redirect编码、成功/失败/未知和URL越界测试。

#### Acceptance Criteria

- [ ] 调起地址符合选定H5合同，禁止任意外部跳转。
- [ ] 金额、归属与attempt一致，超时恢复沿原渠道号。
- [ ] H5支付域名与获准环境门禁有清单，未开通不启用。

#### Risks

代理误信、双重URL编码、渠道H5域名未配置。

#### Dependencies

前置执行阶段：Phase 11、Phase 15、Phase 17。

#### Notes

WAP页面接入在Phase32；获准渠道验证在Phase43。

证据：拟保存为 docs/pay/validation/phase-19.md；当前尚不存在，不代表已验证。

---

### Phase 20: 接入支付宝 WAP 下单场景

#### Status

NOT_STARTED

#### Goal

ALIPAY_WAP输出符合合同的表单或调起数据。

#### Scope

WAP请求、金额与订单号、服务商参数、受控表单/返回地址和状态分类。

#### Out of Scope

不执行任意HTML脚本、不将return_url当成功凭证、不兼做LITE。

#### Inputs

Phase16支付宝核心、Phase17尝试、Phase11公开域名合同。

输入门禁：Q09/Q16 WAP开通与回调地址。

#### Execution Requirements

- 只生成/接受批准渠道目标的表单内容或受控字段，避免转发任意第三方HTML。
- 异步通知、同步返回和主动查单的信任等级分开。

#### Expected Outputs

- 支付宝WAP适配器与返回DTO。
- 表单目标、签名/金额、返回URL、错误和未知结果测试。

#### Acceptance Criteria

- [ ] 输出最少调起数据且不包含私钥；请求绑定原意图。
- [ ] 修改结果页status不会改变支付状态。
- [ ] 直连/服务商模式和WAP白名单有显式支持清单。

#### Risks

沿用旧前端innerHTML导致不受控内容执行、金额单位错位。

#### Dependencies

前置执行阶段：Phase 11、Phase 16、Phase 17。

#### Notes

WAP真实浏览器调起与结果回查由Phase32/43验证。

证据：拟保存为 docs/pay/validation/phase-20.md；当前尚不存在，不代表已验证。

---

### Phase 21: 接入支付宝小程序 LITE 下单场景

#### Status

NOT_STARTED

#### Goal

ALIPAY_LITE生成小程序可使用的稳定trade_no。

#### Scope

小程序应用/付款人绑定、预下单、服务商参数和必要返回字段。

#### Out of Scope

不把my.tradePay 9000写为后端成功，不兼做原生小程序UI。

#### Inputs

Phase16/17、冻结LITE应用与付款人标识合同。

输入门禁：Q09/Q16小程序应用、模板及渠道模式。

#### Execution Requirements

- 付款人标识与渠道应用匹配，商户/金额来自原意图。
- 持久化渠道请求号与trade_no，未知响应先查明原请求。

#### Expected Outputs

- LITE适配器、DTO及渠道合同样本。
- 错app/付款人、金额/商户、重复请求与超时测试。

#### Acceptance Criteria

- [ ] 合法请求取得正确结构trade_no，客户端只获得必要数据。
- [ ] 同键恢复不新建第二笔渠道交易。
- [ ] 未开通小程序/服务商能力显式拒绝，不按枚举开放。

#### Risks

小程序appId和支付渠道应用配置混用。

#### Dependencies

前置执行阶段：Phase 16、Phase 17。

#### Notes

小程序授权/页面由Phase31/33完成。

证据：拟保存为 docs/pay/validation/phase-21.md；当前尚不存在，不代表已验证。

---

### Phase 22: 实现权威查单、严格关单与查补用例

#### Status

NOT_STARTED

#### Goal

渠道状态可收敛，旧单不明时阻止新尝试。

#### Scope

微信/支付宝普通查单关单合同、关前查/关后查、待确认查询和可调度查补用例。

#### Out of Scope

不在此阶段完成worker调度；不将关闭请求成功等同确认旧单不可支付。

#### Inputs

两个协议核心、四场景attempt合同；旧PayOrderService严格关单规则。

输入门禁：Q13历史原账户证据在Phase37处理。

#### Execution Requirements

- 使用实际attempt原账户/配置，不按最新绑定重选。
- 成功事实持久化后独立表达禁止重付错误，不能因业务异常回滚真实收款。

#### Expected Outputs

- query/close端口实现和查补用例、状态映射。
- 已支付/处理中/关闭未知、关后仍成功、改绑定后旧单查询测试。

#### Acceptance Criteria

- [ ] 未知旧单阻止新的渠道尝试；只有明确合同允许才重新发起。
- [ ] 查单返回核对商户、交易号、金额和币种，错目标拒绝。
- [ ] 回放同一权威事实幂等，终态不会被迟到低可信状态覆盖。

#### Risks

查关单使用当前绑定、错误回滚已发生事实。

#### Dependencies

前置执行阶段：Phase 15、Phase 16、Phase 17、Phase 18、Phase 19、Phase 20、Phase 21。

#### Notes

Phase24把查补用例接入有界持久任务，资金不确定性不得靠固定sleep解决。

证据：拟保存为 docs/pay/validation/phase-22.md；当前尚不存在，不代表已验证。

---

### Phase 23: 实现渠道回调可靠收件与状态推进

#### Status

NOT_STARTED

#### Goal

验签回调可靠落库，重复乱序和崩溃不会丢资金事实。

#### Scope

原始报文限制、验签解密、交易核验、inbox去重、ACK合同、条件状态推进。

#### Out of Scope

不在未验签情况下信任URL归属，不将平台用户鉴权套在渠道回调。

#### Inputs

渠道协议验证器、Phase17/22状态机；批准的回调样本。

输入门禁：Q16回调来源与模式；Q11新旧分发不在此阶段暗含完成。

#### Execution Requirements

- ACK前完成合同要求的验证及持久收件，处理可从inbox恢复。
- 核对账户/app/交易号/金额/币种；未知交易隔离告警，不新建有权交易。

#### Expected Outputs

- 回调路由、inbox schema与处理器。
- 错签名/目标/金额、重复乱序、ACK前后崩溃、平台授权故障测试。

#### Acceptance Criteria

- [ ] 成功不会被迟到pending/failed覆盖；迟到真实收款进入明确纠偏流程。
- [ ] 重复报文不重复产生业务效果，已ACK数据可恢复。
- [ ] 回调在管理授权服务失联时仍通过自己的验证链处理存量事实。

#### Risks

报文被Nginx/JSON重编码、ACK先于可靠落库。

#### Dependencies

前置执行阶段：Phase 15、Phase 16、Phase 17、Phase 22。

#### Notes

跨新旧引擎的加密回调兼容分发独立在Phase39实现，所需地址/渠道限制输入为Q21，不与Q11退款政策混用。

证据：拟保存为 docs/pay/validation/phase-23.md；当前尚不存在，不代表已验证。

---

### Phase 24: 实现事务 outbox 与可恢复工作进程

#### Status

NOT_STARTED

#### Goal

支付变更事件和查询任务可在多副本、崩溃后可靠执行。

#### Scope

状态/outbox同事务、任务幂等键、原子claim、lease/fencing、查补调度、背压和优雅退出。

#### Out of Scope

不承诺跨渠道exactly-once，不把租约当重复资金调用的唯一防线。

#### Inputs

Phase22查补、Phase23状态处理、Phase2库和并发预算。

输入门禁：Q17容量/超时；强撤销另由Q04定义。

#### Execution Requirements

- 外部HTTP在领取事务结束后执行，写回核对owner/version。
- 过期任务可接管，旧worker迟到不能覆盖新状态；已发请求仍靠稳定渠道号与查询恢复。

#### Expected Outputs

- outbox/jobs schema、独立或同包worker入口和调度器。
- 竞争领取、崩溃回收、旧worker迟到、容量耗尽和停机恢复证据。

#### Acceptance Criteria

- [ ] 业务提交必有对应事件意图，事务回滚不留下虚假事件。
- [ ] 同任务至少一次执行可恢复但不制造第二笔资金事件。
- [ ] 查补持续收敛待确认普通支付，超限/长期异常有可核查处置状态。

#### Risks

本地内存队列丢失、在锁内做HTTP、租约过期重发新交易号。

#### Dependencies

前置执行阶段：Phase 22、Phase 23。

#### Notes

不保存用户token；机器执行权限来自已受理持久意图和限制，不冒充管理员。

证据：拟保存为 docs/pay/validation/phase-24.md；当前尚不存在，不代表已验证。

---

### Phase 25: 实现签名商户通知与受控重放

#### Status

NOT_STARTED

#### Goal

成功支付/退款事件可向固定业务目标可靠投递并幂等恢复。

#### Scope

固定目标/事件、签名版本、ACK规则、退避/最大次数、lease、查询与人工重放审计。

#### Out of Scope

不以平台Toast代替商户通知，不保存用户token用于后台站内发布。

#### Inputs

Phase12业务客户端、Phase24 outbox/worker、Phase1通知合同。

输入门禁：Q12通知兼容/ACK；Q19留存。

#### Execution Requirements

- 目标从服务端登记取得，禁止随意redirect与未批准网络地址；重试保留原事件内容。
- 接收端按稳定事件ID去重，人工重放不得换资金意图；退款事件可先以fake合同验证。

#### Expected Outputs

- 商户通知投递器、通知状态API和受控接收方fixture。
- 响应丢失、重复投递、目标变更、重定向/内部地址拒绝和崩溃恢复测试。

#### Acceptance Criteria

- [ ] 本地交易成功即使通知失败也保留持久事件，重试可查。
- [ ] 同事件签名合同一致，目标/载荷不能在重试中被无审计改写。
- [ ] 通知成功、待重试、需人工区分，HTTP网络错误不解释为业务未支付。

#### Risks

SSRF、签名正文漂移、把2xx或UIToast混作交易事实。

#### Dependencies

前置执行阶段：Phase 12、Phase 24。

#### Notes

后台人员提示若列为必需，另补有界机器通知合同；不自动扩平台通知权限。

证据：拟保存为 docs/pay/validation/phase-25.md；当前尚不存在，不代表已验证。

---

### Phase 26: 实现人工退款申请与复核状态机

#### Status

NOT_STARTED

#### Goal

退款申请和复核有独立动作、数据范围、版本及不可篡改目标。

#### Scope

申请/读取/approve/reject、禁止自审、金额/原支付绑定、批准有效期和业务审计。

#### Out of Scope

本阶段不发送真实退款，不把审批完成等同渠道退款成功。

#### Inputs

Phase1退款政策；Phase3权限；Phase17原支付尝试；Q11已冻结。

输入门禁：Q11人工/机器退款、限额、复核与有效期。

#### Execution Requirements

- 按原交易来源组织和当前权限/grant/业务范围核验；复核人执行时重新鉴权。
- 申请阶段不承诺占款，批准目标不可换金额/换原单；执行接线在Phase28。

#### Expected Outputs

- 退款请求schema、申请复核API、审批审计和状态合同。
- 自审、越组织、无grant、版本冲突、批准过期和篡改请求测试。

#### Acceptance Criteria

- [ ] 申请者不能批准自身请求，角色名字不绕过独立approve权限。
- [ ] 同键重复申请幂等，已批准目标变化返回冲突。
- [ ] 尚未接入额度/执行前，公开API明确只完成申请记录，不放行资金动作。

#### Risks

将管理权限当双人审批、批准后正文仍可修改。

#### Dependencies

前置执行阶段：Phase 3、Phase 5、Phase 17。

#### Notes

机器退款按冻结的直接/复核政策单独映射，不默认绕过审批。

证据：拟保存为 docs/pay/validation/phase-26.md；当前尚不存在，不代表已验证。

---

### Phase 27: 实现微信与支付宝渠道退款合同

#### Status

NOT_STARTED

#### Goal

原支付账户下的退款提交、查询和退款通知具备可验证渠道合同。

#### Scope

退款请求号/金额、原交易引用、响应/查单状态、退款回调验证及单位转换。

#### Out of Scope

不在此阶段开放公开退款执行或承担累计额度预占。

#### Inputs

两个协议client、原账户/查单引用、渠道退款黄金样本。

输入门禁：Q09实际退款能力和限制。

#### Execution Requirements

- 固定原支付attempt与账户，不能根据当前商户绑定改退款目标。
- 未知结果保留稳定退款号，明确成功/处理中/失败/关闭的渠道含义。

#### Expected Outputs

- 两渠道退款/退款查询适配器、通知解析和统一状态映射。
- 金额/币种、错账户、重复号、超时及回调向量测试。

#### Acceptance Criteria

- [ ] 黄金样本能区分已成功、处理中及不可继续的确定终态。
- [ ] 不会因返回超时生成新退款号，响应校验对应原支付。
- [ ] 未支持的退款能力显式拒绝，不按接口枚举假定可用。

#### Risks

退款元分转换、渠道关闭含义误解、原路账户错选。

#### Dependencies

前置执行阶段：Phase 15、Phase 16、Phase 22。

#### Notes

Phase28将合同接入本地额度预占与持久执行，Phase43做获准渠道验证。

证据：拟保存为 docs/pay/validation/phase-27.md；当前尚不存在，不代表已验证。

---

### Phase 28: 实现退款额度预占与持久执行

#### Status

NOT_STARTED

#### Goal

不同退款号并发也不能突破原支付可退余额，未知结果可恢复。

#### Scope

原支付维度预占、批准与执行意图事务、稳定退款号、worker执行、回调/查补与通知。

#### Out of Scope

不在超时或结果未知时释放额度，不用进程锁保证多副本安全。

#### Inputs

Phase26批准合同、Phase27渠道退款、Phase24/25任务与通知、成功支付事实。

输入门禁：Q11政策；Q04新受理撤销与存量执行边界。

#### Execution Requirements

- 成功退款与未解除处理中/未知预占共同扣减可退金额。
- 批准生成执行意图时原支付锁内复核额度和版本，提交后才调渠道；确定不再执行的失败才释放。

#### Expected Outputs

- 退款执行schema/用例、预占不变量、查询和恢复接口。
- 不同退款号并发、重复approve、提交/渠道响应丢失、重复回调和撤USE后存量恢复证据。

#### Acceptance Criteria

- [ ] 成功退款总额加有效预占不超过原收款金额。
- [ ] 审批通过但执行未知不展示已退回，稳定退款号贯穿所有恢复。
- [ ] 原账户USE撤销不丢失已受理义务，新的人工操作仍需当前权限与订单范围。

#### Risks

未知退款释放额度、worker绕过批准范围、跨库撤权被当原子保证。

#### Dependencies

前置执行阶段：Phase 13、Phase 24、Phase 25、Phase 26、Phase 27。

#### Notes

直接机器退款也必须通过已批准策略和同一金额防线。

证据：拟保存为 docs/pay/validation/phase-28.md；当前尚不存在，不代表已验证。

---

### Phase 29: 实现重复成功收款裁决与自动退款义务

#### Status

NOT_STARTED

#### Goal

同业务订单多笔真实成功收款只确认一笔有效收款，其余有唯一可恢复退款义务。

#### Scope

意图级有效尝试裁决、关系纠偏、重复收款固定幂等键、自动原路退款及人工差异状态。

#### Out of Scope

不丢弃真实收款、不向业务系统发送第二笔有效成功、不机械套用免押扣费。

#### Inputs

旧DuplicatePaymentGuard规则；Phase17意图唯一性、Phase23成功事实、Phase28退款。

输入门禁：Q07唯一身份；免押重复扣费另列扩展E07/E08。

#### Execution Requirements

- 在意图锁或等价CAS下裁决；保留各笔实际商户/账户，退款只针对重复成功attempt。
- 自动退款意图先持久提交，再执行；失败或CLOSED保留未清偿义务和告警，不自动换退款号。

#### Expected Outputs

- 重复收款裁决用例、系统退款批准依据及差异记录。
- 旧单先付/新单先付、并发成功、退款关闭/未知、重放和通知唯一性测试。

#### Acceptance Criteria

- [ ] 一意图最多一个有效收款引用，双成功均可追溯。
- [ ] 相同重复attempt只有一个自动退款意图，崩溃后沿原号恢复。
- [ ] 商户只收到有效支付成功事件，未退资金不会被误标已退。

#### Risks

有效引用被最后写覆盖、自动退款另造旁路绕过累计额度。

#### Dependencies

前置执行阶段：Phase 17、Phase 23、Phase 25、Phase 28。

#### Notes

首期资金闭环完成前必须通过本阶段，不能隐藏在回调分支里省略验收。

证据：拟保存为 docs/pay/validation/phase-29.md；当前尚不存在，不代表已验证。

---

### Phase 30: 实现收银台支付会话与旧接口兼容层

#### Status

NOT_STARTED

#### Goal

补齐QuickPay缺失服务，以受限会话读取/支付/查询权威订单。

#### Scope

付款能力摘要、期限/撤销/重开、订单展示、场景列表、下单及结果；五类旧路径适配中的非OAuth部分。

#### Out of Scope

不允许只凭tenant/org/orderId或Base64查询结果，不任意抓取url，不新增前端可控金额。

#### Inputs

Phase14订单provider、资金闭环、Phase11公开入口、QuickPay现有paths/types。

输入门禁：Q06缺失服务与链接合同；Q16公开入口。

#### Execution Requirements

- 链接s先核原服务再迁移；新能力高熵有界、最小展示、no-store/no-referrer，原始能力不进日志。
- 会话绑定意图和允许场景，重开页面仍是同一逻辑订单；OAuth接口由Phase31完成后才可启用。

#### Expected Outputs

- cashier session API、旧路径兼容合同、受权结果查询。
- 篡改归属/金额、过期/撤销、链接重开、订单变价/取消和未知恢复测试。

#### Acceptance Criteria

- [ ] 五类旧接口各有明确实现或等待Phase31的禁用状态，不指向不存在服务。
- [ ] 前端自报tenant/org仅作一致性检查，不能决定访问范围。
- [ ] 结果状态来自服务端资金事实，payer标识不等于SaaS用户。

#### Risks

旧s生成服务无法找到、URL能力被日志/第三方资源泄露。

#### Dependencies

前置执行阶段：Phase 11、Phase 14、Phase 17、Phase 22、Phase 23、Phase 28、Phase 29。

#### Notes

本阶段单独不标QuickPay可用；需OAuth及两个客户端、真实场景门禁完成。

证据：拟保存为 docs/pay/validation/phase-30.md；当前尚不存在，不代表已验证。

---

### Phase 31: 实现付款人 OAuth 与一次性状态流

#### Status

NOT_STARTED

#### Goal

收银台通过正确渠道应用取得付款人身份，state可跨副本安全消费。

#### Scope

微信/支付宝付款授权、state与session/app/scene/返回地址绑定、code交换、并发与过期恢复。

#### Out of Scope

不把渠道OAuth用作管理登录，不将openid或code作为租户凭证。

#### Inputs

Phase30会话、渠道client、Phase11精确HTTPS回调；Q09/Q16。

输入门禁：Q09渠道授权模式；Q16精确回调和应用。

#### Execution Requirements

- state持久保存且原子一次消费，回调地址来自允许清单，拒绝任意targetUrl。
- 服务端关联code与正确应用，交换未知不盲目重复消费；秘密不进浏览器。

#### Expected Outputs

- 付款OAuth端点及旧redirect/authorize兼容接线。
- state重放、错应用/会话、过期、开放跳转、并发回调与响应丢失测试。

#### Acceptance Criteria

- [ ] 渠道付款身份仅绑定原会话，篡改tenant/org不改变归属。
- [ ] 微信H5返回、JSAPI授权及支付宝小程序身份合同各自明确。
- [ ] 五类QuickPay API全部有实现和错误合同，缺渠道开通仍保持该场景禁用。

#### Risks

将Base64 state当签名凭证、一次性code自动重试、OAuth域名缺失。

#### Dependencies

前置执行阶段：Phase 15、Phase 16、Phase 30。

#### Notes

服务端可用不等于客户端联调通过，分别由Phase32/33/43验收。

证据：拟保存为 docs/pay/validation/phase-31.md；当前尚不存在，不代表已验证。

---

### Phase 32: 接入 QuickPay WAP 与服务端结果恢复

#### Status

NOT_STARTED

#### Goal

公开WAP可按环境选择获准场景，并以服务端状态完成结果展示。

#### Scope

接口/域名/base/路由、授权回跳、微信bridge/H5、支付宝WAP、待确认与有界轮询/恢复。

#### Out of Scope

不改写为Rust前端，不将非SUCCESS一律显示失败，不相信URL status。

#### Inputs

旧quick-pay-wap；Phase30/31合同；Phase11部署；选定场景子集。

输入门禁：Q09上线场景；Q16公开origin和平台域名白名单。

#### Execution Requirements

- 以受限会话读取订单与结果；资金写入保留原事件键，不因重进页面新建意图。
- 限制表单目标和第三方内容，状态机区分处理中/待确认/关闭/明确失败。

#### Expected Outputs

- WAP迁入或受管工程、环境公开配置、结果恢复UI。
- 所选浏览器与渠道桥接fixture、OAuth返回、刷新/断网/迟到结果测试。

#### Acceptance Criteria

- [ ] 客户端不能修改金额、商户或组织，服务端结果决定成功。
- [ ] 付款未确认时可安全核对同事件，不自动再次扣款。
- [ ] 产物不含密钥或管理token，合法渠道调起参数完整。

#### Risks

旧内存缓存和绝对/result路由、HTML注入、bridge回调被当收款事实。

#### Dependencies

前置执行阶段：Phase 18、Phase 19、Phase 20、Phase 30、Phase 31。

#### Notes

真实微信内/普通浏览器/支付宝渠道条件在Phase43另验收。

证据：拟保存为 docs/pay/validation/phase-32.md；当前尚不存在，不代表已验证。

---

### Phase 33: 接入支付宝原生小程序与结果回查

#### Status

NOT_STARTED

#### Goal

小程序在受控会话下授权、调起支付并查询服务端结果。

#### Scope

环境API地址、authCode、LITE下单、my.tradePay、服务端结果页与重新进入恢复。

#### Out of Scope

不将my.tradePay 9000或URL参数作为后端成功，不实际提交审核/发布。

#### Inputs

旧quick-pay-alipay及发布工具边界；Phase21/30/31合同；Q16小程序环境。

输入门禁：Q16小程序app/模板、测试设备与授权范围。

#### Execution Requirements

- 小程序应用和付款人绑定由服务端确认；页面保留原会话/操作引用。
- 构建配置只含公开信息，发布私钥通过发布环境管理。

#### Expected Outputs

- 原生客户端适配、环境配置与结果状态UI。
- 开发者工具/获准客户端流程、超时/回跳/假status与服务端回查证据。

#### Acceptance Criteria

- [ ] 支付结果以服务端为准，未知状态有显式查询入口。
- [ ] 硬编码生产域名被受控环境配置替换，不泄露发布私钥。
- [ ] 未获准发布不执行上线操作，测试与实际发布结果分别记录。

#### Risks

原发布脚本只操作已有模板版本，不能假定包含本地代码上传。

#### Dependencies

前置执行阶段：Phase 21、Phase 30、Phase 31。

#### Notes

保持TypeScript原生客户端，Node仅用于构建发布工具。

证据：拟保存为 docs/pay/validation/phase-33.md；当前尚不存在，不代表已验证。

---

### Phase 34: 实现交易管理查询接口与页面

#### Status

NOT_STARTED

#### Goal

管理用户在当前授权组织范围查看交易并安全执行关单/导出。

#### Scope

范围化管理交易list/count/detail/export及管理close HTTP接线；订单页面、意图/尝试/有效收款展示和恢复。

#### Out of Scope

不因共用账户显示其他组织交易，不引入任意金额手工收款。

#### Inputs

Phase8微应用及登记授权清单，Phase14/17意图/尝试模型、Phase22查关单用例、Phase29裁决；本阶段负责补齐管理HTTP/SQL，不假定它们已由前置阶段交付。

输入门禁：Q18跨租户运营不在首版默认开放。

#### Execution Requirements

- 每页面动作对应独立permission/grant；列表与汇总使用后端同范围。
- 关单未知先受权核对原attempt，不自动重发创建或提示已关闭。
- 本阶段交付后端范围化列表/count/详情/导出SQL和平台鉴权close路由，再接前端；读/导出/关单各用自身动作与grant，不依赖资金开放API。

#### Expected Outputs

- 管理交易范围SQL、list/count/detail/export/close HTTP接口及页面/API客户端。
- 普通总部/分支/门店、兄弟组织隔离、409/未知关单、导出权限和容量测试。

#### Acceptance Criteria

- [ ] 账户owner不自动拥有全部订单读权，数据按原订单组织和业务范围过滤。
- [ ] 有效收款、重复收款和待退款义务不会混为一笔。
- [ ] 切组织后旧响应不回填，导出不能绕过同一SQL范围。
- [ ] 后台列表、count、详情、导出和close直调均有跨租户/组织拒绝证据；导出容量/快照与SDK下载合同符合冻结要求，不存在仅前端过滤。

#### Risks

旧全局列表或导出旁路保留、账户共享等同订单共享。

#### Dependencies

前置执行阶段：Phase 8、Phase 17、Phase 22、Phase 25、Phase 29。

#### Notes

缺必要菜单时平台正确拒绝，不通过前端隐藏绕开。

证据：拟保存为 docs/pay/validation/phase-34.md；当前尚不存在，不代表已验证。

---

### Phase 35: 实现退款申请、复核及执行结果页面

#### Status

NOT_STARTED

#### Goal

用户可按职责申请/复核退款，并安全核对预占与执行结果。

#### Scope

申请表单、approve/reject、禁止自审、版本/幂等恢复、执行中/未知/成功/差异展示。

#### Out of Scope

不把批准成功等同资金退回，不允许前端修改批准后的目标。

#### Inputs

Phase26/28/29 API；当前应用权限和订单范围。

输入门禁：Q11最终退款政策应已冻结。

#### Execution Requirements

- 基于原成功attempt展示金额和可退额度，不在浏览器浮点累计。
- 网络/5xx未知保留原事件，受权查询后恢复；授权撤销清理旧服务器视图。

#### Expected Outputs

- 退款工作流页面、API适配、角色矩阵。
- 申请者/复核者、自审、并发额度不足、过期批准、未知恢复和组织切换证据。

#### Acceptance Criteria

- [ ] 无approve权限或订单范围的用户无法复核，申请人无法自批。
- [ ] 处理中/未知预占与已成功退款金额展示一致。
- [ ] 重复点击和刷新不产生新资金意图，系统自动退款与人工申请可追溯。

#### Risks

旧表单默认全量权限、客户端余额估算、提交超时后换键。

#### Dependencies

前置执行阶段：Phase 8、Phase 26、Phase 28、Phase 29、Phase 34。

#### Notes

人员候选需登记memberLookups且不授予被选人员额外平台权限。

证据：拟保存为 docs/pay/validation/phase-35.md；当前尚不存在，不代表已验证。

---

### Phase 36: 实现通知、补偿与支付审计运维页面

#### Status

NOT_STARTED

#### Goal

有权人员可查询任务和安全审计，并受控恢复失败通知。

#### Scope

任务/通知列表详情、事件关联、受控retry、支付业务审计查询与差异提示。

#### Out of Scope

不暴露原始敏感回调/密钥，不将通知重放变成重新支付或退款。

#### Inputs

Phase24/25任务API、Phase28退款状态、pay-notification/pay-audit权限。

输入门禁：Q19审计/报文保留期与查看权限。

#### Execution Requirements

- 查询按自身resource/action范围，不借订单读权限取得全部审计。
- 人工重放固定事件和目标，重复操作幂等并写入业务审计。

#### Expected Outputs

- 支付运维页面和最小管理接口。
- 越组织、无retry权限、重复重放、已投递响应丢失及脱敏测试。

#### Acceptance Criteria

- [ ] 任务lease/失败/待人工状态可核对，列表不泄露其他租户数据。
- [ ] 通知重放不新增资金动作，也不任意更换正文或目标。
- [ ] 系统worker/商户client/渠道/管理用户actor明确区分。

#### Risks

用平台audit表直接存全部业务报文，运维入口绕过资金权限。

#### Dependencies

前置执行阶段：Phase 8、Phase 24、Phase 25、Phase 28。

#### Notes

平台审计查询和支付业务审计是不同所有权，不直写平台通知/审计表。

证据：拟保存为 docs/pay/validation/phase-36.md；当前尚不存在，不代表已验证。

---

### Phase 37: 盘点旧数据、身份映射与原账户证据

#### Status

NOT_STARTED

#### Goal

为每类存量记录确定租户/组织/新ID/原渠道及唯一处理引擎。

#### Scope

只读源schema/导出、旧用户候选、订单组织来源、原账户版本证据、余额/免押/在途义务清单。

#### Out of Scope

不向生产库导入，不按merchantNo给所有关联组织复制历史单，不自动合并同邮箱用户。

#### Inputs

旧库隔离副本或批准导出、源仓库、Phase1合同；Q01/Q13/Q14/Q15。

输入门禁：Q01/Q13/Q14/Q15为本阶段必需外部输入。

#### Execution Requirements

- 记录证据等级与核对状态，无法确定归属进入隔离暂存；旧ID/渠道号/时区保留。
- 账户身份无法确认时不接管资金动作；旧引擎也须经核实可处理，不能用当前绑定伪造历史版本。

#### Expected Outputs

- 带批次/摘要的归属、用户、账户和实体映射文件规范与受控数据清单。
- 未迁移扩展能力/未结清义务/旧引擎责任登记。

#### Acceptance Criteria

- [ ] 源数据每个范围有确定owner或明确隔离/待核对记录，不带未知归属上线。
- [ ] 用户UUID映射与membership/角色/grant顺序明确；无法核对的历史actor保留来源快照。
- [ ] 成功/退款/余额冻结/免押/任务等义务均有金额或逐笔核对依据。

#### Risks

无现网schema证据、物理删除/updated_at遗漏、旧时间无时区。

#### Dependencies

前置执行阶段：Phase 1、Phase 5、Phase 6、Phase 17、Phase 28。

#### Notes

这是迁移盘点终态，不代表数据已经搬迁或账号已经实际开通。

证据：拟保存为 docs/pay/validation/phase-37.md；当前尚不存在，不代表已验证。

---

### Phase 38: 实现核心导入器与隔离迁移核对

#### Status

NOT_STARTED

#### Goal

源副本可重复导入新隔离库，金额与在途关系可核验。

#### Scope

账户/授权/意图/attempt/退款/通知任务映射导入、批次幂等、隔离错误及核对报表。

#### Out of Scope

不使用同构搬运工具替代新归属映射，不修改已执行Flyway/SQLx历史，不接管生产。

#### Inputs

Phase37已批准映射与导出、目标schema、成功/退款/在途合同。

输入门禁：Q13—Q15映射与留存；Q08实际数据库类型。

#### Execution Requirements

- 以稳定来源实体键导入，保持外部tradeNo/refundNo及原关系。
- 按tenant/org/merchant/date/currency/status核对数量金额；缺原账户/组织的记录不进入开放业务域。
- 导入保留原业务状态，另加migration_hold/等价执行禁用标记；全部导入jobs/outbox/通知/退款/查补默认不可claim。演练出口仅受控fixture，禁止旧notifyUrl或生产渠道；owner与允许动作核对后才能显式激活。

#### Expected Outputs

- 核心迁移工具、批次状态/校验报告、可恢复重跑流程。
- 空库/重复批次/中断/无归属/大ID/时区/精度与在途预占测试。

#### Acceptance Criteria

- [ ] 同批次重跑不重复资金或任务，原支付/退款单号保持。
- [ ] 核对使用原始导出及批准权威来源，差异有逐条解释或阻止切换。
- [ ] 导入后普通用户跨租户/组织查询仍拒绝，历史日志映射不赋予新权限。
- [ ] 启动worker也不能领取隔离批次任务；向旧生产notifyUrl投递、查单、退款等副作用均被任务门禁和受控egress拦截，解除隔离有明确批准及审计。

#### Risks

重复导入激活任务、把无证据数据强行归当前组织、忽略余额义务。

#### Dependencies

前置执行阶段：Phase 28、Phase 29、Phase 37。

#### Notes

导入默认隔离所有可执行副作用，不仅是渠道写。任务激活与唯一owner转交须Phase39/44核对，演练只连接受控fixture。

证据：拟保存为 docs/pay/validation/phase-38.md；当前尚不存在，不代表已验证。

---

### Phase 39: 实现旧新引擎 owner 路由与可信回调分发

#### Status

NOT_STARTED

#### Goal

查询、退款、回调和补偿均到达原单唯一权威引擎。

#### Scope

持久owner_engine、稳定业务/渠道标识映射、共享账户加密回调验证分发、可靠交付与unknown隔离。

#### Out of Scope

不广播同回调到两个引擎，不信任URL明文tenant/merchant直接选owner，不让新旧同时写同单。

#### Inputs

Phase37旧单责任、Phase38映射、两个引擎可验证协议与实际回调配置；Q21。

输入门禁：Q21回调固定地址/渠道限制、共享账户分发和旧引擎保留方式。

#### Execution Requirements

- 共享账户且交易号加密时，分发边界先必要验签解密再定位owner；保存可靠分发意图和原始报文验证证据。
- 下游复验或使用明确认证内部封装；切换前已发/切换后才确认的请求也有owner，账户未知不盲选。

#### Expected Outputs

- 兼容路由/收件分发实现和部署合同。
- 同账户新旧交易、加密报文、owner不明、响应丢失/重复、延迟回调与防双写测试。

#### Acceptance Criteria

- [ ] 每笔测试交易恰有一个写入引擎，查/关/退与回调使用相同owner。
- [ ] ACK前满足可靠收件，分发重试不制造第二笔资金事件。
- [ ] 无法可信分发时采用已验证专用入口/账户隔离，未演练不得标完成。

#### Risks

微信v3交易号在密文中、同账户不能定位引擎、回调地址变更影响旧单。

#### Dependencies

前置执行阶段：Phase 11、Phase 23、Phase 24、Phase 37、Phase 38。

#### Notes

资金真实归属与逻辑订单唯一键保持一致，路由迁移不变更外部单号。

证据：拟保存为 docs/pay/validation/phase-39.md；当前尚不存在，不代表已验证。

---

### Phase 40: 固化全链路 CI 与授权隔离回归

#### Status

NOT_STARTED

#### Goal

一期上线子集在同一源码基线上通过自动化协议、数据库和浏览器回归。

#### Scope

快速/数据库/多进程/浏览器分层门禁、受控渠道HTTPfixture、根账户子树及资金故障矩阵。

#### Out of Scope

不以fixture通过代替真实渠道，也不把已有平台历史测试当支付通过。

#### Inputs

前述实现与每阶段证据；冻结首期场景、平台合同和浏览器清单。

输入门禁：Q09上线子集、Q16测试环境必须可用。

#### Execution Requirements

- 固定依赖锁与源码摘要、隔离测试资源/账号；缺失必需用例不能用skip填完成。
- 强制检查根共享、未来新增、无SHARE、兄弟订单隔离、机器准入、并发退款及重复收款。

#### Expected Outputs

- 支付CI入口、用例清单及矩阵结果。
- Linux构建/协议/私有PG/三类客户端或批准浏览器的同基线证据。

#### Acceptance Criteria

- [ ] 必需自动化用例全部实际通过，失败/跳过/未执行明确区分。
- [ ] 故障注入包含提交丢确认、渠道已成功、重复回调、lease过期及授权变化。
- [ ] 秘密与高基数指标扫描、路由鉴权合同回归通过，现有平台/业务未被破坏。
- [ ] 恢复/导入隔离批次在worker运行时仍无外部副作用，关系证明换target或用途不能复用；payments正式登记清单可重复核对。

#### Risks

测试只镜像实现、跨进程测试未实际发现、测试owner资源污染。

#### Dependencies

前置执行阶段：Phase 10、Phase 11、Phase 18、Phase 19、Phase 20、Phase 21、Phase 22、Phase 23、Phase 24、Phase 25、Phase 26、Phase 27、Phase 28、Phase 29、Phase 30、Phase 31、Phase 32、Phase 33、Phase 34、Phase 35、Phase 36、Phase 38、Phase 39。

#### Notes

每阶段都需自身验证，本阶段只把完整链路固定为正式可重复门禁。

证据：拟保存为 docs/pay/validation/phase-40.md；当前尚不存在，不代表已验证。

---

### Phase 41: 演练备份恢复与兼容回退

#### Status

NOT_STARTED

#### Goal

从受控备份恢复支付库、密钥引用和在途任务，验证兼容构建回退。

#### Scope

备份/恢复链、映射/版本/owner保留、worker恢复、schema兼容和停止新受理后的回退演练。

#### Out of Scope

不删除迁移记录、不用快照覆盖已发生渠道资金、不执行生产故障。

#### Inputs

Phase38/39导入与路由、Phase40基线；Q17恢复目标、秘密恢复条件。

输入门禁：Q17 RPO/RTO、保留期、容灾与凭据恢复。

#### Execution Requirements

- 包含支付库、必要密钥恢复链、原账户版本、幂等/任务和owner映射。
- 模拟恢复期间迟到回调和未确认资金，明确旧单仍由原引擎处理。
- 恢复库启动前所有jobs/outbox/通知/查补/退款默认migration_hold/等价禁用，演练出口锁到fixture；核对owner和获准动作后分批显式激活，不能发往旧生产notifyUrl。

#### Expected Outputs

- 恢复/回退运行手册和隔离演练记录。
- RPO/RTO实测、恢复后金额/任务/版本核对及兼容镜像清单。

#### Acceptance Criteria

- [ ] 可从备份在新隔离库恢复并处理原单，秘密可用性已验证。
- [ ] 回退不重复资金请求、不丢原渠道号和预占。
- [ ] 实测满足冻结目标；不满足则保留未完成和具体修复项。
- [ ] 隔离恢复启动不会发任何生产通知/查询/退款，受控激活后只处理已批准owner范围；原业务状态和执行禁用状态分别保留。

#### Risks

备份缺解密密钥、回退二进制不理解新schema、把数据库回退当交易撤销。

#### Dependencies

前置执行阶段：Phase 38、Phase 39、Phase 40。

#### Notes

恢复目标由业务明确，不从平台其他阶段状态推定可用。

证据：拟保存为 docs/pay/validation/phase-41.md；当前尚不存在，不代表已验证。

---

### Phase 42: 验证容量预算与授权模式默认值

#### Status

NOT_STARTED

#### Goal

在真实同口径负载下选择支付默认授权模式并验证有界运行。

#### Scope

combined/legacy比较、账户关系核验成本、DB/HTTP/worker/队列预算、限流和故障退化。

#### Out of Scope

不为性能缓存正向授权、不关闭验签/租户过滤、不宣称未测生产容量。

#### Inputs

Phase40可重复环境；Q17期望负载、时延与阈值；平台最新预算。

输入门禁：Q17容量、预算和可接受失败/延迟标准。

#### Execution Requirements

- 固定相同源码、数据、并发和总耗时口径，记录实际资源与尾延迟。
- 授权不可达拒绝新管理/机器受理，回调及存量查补按独立信任链继续；满载背压不无限排队。

#### Expected Outputs

- 容量报告、默认模式决定、预算配置与告警阈值。
- 授权/关系并发满、数据库锁等待、积压和恢复压测证据。

#### Acceptance Criteria

- [ ] 冻结目标达到或明确未达到并修复，两个模式合同均不自动降级。
- [ ] 资源保持上限，错误不触发自动新资金事件。
- [ ] 模式选择有支付自身测量，不套用门店/工单结论。

#### Risks

压测省略关系核验、吞吐提升来自放宽安全语义。

#### Dependencies

前置执行阶段：Phase 40。

#### Notes

性能结论只适用记录环境与负载，容量扩大按同合同复核。

证据：拟保存为 docs/pay/validation/phase-42.md；当前尚不存在，不代表已验证。

---

### Phase 43: 完成获准渠道与真实客户端联合验收

#### Status

NOT_STARTED

#### Goal

冻结上线场景在获准渠道环境、真实客户端和业务订单来源下闭环。

#### Scope

实际商户/app/模式、微信内/H5/支付宝WAP/小程序、共享根账户真实结算身份、退款/通知/查补。

#### Out of Scope

未获明确授权不产生真实资金，不以开发工具mock或自动化fixture代替渠道验证。

#### Inputs

Q05/Q09/Q16测试条件、允许额度/操作清单；真实来源订单系统、精确域名与渠道白名单。

输入门禁：Q05结算、Q09场景、Q16测试范围与授权。

#### Execution Requirements

- 逐场景记录渠道环境、实际商户/子商户/appId及稳定交易号，凭据脱敏。
- 根与下级共享时核对实际资金入账主体及订单来源组织，验证撤USE后存量查补和原路退款。

#### Expected Outputs

- 获准操作范围和各场景联合结果、资金核对报告。
- 未开通/失败/未执行能力清单及上线场景开关清单。

#### Acceptance Criteria

- [ ] 所有冻结上线场景实际支付、权威查单/回调、退款、业务通知通过并核对。
- [ ] 兄弟组织交易隔离、下级不读秘密及指定结算主体得到证据。
- [ ] 外部条件不足的必需项仍未完成，不声称现网支持。

#### Risks

渠道无法提供全部故障模式、真实测试资金/设备条件不足。

#### Dependencies

前置执行阶段：Phase 40、Phase 41、Phase 42。

#### Notes

若要缩小上线子集，需有明确业务决定及台账变更，不用静默跳过改变范围。

证据：拟保存为 docs/pay/validation/phase-43.md；当前尚不存在，不代表已验证。

---

### Phase 44: 执行获准灰度、切换核对与回退演练

#### Status

NOT_STARTED

#### Goal

在明确获准域内切换新单，旧单保持唯一处理方并可按演练策略停止新受理。

#### Scope

灰度域/时间边界、在途登记、owner分发、观测窗口、金额差异核对与停止/回退条件。

#### Out of Scope

当前创建台账不授权部署；未来本阶段没有生产操作授权时只做隔离演练，不标生产切换完成。

#### Inputs

批准的发布域/操作范围、固定镜像、Phase37—43证据；Q17观察窗/阈值、Q20发布授权。

输入门禁：Q17观测标准、Q20隔离演练或生产操作批准范围。

#### Execution Requirements

- 先排空或登记切换边界在途请求；新旧不得同时受理同一逻辑订单。
- 发生异常先停新受理，保留已发交易、回调和查补；旧引擎退役另需全量义务核对。
- 逐域核对payments正式登记/两级授权/grant和任务owner后，才审计激活对应迁移任务；通知/查补/退款一并受控，未转交域继续隔离。

#### Expected Outputs

- 发布/停受理/回退执行清单、灰度前后owner和金额核对。
- 实际窗口与差异报告、未完成扩展责任方与运行保障。

#### Acceptance Criteria

- [ ] 获准范围内每单只有一个权威引擎，迟到回调和退款到正确处理方。
- [ ] 观察与差异阈值达到冻结标准，未结束观察不提前标完成。
- [ ] 回退演练保留原资金事实与幂等记录，操作域无越界。

#### Risks

并行旧新创建、以镜像回退掩盖渠道已发生事实、观察期未完成。

#### Dependencies

前置执行阶段：Phase 37、Phase 38、Phase 39、Phase 40、Phase 41、Phase 42、Phase 43。

#### Notes

涉及生产的进一步人类授权是执行输入，不能从‘创建台账’推导；无授权可完成准备但保留阶段未完成。

证据：拟保存为 docs/pay/validation/phase-44.md；当前尚不存在，不代表已验证。

---

### Phase 45: 完成一期验收与未迁移义务交接

#### Status

NOT_STARTED

#### Goal

形成一期可交付结论，并保留全量Rust替代尚未完成的边界。

#### Scope

需求/合同/场景/源码/证据追溯、一期交付说明、扩展覆盖矩阵与旧引擎责任交接。

#### Out of Scope

不把一期通过标为Java全量退役，不把文档产出当实现完成。

#### Inputs

Phase1—44终态证据、本台账扩展E01—E09、旧引擎未结清清单。

输入门禁：Q15扩展真实使用范围与责任角色；全量退役须后续独立证据。

#### Execution Requirements

- 逐项核对验收ID和真实证据，架构现状只同步实际实现。
- 扩展活跃能力、余额冻结/免押/通知/查补义务逐项有处理引擎、责任角色和后续入口。

#### Expected Outputs

- 一期验收与交付报告、运行手册索引、扩展阶段拆分入口。
- 台账状态/指针/决策/变更记录同步，保留历史结果。

#### Acceptance Criteria

- [ ] 主线所有必需阶段实际完成且证据同源；没有失败/skip被改写为通过。
- [ ] 账户子树授权、机器受理、四类身份和首期资金闭环均达到冻结合同。
- [ ] 未迁移扩展仍NOT_STARTED或其真实状态；全量替代目标未被错误标完成。

#### Risks

覆盖清单遗漏旧余额/免押义务、汇总阶段掩盖子项未通过。

#### Dependencies

前置执行阶段：Phase 1、Phase 2、Phase 3、Phase 4、Phase 5、Phase 6、Phase 7、Phase 8、Phase 9、Phase 10、Phase 11、Phase 12、Phase 13、Phase 14、Phase 15、Phase 16、Phase 17、Phase 18、Phase 19、Phase 20、Phase 21、Phase 22、Phase 23、Phase 24、Phase 25、Phase 26、Phase 27、Phase 28、Phase 29、Phase 30、Phase 31、Phase 32、Phase 33、Phase 34、Phase 35、Phase 36、Phase 37、Phase 38、Phase 39、Phase 40、Phase 41、Phase 42、Phase 43、Phase 44。

#### Notes

下一指针只在用户要求的范围内进入已细分扩展阶段；不自动执行E整包或停Java。

证据：拟保存为 docs/pay/validation/phase-45.md；当前尚不存在，不代表已验证。

---
## 5. Current Execution Pointer（当前执行指针）

Current Phase: Phase 1 — 核对源码基线并冻结首期合同。

Current Status: NOT_STARTED。

Current Objective:

收到后续明确执行请求后，先核对S1—S8实际源码与工作树、补足源仓库定位；整理支付账户对象/子树分享、订单权威/唯一键、机器租户准入、首期场景和存量义务合同。当前不执行该步骤，也不将台账创建记为阶段开始。

Current Restrictions:

- 当前只交付本台账；不实施Phase1，不创建业务工程/迁移/发布/真实交易。
- 后续执行前检查全仓在进行的独立改动，保持支付台账与全仓架构台账分离；不回退用户工作。
- Q表缺失信息先通过能完成的静态核对准备具体问题；必要输入到齐前不执行依赖它的资金/schema/生产动作。
- 当前扩展E01—E09不进入默认指针，先确认范围并细分，不能把整包作为一个阶段自动启动。

Next Phase After Completion: Phase 2；若合同必需输入尚未满足，保留Phase1真实状态与具体阻碍，不跳到实现。

## 6. Decision Log（决策记录）

| 日期 | 决策/规划 | 依据 | 影响 |
| --- | --- | --- | --- |
| 2026-10-19 | 建立独立支付台账，只创建文档 | 用户明确调用task-ledger-creator；技能限定只创建台账 | 所有阶段NOT_STARTED；当前不实现或发布 |
| 2026-10-19 | 平台统一管理用户/成员/角色 | 用户要求接入现有租户组织用户体系 | 旧账号仅映射历史，所有管理请求使用平台应用身份 |
| 2026-10-19 | 纳入根组织账户子树USE能力 | 用户明确提出根账户及下级使用场景 | 独立Phase4—7和10；账户管理/使用/授权及交易范围分别验证 |
| 2026-10-19 | 默认分享与转授仍按Q项冻结 | 前轮为分析建议，未给产品默认最终决定 | 建议仅本组织、显式子树、无隐式转授；不能冒充已批准 |
| 2026-10-19 | combined优先评估、legacy显式回退 | 平台已有两模式且消费者默认值不同 | Phase3验证合同，Phase42据支付负载决定默认；无失败自动降级 |
| 2026-10-19 | 不扩大子组织context读取祖先配置 | 平台硬上限向下，账户使用属于支付域授权 | 新增用途关系核验和可用账户投影；机器另有受限合同 |
| 2026-10-19 | 当前不新增组织移动功能 | 现有更新不支持parent_id；支付项目无此实现指令 | 定义未来动态关系语义，用fixture验收，不宣称真实移动API存在 |
| 2026-10-19 | 逻辑订单唯一键不包含merchant/channel | 切账户不能绕过同单有效收款裁决 | 默认tenant/source/orderId，命名空间例外需权威合同 |
| 2026-10-19 | 原账户/配置证据不明不接管资金动作 | 旧单缺完整历史版本，当前绑定不能补造事实 | Phase37—39隔离/核查并保留已验证旧引擎责任 |
| 2026-10-19 | 一期与全量替代分开验收 | 旧系统有余额、转账、免押等独立生命周期 | 45阶段主线只达一期；扩展矩阵必须后续细分并有退役证据 |
| 2026-10-19 | 拆分45个可执行阶段 | 原P0—P10包含多个独立合同与验证终态 | 保留里程碑映射；不为压缩数量制造超大执行阶段 |
| 2026-10-19 | 新库PG为建议，双库要求仍待Q08 | 与现有平台一致但旧项目有双库合同 | 冻结之前不实施最终schema；旧Java维护期不改变双库约定 |

## 7. Open Questions（待确认事项）

所有问题初始OPEN。建议默认仅用于形成可审阅合同，不表示用户已经批准；负责人是待落实的责任角色。每项结论写回本表、Phase1合同和受影响阶段输入，保留来源和日期。

| ID | 问题 | 建议默认与推进边界 | Owner | 受影响阶段 | Status |
| --- | --- | --- | --- | --- | --- |
| Q01 | 旧支付服务哪些客户，商户与历史订单如何映射租户/来源组织？ | 一商户一租户；订单按权威来源归组织；不明数据隔离，不复制给所有共用组织。 | 业务负责人/迁移负责人 | 1、37—39、44 | OPEN |
| Q02 | ‘支付账户’指业务收款商户、渠道商户还是服务商应用？ | 逻辑可授权账户和渠道技术身份分开；首版可复用支付商户聚合，但只能有一套owner/使用授权权威。 | 支付产品/架构负责人 | 1、5—7、9—10 | OPEN |
| Q03 | 创建账户默认仅本组织还是整棵子树？是否覆盖未来后代？ | 建议默认仅本组织；显式选择子树后含自身及未来有效后代；创建即全树可作为明确产品选择，尚未批准。 | 产品负责人 | 1、6、10、43 | OPEN |
| Q04 | 是否支持下级转授、排除分支、组织移动和强一致立即撤销？ | 首版无隐式转授/复杂例外；下一新受理实时校验；跨库非原子。当前不新增组织移动，用fixture验证未来语义。 | 产品/平台架构负责人 | 1、4、6—7、13、40 | OPEN |
| Q05 | 共用账户实际结算给谁，各组织是否不同收款主体？ | 按真实渠道账户/子商户结算；本地组织归属不承诺分账或独立余额/银行入账；缺主体事实不能开展真实渠道测试。 | 财务/渠道负责人 | 1、5、15—21、43 | OPEN |
| Q06 | QuickPay缺失的订单/短链/五类接口后端在哪里，订单事实由谁提供？ | 先核实原服务；缺失则实现受控订单桥接，fake不能当上线权威。 | 业务订单系统负责人 | 1、14、30—33 | OPEN |
| Q07 | 业务单号在哪个来源命名空间唯一，是否跨组织全局唯一？ | 默认tenant/source/orderId；组织与金额冲突拒绝；只在来源证实时加入服务端命名空间，商户/渠道不入唯一键。 | 业务订单/支付架构负责人 | 1、12、14、17、29、37—39 | OPEN |
| Q08 | 新 Rust 支付服务是否仍需同时支持 MySQL/PostgreSQL？ | 建议新库PG17；旧Java维护期保持双库；客户合同要求优先。 | 产品/架构负责人 | 1—2、38、41 | OPEN |
| Q09 | 四场景、直连/服务商模式及退款接口实际开通范围？ | 本版一期按WECHAT_JSAPI/H5、ALIPAY_WAP/LITE四场景规划；收缩须显式改范围/依赖，不能静默skip。 | 支付产品/渠道负责人 | 1、15—23、27、31—33、43 | OPEN |
| Q10 | 机器调用如何取得可信租户/应用开通状态和动态组织关系？ | 新增或核实逐应用逐租户受限合同；不能只用全局Basic遍历租户，也不保存用户token；未知拒绝新受理。 | 平台/支付架构负责人 | 1、12—14、30、40 | OPEN |
| Q11 | 人工/机器退款是否审批、限额、批准有效期及自动重复退款依据？ | 建议人工申请复核且禁止自审；机器按明确策略；所有路径共用原支付额度预占。 | 财务/支付产品负责人 | 1、26—29、35、43 | OPEN |
| Q12 | 旧HMAC/API调用方及通知目标/签名/ACK兼容期限？ | 登记稳定来源/资源范围和合同版本；按原号恢复，目标服务端固定。 | 业务接入/渠道负责人 | 1、12、25、30、39 | OPEN |
| Q13 | 生产schema、旧时间时区、物理删除、历史账户配置有何证据？ | 先用批准副本/导出核对；原账户不明不接管资金动作，当前绑定不能伪造历史。 | 数据库/迁移负责人 | 1、37—39、41 | OPEN |
| Q14 | 旧管理用户与现有全局用户如何核对？是否要求邀请？ | 显式映射；同邮箱/手机号不自动合并；账号在平台开通；当前邀请能力不假定存在。 | 平台管理员/业务负责人 | 1、3、37—38、45 | OPEN |
| Q15 | 余额/冻结、转账、付款、充值、进件、发货、免押、支付分等实际使用和在途义务？ | 保留E01—E09未迁移清单和原引擎责任；一期完成不代表可停Java。 | 业务/财务/旧系统运行负责人 | 1、37、41、44—45及扩展 | OPEN |
| Q16 | 测试域名、渠道应用/商户、证书、白名单、小程序模板/设备及获准操作范围？ | 用隔离环境，真实操作需已有明确授权；缺项不以模拟通过冒充真实渠道验收。 | 渠道/部署负责人 | 1、11、18—21、31—33、43 | OPEN |
| Q17 | 关系证明/DB/HTTP/worker预算、容量、RPO/RTO、留存及灰度窗口阈值？ | 各自冻结有界参数并实测；不虚构生产性能或已经达标的恢复指标。 | 平台/运维/财务负责人 | 1、4、7、13、24、40—44 | OPEN |
| Q18 | 跨租户支付运营、共享渠道密钥和平台根管理权如何分界？ | 默认同租户共享；跨租户另有批准模型；平台根不自动全量退款/读数。 | 产品/平台架构负责人 | 1、3、5、34 | OPEN |
| Q19 | 回调正文、操作审计、个人资料和通知载荷的保留/脱敏/查看规则？ | 最小持久化，必要原文加密限期保存；交易/审计不随平台用户删除级联。 | 数据/运行负责人 | 1、23、25、36—38、41 | OPEN |
| Q20 | 本次后续执行允许哪些发布/环境写入？ | 当前只创建台账；阶段实现需后续明确执行请求；生产发布/真实资金操作按具体批准域进行，不能由文档创建推导。 | 用户/发布负责人 | 11、43—44 | OPEN |
| Q21 | 旧渠道回调地址变更限制、共享账户加密回调、新旧owner分发与旧引擎保留方式是什么？ | 核实原渠道配置；先可信验签解密再按稳定交易标识分发；未完成可信分发时用已验证入口/账户隔离；不广播。 | 渠道/旧系统运行/迁移负责人 | 1、23、37—39、43—44 | OPEN |

### 阶段开始前的输入规则

Phase1可以先做只读核对，不要求所有外部输入已具备。进入Phase2前需确定数据库与基础标识；账户/共享实现前需Q02—Q05；机器资金受理前需Q06/Q07/Q10/Q12；真实渠道前需Q05/Q09/Q16；迁移/灰度前需Q01/Q13—Q17/Q19—Q21。每阶段详情给出更具体门禁。Q项未清而对应动作不可安全具体化时，应留下已完成准备及恢复条件，不伪造通过。

### 未来扩展进入执行的规则

先为选定E项增加满足本台账格式的独立Phase，登记明确合同、来源实现、身份/资金范围、前置状态、具体产物和可测终态；更新汇总、决策和指针。若旧能力真实仍在使用，必须持续保留原引擎/责任/未结清清单，不能因尚未排期而停止运行或宣称全量替代。

## 8. Change Log（变更记录）

| Version | Date | Change |
| --- | --- | --- |
| v0.1 | 2026-10-19 | 创建支付独立执行台账：45个一期阶段、9个未迁移扩展工作包、21项待确认输入；完整纳入根组织账户动态子树授权及其平台/资金边界，补齐正式登记、目标组织见证、管理查询接口与迁移任务隔离。仅文档交付，所有阶段NOT_STARTED。 |

### 本次交付说明

- 假设：基于现有方案建立普通支付一期主线，后续扩展完整保留；默认分享、数据库合同、渠道范围等仍是待冻结建议。
- 需要确认的输入：Q表覆盖真实订单服务/归属、账户与结算对象、共享默认、机器准入、渠道条件和旧系统义务；当前不要求一次性回答全部问题。
- 推荐下一步：明确执行Phase1后，先做源码盘点和合同整理，再针对仍影响后续动作的缺项提出具体问题。

