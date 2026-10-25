# Phase 13 验收记录

状态：**COMPLETED／PASS（用户授权的TASK_OWNED分轮范围）**。依据 [台账 Phase 13](../task-ledger.md)，实现与流程见 [灰度与身份感知恢复](../phase-13-rollout-recovery.md)，最终聚合见 [验收摘要](../../../../local-identity-sso-phase13-qa/final-acceptance-summary.json)。按用户最新要求跳过已成功阶段，容量复用R20完整1800秒steady并由R22真实300秒burst补齐，协议／灰度复用R21；R22另已通过140项双库恢复及第一版UI的restored browser247。R22与R23原initial FAIL247均保留；第二版UI经55项前端门禁后，R24只补失败initial，已PASS343／notification PASS。R24未重跑恢复或restored，两个UI版本及各轮来源分别登记。16图视觉、最终source／runtime与全资产清理均已有独立PASS回执；不将driver单独PASS或分轮聚合表述为同轮连续35分钟通过。

### 执行历史与原始结果

2026-10-25 已实际运行并保留 preflight 01～08 的安全回执，任务库和角色均清理。通知角色权限检查已修复 PostgreSQL 条件重排导致的非序列表权限函数报错；严格 HTTP 客户端已修复 HTTP/1.0／Connection: close 的文件引用生命周期。完整协议轮已通过 221 项检查；08 调试轮实际通过 10／50／100% RP 请求灰度、候选故障与 2.396338 秒显式路由回退。08 浏览器通过 98 个检查后在有效员工会话重新登录的工具回调失败，已改为支持的 PLATFORM 注销再登录；其恢复调试因明确跳过协议而缺少真实 OIDC 创建 receipt，未计恢复通过。后续13调试轮 initial341／restored247及140条恢复检查已通过，但最终扫描仍命中，整轮保持FAIL。源码随后证实普通code标签会被错误登记；历史具体匹配值未保存，不反写旧回执。14轮在容量前主动停止以修正服务／客户端锁等待计量边界；54项Linux工具回归通过后，15轮冻结608文件启动。第15轮在约180秒时已出现容量漏发与延迟不达标，主动停止并保留FAIL；第16轮采用身份release与并发TLS握手，协议221及灰度15项通过，但约120秒仍严重漏发，整轮FAIL并完成任务清理。两轮部分容量及实际清理详见各自失败分析，不能当完整35分钟。独立严格TLS回显对照复现了同进程客户端／网关线程竞争，已改为新解释器常驻网关，21项worker与55项load回归通过。完整流scanner改为精确末12bytes索引，13项等价／边界回归通过，100k模拟凭据＋1MiB完整日志在3.394秒完成。原100会话、1800＋300秒、主计数与延迟阈值全部不变。第17轮以609文件快照cd5d1c842acad7b3f4c1df5b700f3a0d5e9fe1ce375fc15f143d0c4aa454cce2实测约214秒，服务部分分位数已低于候选门槛，但18条introspect漏发使整轮仍FAIL，独立gateway退出与扫描通过。第18轮将负载发生器资源从32／32调整为100 workers／100 in-flight，与100 actor规模对齐，新增五类有界漏发诊断；不改产品配置、请求目标、速率、时间、0.5s调度上限、3s绝对预算或延迟门槛。Linux59项工具回归通过后，第18轮以609文件快照139abb88a721fccc9b6983697c5202e37ccf09eeafe7cca02b1a51f44c83e721实测至约754.5秒自动异常终止；已完成主操作全success且scheduler五类全0，但内层异常类未保存，整轮FAIL。后来独立复现两处harvest跨时点后负数sleep竞态，修为重新计算remaining且仅正值等待，并保留安全异常类别与筛选源码帧；不能倒推该次现场异常已被直接证明。Linux69项回归与五种完整264000槽／2100假秒纯调度模式通过后，第19轮以609文件快照66dba13490fd979db12fa2308f4b2766ece2e3e484cdbfadeeb8881defc655e6启动，原真实2100秒／全部计数及阈值不变。以上为R19时点的历史要求与状态；后续R20～R24按下节最新用户分轮授权及真实结果记录，原FAIL不改判。

活动消费者 [本轮审查](../phase13-consumer-audit.md) 已实际通过 233 项 Node 和 184 项 Rust 普通测试；806 个源码／合同／发行物文件与 13 份日志已登记摘要和凭据模式扫描。[最终只读消费者复核](../../../../local-identity-sso-phase13-qa/final-consumer-source-verification.json)重新枚举806文件：803不变、3项准确变更为第二版portal controller及已在R22／R23／R24封存的两项恢复ops文件，417项既有测试未重跑；portal另外完成55项门禁。当前成员字面量检索口径为29个生产文件、5个writer／11个mutation位置、两处三元UPSERT。上述结果不替代数据库／容量／浏览器验收。

### 最终分轮证据

用户范围调整见 [R21优化中止原因](../../../../local-identity-sso-phase13-qa/full-round-21-abort-reason.json)及 [分轮聚合准则](../../../../local-identity-sso-phase13-qa/split-run-evidence-policy.md)。原失败和中止回执不改判；各项按准确task／source／实际配置分别引用，不拼出未发生的同轮35分钟或合并分位数。

| 轮次／范围 | 实际结果 | 来源与限制 |
| --- | --- | --- |
| R20完整steady | **PASS子项**：1800秒；login3600／code7200／refresh7200／introspect180000四计数一致、全success、零deficit，原延迟门槛通过。 | [R20原始FAIL聚合](../../../../local-identity-sso-phase13-qa/full-round-20-failed.json)，task `01a171de-4a23-7c08-a0cc-0d29e47bafc3`。只复用steady；原burst8553个admission_saturation与整轮FAIL保留。 |
| R21协议／灰度 | **PASS子项**：protocol221、canary15、真实candidate503／无fallback／显式set0／两RP新登录，rollback3.336875秒。 | [R21中止聚合](../../../../local-identity-sso-phase13-qa/full-round-21-aborted.json)，task `01a1720e-4dd8-73af-a166-fa7c21547fd7`。重复steady约634秒后按用户要求中止，原driver FAIL及sent未completed差额保留，不补成完整1800秒。 |
| R22定向burst | **PASS**：固定300秒，主观察／收尾300.136467秒；100actor／100worker／100in-flight，login1200／code2400／refresh2400／introspect60000四计数一致，零deficit／非预期错误。 | [容量原报告](../../../../local-identity-sso-phase13-qa/targeted-round-22-capacity.json)、[独立复核](../../../../local-identity-sso-phase13-qa/targeted-round-22-capacity-review.md)。只执行失败burst，6000认证＋60000authority；不重跑成功steady。 |
| R22 initial browser | **FAIL原样保留247检查**；initial通知生命周期 **PASS**。 | [initial browser](../../../../local-identity-sso-phase13-qa/targeted-round-22-browser-initial.json)、[launcher](../../../../local-identity-sso-phase13-qa/targeted-round-22-browser-launch-initial.json)、[分析](../../../../local-identity-sso-phase13-qa/targeted-round-22-browser-cross-tab-analysis.md)。`CROSS_TAB_CUSTOMER_SELECTION`旧页profile401后永久signed-out，等待超时；不能把通知PASS或后来restored成功改写成initial PASS。 |
| 第一版门户UI修复 | 旧实现4项定向回归先FAIL；修复后 **43／43 Node、typecheck、lint、production build PASS**。 | [第一版前端回执](../../../../local-identity-sso-phase13-qa/portal-profile-reconciliation-validation.json)。仅profile读401／409时额外读取一次当前session及严格绑定profile；持续401仍清空，403／503／协议异常不重试，写操作不重放，epoch／abort保留。该版本用于R22 restored及R23 initial。 |
| R22恢复／restored | **恢复PASS140、restored browser PASS247、notification.after PASS、launcher0／完整扫描空**。 | [R22完整安全聚合](../../../../local-identity-sso-phase13-qa/targeted-round-22-complete.json)、[restored browser](../../../../local-identity-sso-phase13-qa/targeted-round-22-browser-restored.json)、[launcher](../../../../local-identity-sso-phase13-qa/targeted-round-22-browser-launch-restored.json)。task `01a17221-6d7e-7a61-8db8-b4008279d633`；restored使用第一版UI，不能描述为当前第二版controller已重跑恢复。driverPASS、22个产品／恢复进程与独立gateway停止／完整扫描PASS，两库与限定角色删除、container hosts恢复；这不包含全部Windows／历史私有资产清理。 |
| R23只复测initial | **FAIL247原样保留**；notification PASS。 | task `01a17238-0b65-7be9-a9d9-11a90c776027`；[initial原报告](../../../../local-identity-sso-phase13-qa/targeted-round-23-browser-initial.json)、[610文件snapshot](../../../../local-identity-sso-phase13-qa/targeted-round-23-source-snapshot.json)，seal `8a55e452be8a1c6bed210e327250538b01bd91c14165acbc84896ca3524299e5`。同一cross-tab阶段在profile401后的核对session503，旧数据清空、UI unavailable而等待超时。缺少该请求的lease／uncertain现场，不认定确定根因或已注销。 |
| 第二版门户UI有界核对 | 旧实现5项定向回归先FAIL；修复后 **55／55 Node、typecheck、lint、production build与独立只读复核PASS**。 | [第二版门禁](../../../../local-identity-sso-phase13-qa/portal-profile-lease-reconciliation-validation.json)、[独立复核](../../../../local-identity-sso-phase13-qa/diagnostics/portal-profile-lease-reconciliation-independent-review.json)。仅在profile401／409触发的核对链内对session503最多3读、100／200ms可取消等待、500ms后续派发预算；第二profile失败不重试，epoch／abort及严格绑定保留，普通初始503仍立即unavailable，持续503清空且不重放写操作。 |
| R24只复测initial | **PASS343、notification PASS、pageerrors0、launcher0、8PNG**。 | task `01a1724a-75de-71d4-9fbf-7b9dfd440a53`；[安全聚合](../../../../local-identity-sso-phase13-qa/targeted-round-24-complete.json) SHA `3826bb3a5930c8178b41558887126dc27023cc14d1560664a96ed157f6098f58`、[initial](../../../../local-identity-sso-phase13-qa/targeted-round-24-browser-initial.json)、[launcher](../../../../local-identity-sso-phase13-qa/targeted-round-24-browser-launch-initial.json)、[610文件snapshot](../../../../local-identity-sso-phase13-qa/targeted-round-24-source-snapshot.json) seal `886feadc9893dbd447628db1996ed96981a37892e50c2c0c26e2c90a84a0f7c6`。第二版UI实际补测原失败initial；driver5进程退出／扫描、当次两DB／roles清理PASS，未重新执行容量、协议、灰度、恢复或restored。最终逐图及全局私有资产清理见下节独立回执。 |

R24网关记录实际authority503为0，另有3次task-owned authority-unavailable故障注入响应。因此initial343成功不是自然lease503已现场复现并确定修复的证据；有界503核对行为由12项新增回归覆盖，R23具体根因仍未确定。浏览器沿用任务CA的Edge `ignoreHTTPSErrors`，Node与两个BFF验证任务CA和准确hostname，不代表生产DNS／OS信任或发布验收。

R22 burst成功服务p95／p99（毫秒）：login142.073674／178.600577，code283.635366／371.348956，refresh117.148257／150.580547，introspect48.161455／64.043412。code／refresh≤500／1000、introspect≤100／300、login p95≤1000的原门槛均通过；bootstrap／辅助HTTP分计。R20 steady原分布与R22 burst各自判定，24000认证／240000authority只表示两阶段计划覆盖合计。

R22恢复实测RPO **5.805秒**、RTO **21.443394秒**，RTO准确scope为 `FAULT_TO_OLD_DENIALS_AND_FRESH_DUAL_RP_BUSINESS`，不含整个restored browser或最终清理。checkpoint1792916752218ms、fault1792916758023ms、业务最后commit1792916757811ms、该scope完成1792916779466ms；备份后业务marker确实丢失，认证状态主动废弃单列。140项为同一恢复检查集合，不因跨两行引用重复计数。

[第一版前端变更清单](../../../../local-identity-sso-phase13-qa/targeted-round-22-frontend-change.json)将R22初始seal `56126203f935343810aa8a14e17607bc05597beb88fb77d9a74d2f545fba0fb5`与[restored阶段seal](../../../../local-identity-sso-phase13-qa/targeted-round-22-restored-source-snapshot.json) `2a5ae6dc49dc461dbb966f519be64108e17dc4583413360561fe82b621d44296`分开：第一版portal `frontend/src/controller.ts` SHA为 `1ccd1b83691adbfff04bb3c9a17bec4e164fb2dad917adf32fdec9d8fbba7f6d`，测试SHA为 `b7195dee67874063d53a7e82955e3fa5898f89cb252f9abd87f38f91eb62c98b`。R24当前第二版controller SHA为 `43f9840bf525e086f1e7125f658f11818e60df968a8197832f40a906b226eb9f`，测试SHA为 `e7b6c18b7d64ca1eaac51e4018b526b65bb43c904095040256f7cd29485eb9cc`；55项门禁及initial343绑定此版本，不把R22恢复／restored称作其重跑结果。[最终runtime/source复核](../../../../local-identity-sso-phase13-qa/final-runtime-source-verification.json)证明568个既有产品文件、全部Rust／migration／lock与7个后端二进制pins不变，准确登记2项UI变更。UI修复后仅补相关浏览器，已通过容量不重跑。

## 最终视觉、源码与全资产清理

[最终视觉复核](../../../../local-identity-sso-phase13-qa/final-visual-review.json)逐一审查16张PNG并PASS：R24第二版UI initial8张、R22第一版UI restored8张；桌面／移动资料、选择、权威不可用／明确重确认、员工无客户身份及局部／全局退出均无裁切或重叠。[最终source snapshot](../../../../local-identity-sso-phase13-qa/final-source-snapshot.json)610项，seal与R24一致为 `886feadc9893dbd447628db1996ed96981a37892e50c2c0c26e2c90a84a0f7c6`；消费者806=803+3、runtime568+2／7pins按各自准确选集独立复核，不把文档或清理操作算作产品重测。

[最终只读runtime观察](../../../../local-identity-sso-phase13-qa/diagnostics/final-runtime-observation.json)证明观察时任务DB／roles／connections均0，24个runtime任务目录均绑定真实terminal公开归档；它明确只观察，不签发清理许可。[root current clearance](../../../../local-identity-sso-phase13-qa/diagnostics/final-current-clearance.json)随后PASS：任务进程和监听0，443及38081～38084监听0，bridge22394实际exit0，R24 driver5进程与gateway完整扫描PASS、两Vite子进程close且全部流reader完整、launcher0。该回执将当前root核查与历史各轮局部清理分开。

[精确Linux私有材料清理](../../../../local-identity-sso-phase13-qa/diagnostics/final-private-cleanup-applied.json)按plan SHA `2d58b8cda0809b71675b020cbd09ce66acb079a8ec6e8abe7047a76e825146f9`删除24个任务目录与Phase13 PKI；[删除后验证](../../../../local-identity-sso-phase13-qa/final-linux-private-cleanup-verification.json)确认任务目录0、PKI不再存在，未读取私有内容、未触及Phase09材料。[Windows清理](../../../../local-identity-sso-phase13-qa/final-windows-private-cleanup.json)删除8个任务目录及1个PKI目录中的15个文件，reparse0、无残留。两侧均不声明安全擦除。PG在观察后09:06:48 UTC正常exit0、非OOM；通用network-container dry-run因此失败后改用network-none完成文件清理，未重启DB、未重测已通过阶段，也未把dry-run失败改写成首次成功。

## Self-Check：台账五项标准

| 验收项 | 当前结果 | 实际证据与剩余事项 |
| --- | --- | --- |
| 平台／门户真实配置下完成双身份登录／选择／局部与全局撤销，客户不泄露员工权限、目录或历史通知 | **PASS** | R24第二版UI initial343／notification PASS与R22第一版UI restored247／notification.after PASS分别验证实际双RP、双向局部撤权、全局注销、cross-tab与客户隔离；R22／R23原initial FAIL247保留。R24 launcher0、pageerrors0与当次扫描PASS，最终16图视觉及全局清理独立PASS。 |
| 按容量／SLO／RPO／RTO完成演练，未达项修复后才通过 | **PASS（TASK_OWNED分轮范围）** | R20 steady1800秒与R22 burst300秒分别达到原RPS／计数／延迟门槛；R22双库／材料／witness恢复PASS140，RPO5.805s／RTO21.443394s（准确scope见上）。不认定同轮35分钟或生产SLO／PITR。 |
| 身份感知回退与真实退役条件，历史对象N/A有清单依据，活动兼容仍受撤销保护 | **PASS（任务回退／范围）** | R21 protocol221／canary15及10／50／100%准确RP计数、candidate503、无fallback、set0、两RP新登录，rollback3.336875s；八仓消费者417普通合同通过。本新库历史退役N/A，现行兼容路径继续受控支持。 |
| 查询／JOIN／ON CONFLICT／DELETE／trigger／广播的双身份歧义审查闭环，旧无唯一成员证明凭据拒绝 | **SQL／旧载体及最终静态复核PASS** | 29 literal／5 writer／11 mutation／2三元UPSERT分类保留；R21协议及R22 canonical history、恢复旧密码／code／device／refresh／平台／app／opaque／pending负例实际PASS。R24 seal、806=803+3消费者重新枚举及runtime568+2／7pins准确区分既有通过与UI修复范围。 |
| 全阶段交付可追溯，实施状态、legacy支持和首个客户业务范围在文档一致 | **PASS** | 分轮原始来源、R22／R23 initial失败、两版UI及各自initial／restored范围完整；最终source／静态发行字节、16图、当前root clearance及两侧私有资产清理PASS。台账与README按最终摘要统一收口，活动legacy支持、生产默认关闭与本人资料首个业务范围保留。 |

## 准备与证据分层

| 范围 | 当前事实 | 证据与限制 |
| --- | --- | --- |
| Phase12依赖 | COMPLETED，独立拓扑基线 | [Phase12验收](phase-12.md)含42普通＋3identity PG、平台12stage、protocol221、browser214记录／8图及共享平台库restricted identity health；不替代本阶段双库／完整业务恢复。 |
| 四份历史准备 | PREPARATION_ONLY／NOT_ACCEPTANCE | [总体](../../../../local-identity-sso-phase12-qa/phase13-readonly-preparation.md)、[负载](../../../../local-identity-sso-phase12-qa/phase13-load-implementation-preparation.md)、[恢复](../../../../local-identity-sso-phase12-qa/phase13-recovery-implementation-preparation.md)、[通知](../../../../local-identity-sso-phase12-qa/phase13-notification-implementation-preparation.md)保留归档时点状态／源码行号，当前以QA13新源码与真实receipt为准。 |
| 当前integration | 源码接线合同，非运行证据 | [integration-context.md](../../../../local-identity-sso-phase13-qa/integration-context.md)记录私有context、callbacks与原完整计划；最新分轮授权以本记录／runbook为准，initial／restored周期及清理仍按真实执行记录，私有state不得直接序列化。 |
| 工具镜像 | 准备PASS | [tools-image-result.json](../../../../local-identity-sso-phase13-qa/tools-image-result.json)记录phase13-tools及准确image SHA，仅加Debian Argon2，productDependenciesChanged=false；不是reset／restore实测。 |
| 独立PKI／keyrings | 准备PASS | [pki-result.json](../../../../local-identity-sso-phase13-qa/pki-result.json)CA:false leaf、auth／console／customer SAN、两BFF keyrings，早期PKI未动；不是live／OS信任结果。 |
| integration preflight | 计划跳过容量，NOT_ACCEPTANCE | root先用调试轮验证完整接线；skip-load／缩短／fake运行不得满足容量项，即使其他stage局部通过也不封账。 |
| helper纯／AST验证 | 独立准备门禁PASS，非live替代 | [scanner回执](../../../../local-identity-sso-phase13-qa/capture-scanner-validation-round02.json)、[gateway keepalive回执](../../../../local-identity-sso-phase13-qa/gateway-keepalive-validation.json)、[burst定向工具回执](../../../../local-identity-sso-phase13-qa/load-generator-burst-retest-validation.json)各按准确source／日志SHA保存；两版UI分别43／55项门禁，真实R22／R24 live另列。纯测试不能当真实容量／DB／浏览器运行。 |

## 实际运行登记

| 项目 | 当前结果 | 实际回执与剩余事项 |
| --- | --- | --- |
| 初始独立四进程／canonical迁移／roles／protocol | PASS子项 | R21协议221及R22完整安全聚合证明task／cluster／两DB marker、canonical20 history、受限roles、运行SCRAM与normal pins；各任务来源分列，非R23重跑。 |
| task RP请求canary10／50／100%及显式回退 | PASS（R21复用） | 准确两client／fixture tenant、eligible=baseline＋candidate、实际比例、candidate503／无fallback／set0、两RP新登录及3.336875s回退；原15检查与candidate清理均有回执。 |
| steady＋burst分轮容量 | PASS（按用户授权） | R20稳态3600／7200／7200／180000与R22突发1200／2400／2400／60000，各行四计数一致；分别真实1800秒与300秒，不宣称同轮35分钟或相同sid横跨两段。 |
| 容量分位数／资源／辅助计数 | PASS | R22 introspect p95/p99=48.161455/64.043412ms；code283.635366/371.348956；refresh117.148257/150.580547；password p95=142.073674。CPU／RSS／DB／Argon2／锁／调度与辅助HTTP均单列，独立复核PASS。 |
| initial browser／notification | **R24 PASS343／通知PASS**；R22／R23 FAIL247保留 | 第二版UI只补失败initial，pageerrors0、launcher0、8PNG；当次driver5进程退出／扫描、两DB／roles清理PASS。R22 restored来自第一版UI；最终16图及全局清理另有独立PASS回执。 |
| 双库／材料checkpoint及外部witness | PASS（R22） | 140恢复检查集合覆盖两库owner/comment/ACL/catalog/history／安全投影、加密材料认证、有效旧载体、高水位／deny、旧signed sequence／tamper拒绝。 |
| 原名restore／fence／deny-only／轮换reset | PASS（R22） | 同一140检查集合覆盖两个真实pg_restore、库外材料、超highwater generation、旧密码／全载体拒绝、批准账号新密码、受限启动receipt及旧SCRAM密码拒绝；不重复计为另140项。 |
| restored browser／notification／RPO／RTO | PASS（R22） | 修复后restored247、notification.after PASS、launcher0／完整扫描空；RPO5.805s／RTO21.443394s与上列原始时点、业务marker丢失／认证主动废弃分类完整。 |
| 最终source／consumer／SQL／visual／清理 | **PASS** | 417消费者既有合同未重跑、806=803+3静态枚举、SQL原表、runtime568+2／7pins复核PASS；最终610 seal与R24一致，16图逐一PASS。root current clearance与Linux24任务目录／PKI、Windows8任务目录／PKI清理PASS，无任务进程／监听或私有材料残留。 |

容量仅TASK_OWNED隔离候选；用户授权分轮复用与重测不等于同轮35分钟，也不证明滚动30天99.9%。恢复候选RPO≤24h、RTO≤4h已按R22实测判定；生产15min／60min、持续WAL／PITR、异地回读和备份调度不由年轻checkpoint或source函数推定。路由回退仅同binary配置，不声明旧二进制降级。initial失败的补测与R22 restored成功分别真实取证，不沿用Phase12的214记录。

## SQL逐命中审查登记

当前逐命中只读源码审查已完成，检索backend/src的memberships、mutation／ON CONFLICT／JOIN／fetch／first／[0]／LIMIT与广播，并追踪0001～0020相关函数最终替换关系。29个生产区域文件包含memberships字面量（包含配置／路由标签，非全部SQL）；排除remote.rs:407起cfg(test)后，生产成员mutation为5文件／11个SQL位置，两个成员ON CONFLICT均三元键。当前检索与逐调用范围未发现新生产SQL／接口安全阻断；该只读SQL分类本身不计Cargo／DB运行。R21协议与R22恢复负例已有真实PASS、Rust／schema字节未改；R24 source seal与最终消费者／runtime字节复核已登记，下列SQL与消费者原表保留，各阶段结果独立引用。

| 当前源码位置／行号 | 语义分类与核对结论 | 既有回归源码 |
| --- | --- | --- |
| [membership_lifecycle/service.rs](../../../src/membership_lifecycle/service.rs):333/436/483/551/634/656/696 | 精确成员／唯一类型；user＋org fetch_all锁全部关系后按源ID／目标类型定位。同类型409，状态递增版本、删除按ID，第二身份与替换分开，不原地改type。 | R1 |
| [routes/users.rs](../../../src/routes/users.rs):1165/1217/1241/1242/1325 | 员工bulk集合；重复org先拒绝，DELETE限EMPLOYEE及管理范围，三元UPSERT仅部门／岗位，不改原ID／停用状态；离开按准确员工ID。 | R2 |
| [routes/onboarding.rs](../../../src/routes/onboarding.rs):210/213/225/871/877 | 唯一员工类型／业务初始化；显式EMPLOYEE＋三元DO NOTHING后复查已存在员工及启用状态。role绑定幂等不是成员二元冲突，LIMIT1取已有管理员另核员工资格，不作登录成员选择。 | R1/R3 |
| [seed.rs](../../../src/seed.rs):1336；[users.rs](../../../src/routes/users.rs):561；[remote.rs](../../../src/routes/users/remote.rs):244 | 明确EMPLOYEE写入；remote welcome要求恰好一行，静默抑制不是成功。客户／第二身份仍走生命周期门禁。 | R1/R3 |
| [users.rs](../../../src/routes/users.rs):150/178/186/220/402；[remote.rs](../../../src/routes/users/remote.rs):119/206 | 账号级EXISTS／全局范围集合命令；RETIRE接受时捕获全部成员(id,identityVersion)，只删原快照，不领养新行／新版本，无type条件符合全账号语义。 | R3/R7 |
| [users.rs](../../../src/routes/users.rs):722/729 | 全局账号删除兼容分支；remote转RETIRE，本地DELETE users经全局范围校验，不误当员工局部离职。 | R3/R7 |
| [authorization.rs](../../../src/authorization.rs):68/230；[users.rs](../../../src/routes/users.rs):75；[roles.rs](../../../src/routes/roles.rs):655 | 原session完整proof先于角色／超管；非EMPLOYEE先拒，祖先支持集合只认ENABLED EMPLOYEE，客户不支持角色／缓存超管。 | R2 |
| [service.rs](../../../src/membership_lifecycle/service.rs):801/814 | 最终员工支持集合清理；删除／替换后仅清理失去剩余有效员工及祖先支持的角色，不按user全删、不让CUSTOMER支持员工角色。 | R2 |
| [data_grants/store.rs](../../../src/data_grants/store.rs):84/272；[discovery.rs](../../../src/data_grants/discovery.rs):156；[app_members.rs](../../../src/routes/app_members.rs):227/258 | grant写精确membership并核user／org／tenant／type／状态／版本；候选仅有效直接员工，返回user ID是业务DTO，不代替身份资格。 | R2/R8 |
| [auth.rs](../../../src/routes/auth.rs):61/333/365/399 | 展示／审计／选择票据锚点；可显示客户／停用且标不可选，find用于LOGIN审计，accounts.first只锚签名选择票据，保留全部验证集合，不直接签业务context。 | R4 |
| [auth.rs](../../../src/routes/auth.rs):510/642；[oauth2.rs](../../../src/routes/oauth2.rs):1472 | 明确选择后唯一性；候选多于一条先拒，员工context或password grant按client允许集合／准确成员过滤，零／多行拒后才[0]或remove(0)。 | R4/R5 |
| [oauth_issuance.rs](../../../src/oauth_issuance.rs):109；[app_token.rs](../../../src/app_token.rs):139；[app_authorization/store.rs](../../../src/app_authorization/store.rs):77 | 完整凭据JOIN以member ID定位，再核user／org／type／身份及账号代际／来源／app／父session；partial／旧proof不按当前关系补绑。 | R4 |
| [bff/context.rs](../../../src/bff/context.rs):69；[customer_portal/store.rs](../../../src/customer_portal/store.rs):87/276/338 | 两入口精确交集；固定EMPLOYEE／CUSTOMER，与RP／resource client／app／tenant／org相交，客户最终原source／组织／版本复验。 | R5 |
| [notifications.rs](../../../src/notifications.rs):238/425/543/555 | 员工广播／原收件身份；publisher绑定session／member／version；APP有效员工，SYSTEM按user／tenant／org去重后INSERT SELECT解析员工ID／version，客户不产生第二条员工收件。 | R6 |
| [notifications.rs](../../../src/notifications.rs):396/469；[routes/notifications.rs](../../../src/routes/notifications.rs):41/650 | 业务幂等／可见性；publication含publisher member／version，收件原proof，列表／详情／改删共用完整VISIBLE，prefs按member＋version；业务唯一键不误报成员二元键。 | R6 |
| [search.rs](../../../src/routes/search.rs):76/79；[organizations.rs](../../../src/routes/organizations.rs):139/256；[tenants.rs](../../../src/routes/tenants.rs):82/149/748；[monitor.rs](../../../src/routes/monitor.rs):79 | 展示／统计；scope内LIMIT1仅代表组织名称，不签凭据。人数DISTINCT user，删除依赖／审计count统计关系行，允许双身份两行。 | R7 |
| [0013](../../../migrations/0013_membership_identity_policy.sql):16/19；[readiness/schema.rs](../../../src/database/identity_readiness/schema.rs):105/146/180；[identity_readiness.rs](../../../src/database/identity_readiness.rs):346 | 模型／catalog／独立收缩；三元唯一、身份immutable、版本单调，二元约束实际存在／缺失受catalog验证，独立job收缩，runtime不自动迁移。 | R1/R9 |
| [0014](../../../migrations/0014_credential_identity_binding.sql):135/151/158；[0018](../../../migrations/0018_platform_bff.sql):123；[0020](../../../migrations/0020_identity_deployment.sql):233 | 当前有效trigger；成员按OLD.id局部撤派生，当前session活性为0018，0020账号代际bridge按user全局撤并记实际数量。0001／0013旧定义被替换，不改写历史字节。 | R4/R6/R10 |

回归键只标当前源码中的已有测试，不计为本轮执行：R1 [membership_lifecycle_tests.rs](../../../src/membership_lifecycle_tests.rs):873/1471/1592；R2 [employee_isolation_tests.rs](../../../src/employee_isolation_tests.rs):525/723/793/840/877/1249；R3 [remote.rs测试](../../../src/routes/users/remote.rs):547/681/708；R4 [identity_binding_tests.rs](../../../src/identity_binding_tests.rs):622/687/906与 [oauth_issuance.rs](../../../src/oauth_issuance.rs):386；R5 [customer_identity_tests.rs](../../../src/customer_identity_tests.rs):720、[customer_portal_tests.rs](../../../src/customer_portal_tests.rs):411/439/567及 [bff_tests.rs](../../../src/bff_tests.rs):2060；R6 [notification_identity_tests.rs](../../../src/notification_identity_tests.rs):494/564/703/758/798/963/1496；R7 [scope_tests.rs](../../../src/scope_tests.rs):292及 [organization_subtree_reporting_tests.rs](../../../src/organization_subtree_reporting_tests.rs):435/441；R8 [data_grant_discovery_tests.rs](../../../src/data_grant_discovery_tests.rs):660/843/1055及 [management_audit_tests.rs](../../../src/management_audit_tests.rs):729；R9 [readiness/tests.rs](../../../src/database/identity_readiness/tests.rs):86；R10 [identity_deployment_tests.rs](../../../src/identity_deployment_tests.rs):515/645。

测试文件和remote cfg(test)内省略type的INSERT／负例查询／故障trigger不计生产writer。目录业务owner／assignee账号、ChatRoom的socket.id、历史migration与scope展示LIMIT1分别归类，不因名字含user/org就改成成员凭据。最终本阶段运行仍须证明旧凭据拒绝、通知／恢复和catalog闭环。

## 活动消费者与legacy合同登记

当前八仓只读消费者审查完成，未发现新安全阻断。范围为六旧仓platform／SDK／stores／work-orders／notification／examples与新identity／portal。源码、登记fixture、真实任务流量与生产流量分开；以下路径／行号是当前读取时点，既有回归只证明测试源码存在，不冒称本轮运行。

| 消费者／载体 | 当前合同与准确源码位置 | 运行与退役限制 |
| --- | --- | --- |
| 平台员工BFF | [protocol.rs](../../../src/bff/protocol.rs):291固定PLATFORM_CONSOLE＋EMPLOYEE；[context.rs](../../../src/bff/context.rs):69精确员工；[mod.rs](../../../src/bff/mod.rs):118逐请求原BFF／sid／代际／policy复验；[authentication-mode.ts](../../../../frontend/src/lib/authentication-mode.ts):3先定模式。 | 上游失败不切登录方式；BFF服务端OIDC refresh不同于公开legacy refresh。 |
| 平台psr_兼容refresh | [frontend api.ts](../../../../frontend/src/lib/api.ts):40与 [oauth2.rs](../../../src/routes/oauth2.rs):1059实际POST /oauth/token调用session::refresh，拒client credentials／scope扩大。 | 不能虚构/api/auth/refresh，也不能与legacy OAuth或/oidc/token混成一个namespace。 |
| 活动SDK3 | [release.json](../../../../../saas-micro-app-sdk/release.json):3、[generated](../../../../../saas-micro-app-sdk/generated/protocol.mjs):2固定3.0.0／MicroProps2.0／Context1.0／Notification1.0；[http.mjs](../../../../../saas-micro-app-sdk/src/http.mjs):82/107捕获Bearer、credentials omit；[notification-publisher](../../../../../saas-micro-app-sdk/src/notification-publisher.mjs):14仅HTTP发布。 | REPOSITORY_CANDIDATE／private package不是外部已发布；SDK不登录、不refresh、不建Socket.IO，DTO真实名MicroProps／AppAuthorizationContext。 |
| SDK1／2归档 | [release.json](../../../../../saas-micro-app-sdk/release.json):25与 [generation.test.mjs](../../../../../saas-micro-app-sdk/tests/generation.test.mjs):94规定不可变归档及逐文件digest。 | 历史字节与当前vendor按最终不变量另验，不把旧README的SDK2标签当活动SDK2消费者。 |
| stores | [config.rs](../../../../../saas-stores/backend/src/config.rs):217默认legacy；[authorization/client.rs](../../../../../saas-stores/backend/src/authorization/client.rs):77固定legacy introspect＋context或显式combined /authorize；[authorization.rs](../../../../../saas-stores/backend/src/authorization.rs):77逐请求有界校验；[policy.rs](../../../../../saas-stores/backend/src/authorization/policy.rs):229/253分别校验introspection与精确context。 | 失败直接传播，无fallback；生产src不登录或兑换/oauth/token；3s／1s及actor／目录witness不放宽。 |
| work-orders | [config.rs](../../../../../saas-work-orders/backend/src/config.rs):218默认combined；[client.rs](../../../../../saas-work-orders/backend/src/authorization/client.rs):77仍支持显式legacy；[authorization.rs](../../../../../saas-work-orders/backend/src/authorization.rs):69新取授权；[member_contracts.rs](../../../../../saas-work-orders/backend/tests/member_contracts.rs):1013显式同assignee重验。 | 无fallback／成功授权缓存；后端HTTP contract1.0与MicroProps2／SDK3分开，业务owner userID不代替身份。 |
| notification | [protocol.rs](../../../../../saas-notification/src/protocol.rs):113/163/186严格{token}、公开5／12字段；[platform.rs](../../../../../saas-notification/src/platform.rs):66固定proof；[server.rs](../../../../../saas-notification/src/server.rs):180/291逐条及emit前后复验；[store.rs](../../../../../saas-notification/src/store.rs):25原recipient。 | 不登录／签发／refresh；周期proof不改绑／续期限，客户首版无通知；持久行、hint与连接member／type／version相交。 |
| machine／CLI | [oauth2.rs](../../../src/routes/oauth2.rs):1366支持现有confidential MACHINE／LEGACY_MIXED；[bootstrap.rs](../../../tools/platform-admin/src/bootstrap.rs):93新登记严格MACHINE＋[]＋client_credentials及EMPLOYEE app；[http.rs](../../../tools/platform-admin/src/http.rs):57/141仅输入Bearer／同源/api。 | CLI无登录／密码／兑换／refresh；登记不是实际机器流量，新规则不等于全部LEGACY_MIXED已退役。 |
| Vue／jQuery／报表／聊天 | [Vue app.js](../../../../../saas-examples/micro-vue/public/app.js):150、[jQuery](../../../../../saas-examples/micro-jquery/public/app.js):83用props.platform；[报表](../../../../../saas-examples/report-center/src/app.tsx):148用app Bearer／omit／signal；[聊天lifecycle](../../../../../saas-examples/websocket/src/qiankun-lifecycle.ts):14、[server](../../../../../saas-examples/websocket/backend/src/server.rs):86为匿名socket.id。 | 聊天是挂载期授权，不是member writer／持久通知消费者；源码示例不当生产流量。 |
| OAuth playground | [http.rs](../../../../../saas-examples/oauth-playground/backend/src/http.rs):118/255、[upstream.rs](../../../../../saas-examples/oauth-playground/backend/src/upstream.rs):127实际legacy machine/password/code＋PKCE/device/refresh统一/oauth/token；[config.rs](../../../../../saas-examples/oauth-playground/backend/src/config.rs):51默认password关闭。 | 缺配置不广告能力，各流程独立client；不是标准OIDC RP，不证明生产旧流量。 |
| 独立portal | [protocol.rs](../../../../../saas-customer-portal/backend/src/protocol.rs):288固定CUSTOMER_PORTAL＋CUSTOMER；[resource.rs](../../../../../saas-customer-portal/backend/src/resource.rs):188客户验证；[config.rs](../../../../../saas-customer-portal/backend/src/config.rs):163独立resource client；平台 [store.rs](../../../src/customer_portal/store.rs):251最终source proof。 | OIDC／refresh／opaque留服务器；门户DTO不是SDK Context1，不向员工API／通知开放。 |
| 独立identity | [Cargo.toml](../../../../../saas-identity/Cargo.toml):7无platform／portal业务crate；[lib.rs](../../../../../saas-identity/src/lib.rs):52装配账号／SSO／OIDC；[internal.rs](../../../../../saas-identity/src/internal.rs):127/150私有单用途服务与customer proof。 | service Basic不是人员proof，稳定issuer／独立keyset／两RP合同不因切流改变。 |

当前回归来源：stores [client_tests.rs](../../../../../saas-stores/backend/src/authorization/client_tests.rs):26/310、[witness_tests.rs](../../../../../saas-stores/backend/src/authorization/witness_tests.rs):29与 [postgres_contracts.rs](../../../../../saas-stores/backend/tests/postgres_contracts.rs):980；work-orders [authorization/tests.rs](../../../../../saas-work-orders/backend/src/authorization/tests.rs):494/552、[member_contracts.rs](../../../../../saas-work-orders/backend/tests/member_contracts.rs):1046/1534；notification [identity_isolation.rs](../../../../../saas-notification/tests/identity_isolation.rs):774/839/890；平台 [bff_tests.rs](../../../src/bff_tests.rs):2004/2245/2342与 [customer_portal_tests.rs](../../../src/customer_portal_tests.rs):411/567；SDK [client.test.mjs](../../../../../saas-micro-app-sdk/tests/client.test.mjs):54、playground [upstream_contract.rs](../../../../../saas-examples/oauth-playground/backend/tests/upstream_contract.rs):302/520、CLI [bootstrap.rs](../../../tools/platform-admin/src/bootstrap.rs):630。业务PG的上游authority为HTTP fixture，不当平台真实成员DB；这些是既有测试源码，不计本轮执行。

## 退役N/A与文档同步

本TASK_OWNED身份新库无历史账号／凭据导入，也无已登记计划退役的历史客户端；历史转换、停用日期及观察窗口只在这一范围N/A。活动SDK／app派生／机器grant／CLI／playground／legacy及combined调用仍支持并受新撤销边界保护；其他部署是否有旧客户端未知，不编owner、流量或停用日期。

此前SDK／工程当前事实已列入文档修正清单：活动SDK3.0.0、八仓及identity／portal实际导航；历史archive／vendor与Phase1～12证据保留。当前同步用户分轮授权、R20～R24实际结果、两版门户UI修复及最终视觉／source／扫描／全局清理PASS；台账／README由主执行者按同一最终验收摘要统一收口，历史失败和原始字节不改写。

## 完成范围与证据安全

本阶段按用户要求复用R20成功steady、R21成功协议／灰度及R22成功burst／恢复／第一版UI restored；第二版UI的失败initial补测由R24 PASS343闭合，最终source／静态字节、各轮完整扫描、16图视觉与全局清理回执均PASS，台账五项在用户授权的TASK_OWNED分轮范围内完成。R22／R23原initial FAIL不改判，原历史FAIL与用户优化中止原样保留；各轮通过子项不能相加宣称同轮35分钟、当前controller重跑恢复或额外独立场景。preflight仍为NOT_ACCEPTANCE，生产默认关闭、生产SLO／PITR及其他部署历史退役不由本任务推定。

只保存固定类别、状态、字段名、公共pins、计数、原始时点与字节／SHA；不输出密码、PHC、token、Cookie、Basic、secret、env、连接串、完整私有fixture或raw工具诊断。外部本地witness不是灾备冗余，ID Token离线签名仍可能有效；必须实际证明旧在线授权与业务Bearer被拒。首个客户业务维持本人资料，生产默认关闭与UUID专项独立。
