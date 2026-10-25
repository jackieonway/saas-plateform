# Phase 13 灰度、容量与身份感知恢复

状态：**COMPLETED／PASS（用户授权的TASK_OWNED分轮范围）**。本操作手册描述已实现并验收的隔离运行、灰度回退、容量与身份感知恢复流程；全阶段状态见 [实施台账 Phase 13](task-ledger.md)、[Phase 13 验收](validation/phase-13.md)与[最终聚合摘要](../../../local-identity-sso-phase13-qa/final-acceptance-summary.json)。按用户最新“跳过已成功阶段、只重测失败阶段”的要求，复用R20完整steady、R21协议／灰度，以及R22已通过的300秒burst／140项恢复／第一版UI restored browser247。R22与R23原initial FAIL247均保留；当前第二版UI经55项前端门禁后，R24只补失败initial，已PASS343／notification PASS，未重跑已通过容量、协议、灰度、恢复或restored。最终16图、source／runtime、完整扫描与全资产清理分别PASS，不宣称同轮连续35分钟通过。Phase 1～12 已交付；[Phase 12](validation/phase-12.md)的221协议、214浏览器记录和单共享库restricted identity健康不能替代本阶段的新证据。

## 依据与证据等级

当前 helper 接线合同为 [QA13 integration-context](../../../local-identity-sso-phase13-qa/integration-context.md)。以下四份QA12文档是归档时点的准备材料，保留原来的NOT_STARTED／NOT_ACCEPTANCE及旧源码行号，不将其当作本阶段运行证据：

| 准备材料 | 当前用途 |
| --- | --- |
| [只读总体准备](../../../local-identity-sso-phase12-qa/phase13-readonly-preparation.md) | 容量、灰度、恢复、SQL与消费者审查的起点。 |
| [负载接入准备](../../../local-identity-sso-phase12-qa/phase13-load-implementation-preparation.md) | 协议actor、主／辅助计数和真实BFF接线依据。 |
| [恢复实施准备](../../../local-identity-sso-phase12-qa/phase13-recovery-implementation-preparation.md) | 双库、库外材料、外部高水位与deny-only恢复顺序。 |
| [通知实施准备](../../../local-identity-sso-phase12-qa/phase13-notification-implementation-preparation.md) | 精确员工通知、逐条proof与恢复前后两轮检查。 |

当前源码说明实现边界，纯测试、AST、镜像与PKI只证明各自准备。已执行的多轮integration preflight、失败与清理保存在 [preflight历史](../../../local-identity-sso-phase13-qa/preflight-failure-history.md)，原始receipt逐轮保留。preflight均属 **NOT_ACCEPTANCE**；当前正式验收按 [分轮证据准则](../../../local-identity-sso-phase13-qa/split-run-evidence-policy.md)复用已通过的准确子项、重测失败burst并完成剩余链路，不使用fake clock、辅助请求或补发来满足真实容量目标。以下各历史轮次记录中的同轮1800＋300秒要求属于当时执行计划，原FAIL与原始receipt保持，不作为用户调整后再次重复成功steady的依据。

第12轮的complete_recovery子报告已实际PASS，任务RPO为7.732秒、RTO为30.831882秒；但整轮driver仍FAIL，initial browser在152个记录检查后因精确通知proof故障探针绕过注入而失败，restored browser未完成，并另保留失败的进程输出扫描记录。第13轮debug已取得initial browser PASS341／notification43、restored browser PASS247及recovery PASS140，RPO7.214秒／RTO30.680196秒；但 [整轮driver](../../../local-identity-sso-phase13-qa/preflight-round-13.json)仍因`PHASE13_PROCESS_STOP_OR_SCAN_FAILED`失败，不能提升为全阶段通过。第14轮FULL已在容量开始前由主执行者主动SIGTERM，在canary阶段结束并完成清理；原因与原结果见 [独立中止说明](../../../local-identity-sso-phase13-qa/full-round-14-abort-reason.json)。测量边界在任何容量开始前修正，并通过54项Linux回归。

### FULL 第15轮失败与第16轮调整

第15轮任务 `01a17167-2e8d-77b0-ab50-10c983292061` 的 [原始安全聚合回执](../../../local-identity-sso-phase13-qa/full-round-15-failed.json)为 `FAILURE_PARTIAL`。driver 在 `ACTUAL_35_MINUTE_CAPACITY_REHEARSAL` 阶段以 `PHASE13_CAPACITY_FAILED` 结束，容量子报告类别为 `driverdeficit`。221项协议全部通过，指定RP的10／50／100%灰度及显式回退通过，100个独立sid／grant的bootstrap也通过；这些结果不能抵消容量失败。主负载只留下steady阶段部分记录，未完成1800＋300秒，也未进入本轮两次browser与完整身份感知双库恢复。逐项计数、部分分位数和清理事实另存 [R15失败分析](../../../local-identity-sso-phase13-qa/full-round-15-failure-analysis.md)及 [机器可读摘要](../../../local-identity-sso-phase13-qa/full-round-15-failure-analysis.json)。原始失败回执保留，不以摘要覆盖。

| R15 steady主操作 | scheduled／sent／completed／success | driverdeficit | 成功服务延迟p95／p99，毫秒 |
| --- | --- | --- | --- |
| login | 362／357／355／354 | 5；另有1次429 | 1192.635855／1253.365900 |
| code | 723／705／704／704 | 18 | 906.040856／971.459027 |
| refresh | 723／682／682／682 | 41 | 370.434216／419.068845 |
| introspect | 18060／17733／17730／17730 | 327 | 183.253023／221.566124 |

上述部分样本的login、code及introspect p95已经超过各自候选门槛，且主计数不齐；不能当作完整容量分位数或成功结果。最后一份周期资源样本是主计时后180.030秒，driver另记含bootstrap共273.861580秒；后者不能当作实际主负载时长。六个进程的停止与完整stdout／stderr扫描均PASS，两任务库、portal／notification／deployment角色已删除，container hosts已恢复；这些回执仅覆盖所列container资产，不扩写为Windows bridge等外部资产的清理证明。

第16轮在另一fresh任务重新执行，identity改用与其生产Dockerfile相同的优化release profile，准确路径为 `/runtime/target/release/saas-identity`，SHA256为 `3534e49b983cb56b5375a59d9eea5bf44b680bac91b90fabd35e4f6d895c8dd3`；[构建回执](../../../local-identity-sso-phase13-qa/identity-release-build-receipt.json)记录源码及lockfile未改、原debug artifact未改，构建成功本身不构成容量通过。任务网关将TLS握手从监听accept路径移到每个连接的worker，避免新连接在accept阶段串行握手；保持严格SNI／Host与固定上游规则，并以 [14项纯回归](../../../local-identity-sso-phase13-qa/gateway-tls-pure-validation.json)验证接线。R16结果继续失败，不能把这些调整写作唯一根因或已解决结论。

R16仍使用100 actor、1800＋300真实秒、相同主RPS与24000／240000目标、32 worker／in-flight、0.5秒调度lag、锁前开始的3秒绝对预算及原p95／p99门槛；不降低Argon2成本，不扩大密码worker或队列，不更换issuer，不补发凑数。当时要求所有后续browser、恢复、通知、扫描与清理在同轮重新验收；该历史时点状态为 **IN_PROGRESS**，R15不改判PASS，R16实际失败另列如下。

### FULL 第16轮实际失败与诊断边界

第16轮任务 `01a17174-9993-7abc-b702-16d0b2e15bee` 的 [安全聚合回执](../../../local-identity-sso-phase13-qa/full-round-16-failed.json)为 `FAILURE_PARTIAL`，driver仍在 `ACTUAL_35_MINUTE_CAPACITY_REHEARSAL` 以 `PHASE13_CAPACITY_FAILED` 结束，容量类别 `driverdeficit`。主执行者记录在主计时约120秒主动SIGTERM停止；原回执最后周期资源样本为120.022秒，含bootstrap总耗时139.313013秒。完整1800＋300秒未完成，不能将这次短轮作为容量验收。完整摘要见 [R16失败分析](../../../local-identity-sso-phase13-qa/full-round-16-failure-analysis.md)及 [机器可读投影](../../../local-identity-sso-phase13-qa/full-round-16-failure-analysis.json)。

| R16 steady主操作 | scheduled／sent／completed／success | driverdeficit／protocol | 成功服务延迟p95／p99，毫秒 |
| --- | --- | --- | --- |
| login | 243／203／199／199 | 40／0 | 668.223797／727.561329 |
| code | 485／344／340／340 | 141／0 | 1442.288846／1601.485701 |
| refresh | 485／257／257／257 | 228／0 | 571.828644／626.720303 |
| introspect | 12121／9518／9498／9472 | 2603／26 | 316.068466／376.839211 |

R16的release identity与candidate在进程receipt中均命中 `3534e49b…c8dd3`。bootstrap login p95为85.334121毫秒，steady主login p95为668.223797毫秒，相比R15部分样本有所降低；但是code／refresh p95、introspect p95与p99仍超过原门槛，主计数严重不齐，introspect另有26条 `protocol`／状态0记录，不能当作实际HTTP状态码或授权拒绝。steady各HTTP端点p95为315.851611～373.026690毫秒，跨多个端点的延迟上升仍需定位；这些不同短轮的部分样本不能证明release或TLS调整的独立因果效果，也不能据此认定单一根因。

本轮221协议、15灰度检查及100独立sid／grant bootstrap均通过；六个进程停止／完整输出扫描PASS，两任务库与portal／notification／deployment角色删除、container hosts恢复均为true。burst、initial／restored browser及完整身份感知恢复未执行；不借前轮结果补齐。该轮失败后的诊断范围为严格TLS路径与driver资源／排队瓶颈，正常CA／SNI／Host验证与真实在线authority保持；诊断、工具修复及R17实际启动的证据另列如下，不降低安全验证、缩短时长或增加重放来制造通过。该历史时点Phase 13为 **IN_PROGRESS**。

### FULL 第17轮启动与工具修复证据

第17轮fresh FULL任务UUID为 `01a1718b-f219-7c01-af6a-03177ea58091`，原主执行会话为91744，现已停止FAIL，结果见下节。该轮使用release identity `3534e49b983cb56b5375a59d9eea5bf44b680bac91b90fabd35e4f6d895c8dd3`，SCRAM路径和既定协议／成员边界保持。准确运行源码封存于 [R17 source snapshot](../../../local-identity-sso-phase13-qa/full-round-17-source-snapshot.json)：609文件，content-set SHA256 `cd5d1c842acad7b3f4c1df5b700f3a0d5e9fe1ce375fc15f143d0c4aa454cce2`。该snapshot只绑定R17输入，不代表完整35分钟、browser、恢复或通知通过；R15／R16的失败也不改判。

R17开始前完成两项测试工具修复，产品源码与安全／容量门槛未据此放宽：

- 严格TLS gateway移到全新独立进程，通过唯一继承的私有socketpair FD接收控制，两个输出流均纳入完整capture；将负载client与TLS server从同一Python进程中隔离，继续保持新TCP／TLS连接、task CA／hostname／SNI／Host验证。独立worker、控制通道及接线的 [21项gateway回归](../../../local-identity-sso-phase13-qa/gateway-worker-pure-validation.json)与 [第06次55项load回归](../../../local-identity-sso-phase13-qa/load-pure-validation-round06.json)均通过，receipt明确为helper／纯诊断，不是live验收。
- 输出scanner将“每个日志块遍历全部历史known credentials”改为12字节suffix索引筛选，再做完整凭据字节匹配；完整流digest／字节数、通用patterns、64KiB tail和晚注册秘密的回查语义保持。该修改没有取消known-secret检查，也没有把suffix命中直接当作完整凭据命中。[13项scanner回归](../../../local-identity-sso-phase13-qa/capture-scanner-validation.json)含500组确定性随机oracle比较、边界与碰撞／晚注册／流完整性检查，全部通过；测试在network-none环境运行，未访问产品服务、私有fixture或原始日志。

[严格TLS合成诊断](../../../local-identity-sso-phase13-qa/tls-echo-microbenchmark-results.json)以32 workers、每变体320次全新严格TLS／TCP请求访问临时echo listener，不接产品端口。同进程变体约172.98～181.19 RPS，独立进程变体约362.35～367.30 RPS；该对照支持工具隔离的选择，不证明生产或R17容量达标，也不认定R16只有这一原因。[scanner合成性能回执](../../../local-identity-sso-phase13-qa/capture-scanner-performance.json)在100000个合成known值、69632字节文本上记录新索引五次扫描约10.819489～11.220961毫秒；旧遍历基线中位数约794.709／1145.211毫秒，来源及形状限制见scanner验证回执。这些是未做CPU隔离的工具算法测量，不能代替实际主操作分位数或证明日志已在R17完整扫描通过。

R17原计划为100 actor、1800＋300真实秒、相同主RPS、24000认证及240000 authority主目标、32 worker／in-flight、0.5秒最大调度lag和锁前起算3秒绝对操作预算，并沿用原p95／p99门槛。该轮实际未完成完整35分钟及后续initial／restored两轮browser、身份感知双库与库外材料恢复、通知，不能因工具回归和部分延迟改善而封账。该历史时点Phase 13为 **IN_PROGRESS**。

### FULL 第17轮实际失败与第18轮负载发生器资源计划

[R17安全聚合回执](../../../local-identity-sso-phase13-qa/full-round-17-failed.json)为 `FAILURE_PARTIAL`，driver在 `ACTUAL_35_MINUTE_CAPACITY_REHEARSAL` 返回 `PHASE13_CAPACITY_FAILED`，容量类别 `driverdeficit`。主执行者记录主计时约214秒停止；原回执最后周期资源样本为210.010秒，含bootstrap总耗时228.865244秒，完整1800＋300秒未完成。详细安全历史见 [R17失败分析](../../../local-identity-sso-phase13-qa/full-round-17-failure-analysis.md)和 [机器可读摘要](../../../local-identity-sso-phase13-qa/full-round-17-failure-analysis.json)。

| R17 steady主操作 | scheduled／sent／completed／success | driverdeficit | 成功服务延迟p95／p99，毫秒 |
| --- | --- | --- | --- |
| login | 428／428／428／428 | 0 | 172.631864／263.239438 |
| code | 856／856／856／856 | 0 | 166.738049／247.848171 |
| refresh | 856／856／856／856 | 0 | 79.051141／125.630385 |
| introspect | 21387／21369／21367／21367 | 18 | 22.138765／49.553675 |

这些部分服务延迟明显低于原门槛，主操作的rate／dependency／protocol等非预期类别均为0；但是introspect有18个scheduled未sent，另有2个sent在部分采集时没有completed，不能当作成功。仅因这18条driverdeficit也必须保留FAIL，不能将其从计划删除、补发或用其它操作成功数抵消；部分分位数不构成35分钟达标证据。login／code／refresh在本轮已记录的slot四计数相等，原始计数保持。

221协议、15灰度检查和100独立sid／grant bootstrap通过。六个产品／candidate／bootstrap进程停止与两流完整扫描PASS；独立gateway worker也实际退出0、graceful RPC close为true、forced termination为false，stdout／stderr均0字节、reader完整且credential pattern分类为空。两个任务库与portal／notification／deployment角色已删除，container hosts已恢复。上述清理只归原回执所列资产，不扩写为Windows bridge或浏览器临时材料清理；本轮burst、两轮browser、完整身份感知恢复与恢复后通知未执行。

R18将**负载发生器**从32 workers／32 in-flight调整为与100 actor相配的100／100，并增加有界的漏发原因分类与计数，以定位和区分driver端不足。它调整测试发生器可并行执行的资源，不扩大identity密码worker、密码队列或任何产品运行配置，不把R17及此前失败改判。100 actor、1800＋300真实秒、相同主RPS及24000／240000目标、0.5秒最大schedule lag、锁前起算3秒绝对预算、每actor串行锁与原p95／p99门槛全部保持；任何漏发或非预期错误仍FAIL，不补发追数。R18的准确输入冻结与实际启动见下节，最终同轮两次browser、恢复、通知、扫描、source／binary复验和清理仍待实际receipt，该历史时点状态为 **IN_PROGRESS**。

### FULL 第18轮启动证据

R18 fresh FULL任务UUID为 `01a17197-a691-7dcd-8087-8d534fe67368`，原主执行会话2424，现已自动异常结束FAIL，终态安全聚合见下节。该轮输入为 [R18 source snapshot](../../../local-identity-sso-phase13-qa/full-round-18-source-snapshot.json)中的609文件，content-set SHA256 `139abb88a721fccc9b6983697c5202e37ccf09eeafe7cca02b1a51f44c83e721`，不再视为活跃运行。release identity使用 `3534e49b983cb56b5375a59d9eea5bf44b680bac91b90fabd35e4f6d895c8dd3`，SCRAM及独立严格TLS gateway、完整输出scanner边界保持。

本轮发生器资源是100 workers／100 in-flight，5类scheduler deficit为 `admission_saturation`、`schedule_late`、`worker_start_late`、`window_expired`、`unfinished`，各类计数及最多64条固定安全字段样本用于定位漏发／未完成slot；这不是重试、catch-up或主请求失败分类的替代。[独立R18工具回执](../../../local-identity-sso-phase13-qa/load-generator-r18-validation.json)记录Linux **59／59项纯回归通过**、exit0，原始完整测试日志以SHA256绑定；该回执明确 `PURE_PREPARATION_ONLY_NOT_LIVE_ACCEPTANCE`、未启动产品服务、未改产品binary／主计数／延迟门槛，也未改R17结果。R18实际未完成35分钟及所有后续browser／恢复／通知，不能把启动、snapshot或59项工具测试改写为验收PASS。该历史时点Phase 13为 **IN_PROGRESS**。

### FULL 第18轮自动异常结束与后续修复依据

[R18终态安全聚合](../../../local-identity-sso-phase13-qa/full-round-18-failed.json)仍为 `FAILURE_PARTIAL`，driver在 `ACTUAL_35_MINUTE_CAPACITY_REHEARSAL` 返回 `PHASE13_CAPACITY_FAILED`，容量子报告类别为 `driverdeficit`。主执行者确认这次在主计时约754.5秒自动异常结束，没有主动kill；原回执最后周期资源样本为750.071秒，含bootstrap总耗时770.468769秒。详细历史另存 [R18失败分析](../../../local-identity-sso-phase13-qa/full-round-18-failure-analysis.md)和 [机器可读摘要](../../../local-identity-sso-phase13-qa/full-round-18-failure-analysis.json)。

| R18 steady主操作 | scheduled／sent／completed／success | sent未completed | 成功服务延迟p95／p99，毫秒 |
| --- | --- | --- | --- |
| login | 1510／1510／1508／1508 | 2 | 98.439091／117.053552 |
| code | 3019／3019／3016／3016 | 3 | 103.662305／124.583268 |
| refresh | 3019／3019／3016／3016 | 3 | 50.399006／65.924051 |
| introspect | 75451／75451／75405／75405 | 46 | 14.266817／19.708954 |

已completed主操作全部success，主分类的rate／dependency／protocol／capacity／securityneg／unclassified503及各行driverdeficit均为0；五类scheduler deficit也全0、样本为空。但尚有54个sent没有completed，完整时长与最终主目标未完成，依然FAIL。不得将54条直接填成成功或改填`unfinished`来重建原报告，也不能将终态笼统的`driverdeficit`解读为已记录了54条调度漏发。

R18当时的load通用异常处理只保留笼统失败类别，内层异常类／frames没有保存；外层driver的`RehearsalError`不能证明内部是`ValueError`。后续负载owner已在受控复现中确认等待循环存在“harvest耗时跨过due后，将负余量传给sleep”的确定竞态，并准备修正余量计算、补充安全异常类和文件／行／函数frames；这属于后来独立复现，不能据此宣称R18当次已直接证明同一种异常。原回执与证据缺口保留。

本轮221协议、15灰度检查通过；六个进程停止与完整两流扫描PASS，独立gateway退出0、graceful RPC close=true、forced termination=false、两流0字节且扫描完整。两任务数据库、portal／notification／deployment角色删除及container hosts恢复均为true，清理范围不扩写为Windows外部资产。burst、两轮browser、完整身份感知双库恢复与恢复后通知未执行。R18原FAIL及内层异常未知的事实保持；随后修复与R19实际启动证据如下。该历史时点Phase 13为 **IN_PROGRESS**。

### FULL 第19轮启动与等待竞态回归证据

R19 fresh FULL任务UUID为 `01a171b0-87e2-7f9d-a3b5-228ce0070515`，原主执行会话46886，现已FAIL并由主执行者SIGTERM停止。[R19 source snapshot](../../../local-identity-sso-phase13-qa/full-round-19-source-snapshot.json)绑定该轮609文件，content-set SHA256为 `66dba13490fd979db12fa2308f4b2766ece2e3e484cdbfadeeb8881defc655e6`；该轮继续release identity `3534e49b983cb56b5375a59d9eea5bf44b680bac91b90fabd35e4f6d895c8dd3`、100 workers／100 in-flight、100 actor及完整原容量合同，实际未完成完整35分钟及随后browser／恢复／通知。

[两项旧负sleep复现](../../../local-identity-sso-phase13-qa/load-generator-r19-negative-sleep-reproduction.json)以原R18负载源码分别覆盖per-slot和observation-window等待路径，受控得到预期`ValueError`／exit1；scope明确不是对R18现场异常的归因。修复在这两个等待点均于harvest之后重新计算remaining，并只在remaining严格大于0时sleep，避免检查与等待之间跨越due／结束窗口产生负值。另新增安全异常诊断：仅输出allowlist异常类，未知类归`UnexpectedError`，最多12条经过自有源码过滤的basename／line／function frames；不输出异常text／args、URL、源码行或完整路径。R18当次内层类仍未知，原FAIL不改判。

[R19独立Linux工具回执](../../../local-identity-sso-phase13-qa/load-generator-r19-validation.json)为 **69／69项纯回归PASS**，其中含上述两项防竞态回归；[五种完整计划压力模式回执](../../../local-identity-sso-phase13-qa/load-generator-r19-long-plan-stress.json)覆盖exact fake clock、floating boundary、harvest跨due、clock-read推进及刻意产生五类deficit的模式，每模式保持264000个不可变主slot／模拟2100秒，验证分类及等待边界。故障模式的测试PASS表示其预期错误／不足被正确记录，不表示该模式所有业务slot都成功。整个纯测试suite真实计算耗时42.199秒、runner耗时44.7094264秒；模拟2100秒与这些真实计算时间均不是产品真实35分钟容量。回执明确无网络／产品服务、无真实sleep、无产品binary或密码池／数据库参数变化，不补发／重放、不改主计数与延迟门槛。

R19原验收要求为1800＋300真实秒、原各操作RPS和24000认证／240000 authority目标、0.5秒最大lag、锁前起算3秒绝对预算、每actor串行锁及原p95／p99门槛，随后同task完成两轮browser、身份感知双库及库外材料恢复、通知、全部输出扫描、source／binary复验与清理。该轮失败及后续计划如下，不能用69项纯测试或压力模式补齐未完成的真实链路，该历史时点状态为 **IN_PROGRESS**。

### FULL 第19轮调度迟发失败与第20轮待验计划

[R19终态安全聚合](../../../local-identity-sso-phase13-qa/full-round-19-failed.json)为 `FAILURE_PARTIAL`，driver在 `ACTUAL_35_MINUTE_CAPACITY_REHEARSAL` 返回 `PHASE13_CAPACITY_FAILED`，capacity为`driverdeficit`。第一批不足对应due1411.500秒：4条 `worker_start_late` 的lag为537.008061～539.619971毫秒、记录in-flight为1～4；另3条 `schedule_late` 的lag为507.412639～527.364918毫秒。7条均超过原0.5秒门槛，`admission_saturation`为0，不能写成已发生100 in-flight饱和。完整安全分析见 [R19失败分析](../../../local-identity-sso-phase13-qa/full-round-19-failure-analysis.md)及 [JSON摘要](../../../local-identity-sso-phase13-qa/full-round-19-failure-analysis.json)。

| R19 steady主操作 | scheduled／sent／completed／success | driverdeficit | 成功服务延迟p95／p99，毫秒 |
| --- | --- | --- | --- |
| login | 3259／3258／3258／3258 | 1 | 104.722233／141.782419 |
| code | 6518／6517／6516／6516 | 1 | 110.077868／147.817760 |
| refresh | 6518／6517／6517／6517 | 1 | 54.125934／81.672178 |
| introspect | 162931／162927／162926／162926 | 4 | 14.707553／22.157161 |

已completed主操作全部success，非预期rate／dependency／protocol／capacity／securityneg／unclassified503均为0，但7个slot未sent，另有2个sent在部分采集时未completed。已记录的低服务分位数不能抵消缺发或替代完整35分钟。原回执最后周期资源样本为1620.230秒，含bootstrap总耗时1647.178680秒；实际主终止时点没有单列，不将任一值当作完整主负载持续时间，也不据模糊的“163x秒”补造时点。

主执行者在发现原门槛已失败后SIGTERM停止，`failure_diagnostic.class=UnexpectedError`及 `phase13-driver.py:terminate` 帧属于该停止路径，不另判为此前自动崩溃。R19没有同轮GC事件时序证据，不能断言1411.5秒处的暂停由GC导致。221协议、15灰度检查通过；六个进程与独立gateway的退出／两流扫描PASS，两库及portal／notification／deployment角色删除、container hosts恢复均为true。burst、两轮browser、完整身份感知恢复及恢复后通知未执行，原FAIL保留。

R19结束时的R20准备记录如下：scanner suffix索引桶从单元素set改为bytes、仅碰撞时使用tuple以减少容器对象；当时协调记录其16项回归已通过，10轮×每轮100000合成凭据的旧／新GC与内存对照尚在生成独立证据。该合成比较即使通过也不能倒推R19历史GC原因。负载侧只新增观测GC telemetry，不调整GC开关／阈值、SLO、产品配置或原容量合同。R20原计划是在准确输入冻结后fresh执行100／100发生器、100 actor、1800＋300真实秒、原各操作RPS／24000＋240000目标、0.5秒lag／3秒绝对预算／每actor锁及原延迟门槛，并完成同轮后续全链；当时尚无R20完成或验收结论。该时点Windows bridge会话为67159，不计作container清理证明，私有材料helper尚未apply并拟按当时预计20任务动态清单重新核验clearance与plan SHA。此段保留原计划，R20实际终态及之后的用户范围调整见下节；不将历史会话或目录数量当作当前值。

### FULL 第20轮持续通过、突发饱和与连接复用验证

[R20 原始安全聚合](../../../local-identity-sso-phase13-qa/full-round-20-failed.json)及[失败分析](../../../local-identity-sso-phase13-qa/full-round-20-failure-analysis.md)保留该轮 FAIL。实际完成 1800 秒持续阶段：3600 login、7200 code、7200 refresh、180000 introspect，scheduled／sent／completed／success 逐项相等，五类调度不足和非成功类别全部为零；持续阶段各延迟门槛通过。进入双倍速率突发后，100 个 in-flight 槽饱和，最终保留 8553 个 admission_saturation；其他四类调度不足为零。主执行者在发现固定计数已不可能满足后 SIGTERM 停止，本轮不计完整容量、浏览器或双库恢复验收。

首个突发饱和在 1801.350 秒，早于突发首个 gen2 GC 事件；本轮 GC 观测最大 360.228225 毫秒，不能将全部失败归因于 GC。多个轻端点同时变慢，独立 TLS 网关在相邻突发采样区间约占用 0.91 个逻辑核。客户端逐请求新建 TLS、网关逐请求关闭连接成为优先检验的测试传输瓶颈；服务／数据库争用尚需与实际对照区分，不据此更改产品池或安全锁。

R20之后的工具改动采用每 Transport／hostname 独立的严格 TLS HTTP/1.1 连接，客户端 5 秒 idle 后发送前主动退役，网关 10 秒有界 idle；每请求仍重验路由、来源、故障 marker、凭据和权威状态，不缓存授权。连接复用重绑定当前绝对 deadline，完整消费响应才允许复用；EOF、超时、部分响应、协议错误或关闭指令清理连接，任何已经发送的 code／refresh／写请求均不自动重发。恢复、兼容重启与最终清理前显式关闭全部 owned 连接。R21最初按原100 actor／100 worker／100 in-flight、1800＋300真实秒和原各RPS／主计数计划启动，随后按下节用户要求中止重复steady。固定burst重测仍保持锁前3秒预算、0.5秒调度上限与全部延迟门槛。纯回归和短时严格TLS对照仅用于工具验证，不代替实际失败阶段重测或剩余安全验收。

### R21用户优化中止与R22定向执行

用户最新要求跳过已成功阶段、只测试失败阶段。主执行者据此停止R21重复steady，依据见 [R21中止原因](../../../local-identity-sso-phase13-qa/full-round-21-abort-reason.json)；[R21原始安全聚合](../../../local-identity-sso-phase13-qa/full-round-21-aborted.json)中的driver FAIL与SIGTERM停止记录保持原样，单列 `USER_APPROVED_REUSE_PASSED_STAGES_AND_RETEST_FAILED_BURST_ONLY`，不将中止改判为完整容量PASS，也不作为新的产品失败。该轮中止前约634秒只证明其已completed操作成功、无调度不足，停止时sent未completed的差额原样保留。

| 当前验收面 | 证据来源 | 已通过事实／剩余执行 |
| --- | --- | --- |
| steady1800秒 | [R20安全聚合](../../../local-identity-sso-phase13-qa/full-round-20-failed.json)，task `01a171de-4a23-7c08-a0cc-0d29e47bafc3` | login3600／code7200／refresh7200／introspect180000四计数逐项相等、全success、零deficit，原steady延迟门槛通过；仅复用此子项，R20失败burst不复用。 |
| 协议／故障221项 | [R21安全聚合](../../../local-identity-sso-phase13-qa/full-round-21-aborted.json)，task `01a1720e-4dd8-73af-a166-fa7c21547fd7` | protocol子报告PASS221，使用当前keepalive工具；本次不重复完整协议流程。 |
| 灰度／显式回退15项 | 同R21 | canary子报告PASS15，实际10／50／100%分流、candidate503、无自动fallback、显式set0及两RP新登录；rollback3.336875秒。 |
| 失败burst及后续交付 | R22 TARGETED，task `01a17221-6d7e-7a61-8db8-b4008279d633` | 固定300秒burst PASS、恢复PASS140、第一版UI restored247／notification.after PASS，driver与22个进程及gateway退出／完整扫描PASS；原initial FAIL247／通知PASS分别保留。R23第一版UI补测仍FAIL247，第二版UI的失败initial最终由R24 PASS343补齐。 |

R20的[610文件输入](../../../local-identity-sso-phase13-qa/full-round-20-source-snapshot.json)seal为 `3be44cbc4695bc642f397f78d1bba851dd52302de106212d07f06b28bbbbaf92`，R21的[610文件输入](../../../local-identity-sso-phase13-qa/full-round-21-source-snapshot.json)seal为 `9aa848268cf6605b8b81f83e8a4d317952ad4cc052c2590ebcbd6eb548a17805`。两者selected差异仅六个QA文件：gateway、protocol client、load、load tests、driver与root helper tests；[normal binary／源码复核](../../../local-identity-sso-phase13-qa/binary-manifest-resume.json)所记录的570个既有产品源码文件字节及normal binary pins保持，identity继续使用release SHA `3534e49b983cb56b5375a59d9eea5bf44b680bac91b90fabd35e4f6d895c8dd3`。旧close transport与新keepalive transport不同，不能宣称两轮完整输入相同或相同100个sid连续跨越两段负载。

R22初始输入见 [targeted-round-22-source-snapshot.json](../../../local-identity-sso-phase13-qa/targeted-round-22-source-snapshot.json)：610文件，seal `56126203f935343810aa8a14e17607bc05597beb88fb77d9a74d2f545fba0fb5`；原driver会话87658已退出，bridge由主执行者独立管理。它以 `TARGETED_BURST_AND_REMAINING_STAGES`运行，不重测已有steady／221协议／canary及共享平台库基线restore。driver先验证R20／R21安全聚合的准确SHA、task与通过子项，将跳过项记为 `NOT_EXECUTED`／`USER_REQUEST_REUSE_PRIOR_PASS`并附原始来源；这不伪造该轮重新执行PASS，也不把debug预检升级为验收。其UI修复后输入和实际终态另列如下。

分轮容量分别判门槛：R20真实1800秒已覆盖18000认证＋180000authority，R22真实300秒已覆盖6000认证＋60000authority；各阶段原分位数、失败分类、四计数及资源单列。24000认证／240000authority只表示两个验证阶段的计划覆盖合计，不表述为同轮连续35分钟、同一进程264000槽或新transport全程35分钟已通过。协议／灰度保留R21准确task，完整恢复／第一版UI restored保留R22准确task，第二版UI initial保留R24准确task。R23 FAIL不改判，最终逐图与全局清理回执现已齐备，完成结论见最终聚合摘要。

### R22实际通过项、两版UI修复与R24 initial补测

[R22容量原报告](../../../local-identity-sso-phase13-qa/targeted-round-22-capacity.json)及[独立复核](../../../local-identity-sso-phase13-qa/targeted-round-22-capacity-review.md)已PASS：固定300秒，实际主观察／收尾300.136467秒，100actor／100worker／100in-flight，主login1200／code2400／refresh2400／introspect60000的四计数逐项一致，五类deficit和全部非预期失败为零，peak_inflight56。成功服务p95／p99（毫秒）分别为login142.073674／178.600577、code283.635366／371.348956、refresh117.148257／150.580547、introspect48.161455／64.043412，原门槛全部通过；bootstrap／辅助请求和HTTP计量独立，不加入主6000／60000。

[R22 initial browser](../../../local-identity-sso-phase13-qa/targeted-round-22-browser-initial.json)保留 **FAIL247**，通知before模块已PASS；在 `CROSS_TAB_CUSTOMER_SELECTION`，另一页切换成功后，原页出现session200→profile401、另一路profile200，旧controller将一次profile401永久映射signed-out，等待超时。没有相邻logout HTTP证据，不能归为已证明的异步注销；后端对version、context／access能力与lease的严格最终检查可以拒绝已陈旧的读快照，客户端需要安全地重新读取当前状态，不能放松权威校验。

[第一版前端修复与门禁](../../../local-identity-sso-phase13-qa/portal-profile-reconciliation-validation.json)仅改portal controller与其测试：profile读401／409时最多额外读取一次当前session及对应严格profile，仍使用同epoch／abort；持续401清空身份，403／503／协议错误不重试，不重发选择、登录、注销或OAuth refresh。旧实现4项定向回归先FAIL；第一版43／43 Node、typecheck、lint和production build全部PASS，用于R22 restored与R23 initial。补测未重跑已通过容量，未改后端／DB。

[第一版前端差异清单](../../../local-identity-sso-phase13-qa/targeted-round-22-frontend-change.json)与[restored输入snapshot](../../../local-identity-sso-phase13-qa/targeted-round-22-restored-source-snapshot.json)将R22输入从 `56126203f935343810aa8a14e17607bc05597beb88fb77d9a74d2f545fba0fb5`更新为 `2a5ae6dc49dc461dbb966f519be64108e17dc4583413360561fe82b621d44296`，两份均610文件。第一版 `portal/frontend/src/controller.ts` SHA为 `1ccd1b83691adbfff04bb3c9a17bec4e164fb2dad917adf32fdec9d8fbba7f6d`，测试SHA为 `b7195dee67874063d53a7e82955e3fa5898f89cb252f9abd87f38f91eb62c98b`。当前第二版两文件SHA及其R24输入另列下文；[最终runtime/source复核](../../../local-identity-sso-phase13-qa/final-runtime-source-verification.json)证明568个既有产品文件、全部Rust／migration／lock及7个后端二进制pins未改，准确登记2项UI变更。上节570文件不变属于UI修复前的R20→R21比较，当前应明确568不变＋2UI变化。

[R22完整安全聚合](../../../local-identity-sso-phase13-qa/targeted-round-22-complete.json)为COMPLETE，driverPASS，22个实际产品／恢复进程与独立gateway均退出0、binary pins与完整流扫描PASS；两库、portal／notification／deployment roles已删除，container hosts恢复。完整身份感知双库与库外材料恢复PASS140，实测RPO5.805秒、RTO21.443394秒，scope为 `FAULT_TO_OLD_DENIALS_AND_FRESH_DUAL_RP_BUSINESS`；备份后业务marker丢失和认证状态主动废弃单列。[第一版UI restored browser](../../../local-identity-sso-phase13-qa/targeted-round-22-browser-restored.json) **PASS247**，包含cross-tab选择／资料精确绑定、局部与全局注销，notification.after PASS；[launcher](../../../local-identity-sso-phase13-qa/targeted-round-22-browser-launch-restored.json) exit0、完整fixture-known与pattern扫描无命中。该恢复／restored未在当前第二版controller下重跑，driverPASS及restored成功不回写initial失败，也不代表外部bridge／TEMP／历史私有资产全部清理完成。

R23只执行 `INITIAL_BROWSER_RETEST_ONLY`：task `01a17238-0b65-7be9-a9d9-11a90c776027`，[610文件snapshot](../../../local-identity-sso-phase13-qa/targeted-round-23-source-snapshot.json) seal `8a55e452be8a1c6bed210e327250538b01bd91c14165acbc84896ca3524299e5`。[initial原报告](../../../local-identity-sso-phase13-qa/targeted-round-23-browser-initial.json)仍为 **FAIL247**，notification PASS；cross-tab切换成功后，原页profile401触发第一版session核对，但session503令UI unavailable并等待超时。缺少该请求的lease／refresh_uncertain现场，不认定已证明的后端根因或注销；backend现有严格lease／最终proof检查保持不变。

[第二版前端门禁](../../../local-identity-sso-phase13-qa/portal-profile-lease-reconciliation-validation.json)与[独立只读复核](../../../local-identity-sso-phase13-qa/diagnostics/portal-profile-lease-reconciliation-independent-review.json)仅扩展已由profile401／409触发的核对分支：对session503最多3次读，100／200ms可取消等待，500ms单调时钟后续派发预算；晚到session不追加profile，第二次profile失败不再核对，epoch／abort及完整资料绑定保留。普通首次session503仍立即unavailable，持续503清空个人数据，不自动登录／bootstrap或重放写操作。旧实现5项定向回归先FAIL；当前55／55 Node、typecheck、lint、production build及独立复核PASS。controller SHA为 `43f9840bf525e086f1e7125f658f11818e60df968a8197832f40a906b226eb9f`，测试SHA为 `e7b6c18b7d64ca1eaac51e4018b526b65bb43c904095040256f7cd29485eb9cc`。

R24 task `01a1724a-75de-71d4-9fbf-7b9dfd440a53`仅补失败initial。[安全聚合](../../../local-identity-sso-phase13-qa/targeted-round-24-complete.json) SHA为 `3826bb3a5930c8178b41558887126dc27023cc14d1560664a96ed157f6098f58`；[610文件snapshot](../../../local-identity-sso-phase13-qa/targeted-round-24-source-snapshot.json) seal为 `886feadc9893dbd447628db1996ed96981a37892e50c2c0c26e2c90a84a0f7c6`。[第二版UI initial browser](../../../local-identity-sso-phase13-qa/targeted-round-24-browser-initial.json) **PASS343**、notification.before PASS、pageerrors0、8PNG；[launcher](../../../local-identity-sso-phase13-qa/targeted-round-24-browser-launch-initial.json) exit0、完整扫描PASS。driver的5个实际进程退出／扫描、当次两DB／roles清理PASS。容量、协议、灰度、完整恢复与restored均引用准确已PASS来源，未在R24重新执行。R22第一版UI restored247与R24第二版UI initial343是独立范围，不拼成当前controller恢复重跑或同轮全链。

R24网关记录实际authority503为0，另有3次task-owned authority-unavailable故障注入响应；initial343成功不证明自然lease503已在现场复现并确定修复，有界核对路径另由12项新增回归覆盖，R23具体根因仍未知。浏览器沿用任务CA的Edge `ignoreHTTPSErrors`，Node及两个BFF校验任务CA和准确hostname，不构成生产DNS／OS信任或发布验收。

[最终消费者source复核](../../../local-identity-sso-phase13-qa/final-consumer-source-verification.json)重新枚举806文件，803不变＋3项准确变更：第二版portal controller及早已在R22／R23／R24封存的恢复ops README／recovery.py；portal测试文件不在此806选集。417项既有合同未重跑，第二版UI另以55项门禁验证；runtime568不变＋2UI／7pins独立复核。[最终source snapshot](../../../local-identity-sso-phase13-qa/final-source-snapshot.json)610项，seal与R24完全一致。[最终视觉复核](../../../local-identity-sso-phase13-qa/final-visual-review.json)16图PASS，准确分为R24第二版initial8图及R22第一版restored8图。执行指针见 [execution-state.json](../../../local-identity-sso-phase13-qa/execution-state.json)；全局清理独立回执见下节最终聚合。

## 隔离拓扑与授权范围

公开地址固定为 `https://auth.saas.test`、`https://console.saas.test`、`https://customer.saas.test`，issuer／subject／精确callback与schema保持稳定。identity、platform、portal及独立notification按各自受限角色运行；认证和平台共用平台数据库，门户有独立会话数据库。candidate identity在固定38086使用同一已验证normal binary，不能把test executable、旧无类型实现或另一个issuer作为回退目标。

任务管理连接仍限定loopback及准确cluster；**所有identity／platform／portal／notification运行角色与恢复后的运行连接均走固定容器地址 `172.20.0.2` 的SCRAM认证路径**。driver用每个准确运行角色分别证明正确密码可连、错误密码被拒，恢复后再验证轮换前密码拒绝。loopback的既有trust行为只属于任务管理连接，不能拿它证明运行密码轮换有效；本流程不修改共享`pg_hba.conf`，receipt记录`shared_hba_changed=false`。该地址属于当前固定测试集群，不能复制成生产网络假设。

所有账户、两RP、客户resource client、通知EMPLOYEE application、三种内部service用途及临时角色都由任务bootstrap明确登记。测试成员生命周期开关不代表生产开放；首次客户业务仍为本人四字段资料，客户通知与员工管理／目录不扩权。不得触碰预览UUID专项数据库、改历史migration或SDK archive／vendor。

[QA13工具镜像](../../../local-identity-sso-phase13-qa/tools-image-result.json)在已验收PG17 client镜像上仅加入Debian Argon2 CLI，产品依赖未变；[独立PKI准备](../../../local-identity-sso-phase13-qa/pki-result.json)为task CA／CA:false leaf、三个准确SAN与独立BFF keyrings。准备PASS不代表live或生产DNS／OS信任已验收。

## 运行入口与证据归档

从平台仓根目录运行 [run-live.ps1](../../../local-identity-sso-phase13-qa/run-live.ps1)。开始前核对工具镜像、正常二进制manifest、任务集群／卷／端口、私有环境文件及PKI pins；保留产品默认关闭开关，只由任务clean environment显式启用fixture。共享正常二进制在运行期禁止重建；同一集群的live driver串行执行。

```powershell
# R24已用此入口补测失败initial；容量、协议、灰度、恢复与restored引用准确已PASS证据。
.\local-identity-sso-phase13-qa\run-live.ps1 -InitialBrowserRetest

# 定向burst模式保留；R22已通过该固定300秒及其恢复／restored流程。
.\local-identity-sso-phase13-qa\run-live.ps1 -BurstRetest

# 完整模式保留：固定100 actor、1800＋300真实秒，再执行两轮浏览器与恢复。
.\local-identity-sso-phase13-qa\run-live.ps1

# 接线预检：明确跳过完整容量，始终NOT_ACCEPTANCE。
.\local-identity-sso-phase13-qa\run-live.ps1 -IntegrationPreflight

# 仅预检可用：额外跳过完整protocol，用真实管理API创建恢复所需receipt。
.\local-identity-sso-phase13-qa\run-live.ps1 -IntegrationPreflight -DebugSkipProtocol
```

`-InitialBrowserRetest`与其余三个模式互斥；R23／R24只执行失败initial browser／notification及当次必要拓扑、退出扫描和清理，不初始化负载actor或重跑任何已PASS容量／协议／灰度／恢复／restored。driver核验R20／R21及R22完整聚合的准确task／SHA与通过子项，将已复用范围另记 `NOT_EXECUTED`／`USER_REQUEST_REUSE_PRIOR_PASS`，不能伪写本轮重新执行成功。

`-BurstRetest`与两个预检及initial-only开关互斥，唯一的定向负载是固定300秒原burst；不提供任意缩时／RPS入口。该模式协议、canary与基线restore保留 `NOT_EXECUTED`安全报告，引用R21准确task／SHA与原PASS；steady引用R20。定向driver仍通过真实 `POST /api/oidc/clients`创建本task、当前OIDC admin service归属的secret-bearing receipt；需要实际执行恢复时，它防止 `RECOVERY_EXISTING_OIDC_CREATION_RECEIPT_REQUIRED`缺前置。secret只进内存与已注册的完整输出scanner，不复制旧task receipt或直接插造结果。

`-DebugSkipProtocol`单独使用立即拒绝。debug分支将protocol记为`NOT_EXECUTED`／`DEBUG_INTEGRATION_ONLY`，同样通过真实管理API建立恢复fixture所需receipt；它不代替221项协议／故障／密钥检查。integration preflight的capacity明确`NOT_EXECUTED`，仍不是容量验收。完整模式保持原1800＋300计划；两种定向模式的 `full_capacity_and_driver_checks_passed`均保持false，分别记录实际定向链完成字段，不冒称同轮35分钟。`--hold-seconds 1800`是每轮等待浏览器的期限，与固定负载计时分开。R24补测已完成，以上入口为流程说明，不再重复启动已通过阶段。

driver输出`PHASE13_BACKEND_READY`后，按 [copy-browser-fixture.ps1](../../../local-identity-sso-phase13-qa/copy-browser-fixture.ps1)核task UUID和两个公共证书SHA，将私有fixture只放Windows任务TEMP。再用 [run-browser.ps1](../../../local-identity-sso-phase13-qa/run-browser.ps1)启动真实浏览器。可复用公开PKI pins及runner生成的task ID：

```powershell
$taskId = (Get-Content -LiteralPath .\local-identity-sso-phase13-qa\active-task-id.txt -Raw).Trim()
$taskPki = Get-Content -LiteralPath .\local-identity-sso-phase13-qa\pki-result.json -Raw | ConvertFrom-Json
$taskCopy = .\local-identity-sso-phase13-qa\copy-browser-fixture.ps1 -TaskId $taskId -CertificateSha256 $taskPki.leaf_sha256 -CaCertificateSha256 $taskPki.ca_sha256 | ConvertFrom-Json
.\local-identity-sso-phase13-qa\run-browser.ps1 -FixturePath $taskCopy.fixturePath
```

完整／burst模式执行实际恢复时，等待`PHASE13_RESTORED_BACKEND_READY`后，以同task ID和pins再次copy并追加`-AfterRecovery`，再执行第二轮browser runner；更新后的fixture含新受控凭据／运行配置，不能沿用initial私有副本。R23／R24 initial-only在初轮finished后直接进入最终证据／清理，不等待恢复事件或再跑restored。WSL loopback bridge只覆盖38081～38084并使用已核对的任务容器地址；Node代理精确拒绝不匹配的Host／SNI，记录Origin是否存在及是否与该host一致，并将Origin转发Rust后端执行实际验证，不能将这些观测字段描述成Node已统一拒绝非法Origin。Socket.IO专用upgrade路径另按其精确Host／SNI／Origin规则处理；浏览器API只能到对应真实后端。bridge／Windows临时资产由主执行者独立清理，不能以driver的container-only hosts回执代替。

每轮立即将driver、browser、launcher、process／protocol／canary／capacity／recovery安全报告另名归档；initial与restored截图分别保存，不覆盖失败轮次。`browser-finished`只是控制面握手，即使它存在或driver某子报告PASS，也须检查实际browser结果。用 [collect-live-evidence.py](../../../local-identity-sso-phase13-qa/collect-live-evidence.py)只收固定公开JSON，失败仅在driver确为FAIL时允许`--allow-partial-failure`；缺少报告保留missing清单，不读私有fixture补充。

## 执行顺序与最终操作要求

1. 创建带准确task／cluster／comment绑定的两个新库，唯一canonical job执行平台schema；完成bootstrap、三service登记、两RP与通知application，再交接六角色、portal及notification必要权限。
2. 启动独立normal进程，核验受限角色、SCRAM与当前pins。当前定向模式复用R21协议／故障／key与共享平台库基线restore的实际PASS，仍用真实管理API创建当前恢复必需receipt并保留本次过程完整扫描；完整模式照常执行这些检查。
3. 当前定向模式复用R21准确两RP的10／50／100%分流、candidate503与显式回退，不启动新candidate；本次gateway保持baseline。完整模式再实际执行灰度及回退并保留当次计数。
4. R23／R24复用R20完整1800秒steady与R22已通过300秒burst，不再执行容量。容量两阶段原主／辅助计数、失败类别、分位数、资源分别登记；不合并成同轮35分钟。需要burst模式时才初始化100独立actor并运行固定300秒，R22已实际完成。
5. R23 initial FAIL247原样保留；第二版UI的R24 initial双RP与独立通知生命周期已PASS343／通知PASS，安全报告及8截图另存。R22原FAIL247同样保留，不覆盖为新PASS；各task私有通知见证由root按最终清理回执收口。
6. R22已实际完成的恢复流程为：另取仍有效旧载体，协调停止所有进程、捕获双库与库外材料checkpoint；恢复源服务后施加post-backup安全收窄，封存备份外witness，再执行源故障／原名双库restore、fence与deny-only、凭据轮换及受控reset。R23／R24引用其140项PASS及原task，不再次执行。
7. R22已实际通过旧密码／旧载体拒绝、新SSO及两RP业务，再以第一版UI及更新私有配置完成restored247／notification.after；该结果不靠已注销载体作唯一恢复证明。R23／R24引用此原始成功报告，当前第二版UI未重跑restored。
8. 最终source、SQL、活动消费者与16图审查已完成，分轮来源、计数／原始时点／限制分别登记；各任务进程、任务库／roles／hosts、bridge／监听与两侧私有材料均完成精确清理。最终摘要聚合五项标准为用户授权TASK_OWNED分轮范围PASS，保留历史FAIL与各自UI版本，不外推生产验收。

## 请求灰度与显式回退

[canary-rehearsal.py](../../../local-identity-sso-phase13-qa/canary-rehearsal.py)和 [gateway](../../../local-identity-sso-phase13-qa/auth-gateway.py)只接受本task准确的console／customer两个RP client ID。gateway在auth host解析唯一一致的query／Basic／form client标识，以100个eligible请求为周期，配置10／50／100%到candidate；其余请求走baseline。登记的tenant是fixture上下文，gateway不按tenant字段路由，因此本轮是**指定task RP请求比例**演练，不声明生产多租户扩围、用户比例或真实版本升级。

每档计划各50次code exchange与50次authority检查，记录eligible／baseline／candidate差值且准确相加；100%时baseline须为零，两个真实BFF均须重新登录及选精确成员。candidate实际SIGSTOP后，受影响请求必须得到真实503，配置仍维持100%，不得用自动路由切换掩盖故障。随后显式set_canary(...,0)，以全新操作重验两个RP及BFF，不重试未知code／refresh；finally恢复并停止candidate。

回退种类是 `SAME_VERIFIED_BINARY_ROUTING_CONFIGURATION`：同normal binary、同身份感知schema和数据库、同issuer与callback的路由配置回退。它不证明跨版本二进制降级，更不能降到user＋org二元唯一、缺member_type／代际证明、客户混员工授权或无在线撤销的旧版本。当前按用户要求复用R21实际15项PASS、rollback3.336875秒及candidate停止证据，准确标记原task，不称本轮重新执行；preflight的局部结果不替换该来源。以后真实灰度操作仍须记录其自身配置、比例、故障与回退事实。

## 容量计划与已通过的分轮结果

[load-rehearsal.py](../../../local-identity-sso-phase13-qa/load-rehearsal.py)的完整 `run`保留1800秒steady＋300秒burst；`run_burst_retest`只包含固定300秒、原速率的burst，R22已通过。R23／R24 initial-only不调用负载模块。[protocol-client.py](../../../local-identity-sso-phase13-qa/protocol-client.py)在容量轮通过真实SSO／PKCE／consent／code／refresh／introspect建立100独立actor，50 EMPLOYEE、50 CUSTOMER。各actor独立CookieJar、sid、grant、refresh和锁，使用固定两RP，不要求新增100个账号。重复登录可能留下仍有效旧sid／grant，100指受控actor／当前会话，不伪称数据库历史session恰好100，也不称R20与R22使用相同sid。

`bootstrap_actors`在主计时前先串行完成100次真实密码登录，再用最多8个worker对100个actor各做一次有界refresh及一次当前proof在线内省。它验证100个不同sid／grant保持原绑定，全部新proof有效，最早access到期距离计时开始仍至少30秒。串行登录耗时可能使最早access自然到期，报告单列该数量；用原refresh正常轮换恢复当前access，不能重试未知refresh。初始化后头部不足30秒、任何失败或绑定改变即停止，不以先启动主计时、增加TTL假造初始化成功。100 login、100 refresh和100proof均归bootstrap／辅助，不计主目标。

| 主操作 | R20已通过steady RPS／1800秒 | R22已通过burst RPS／300秒 | 两阶段计划覆盖总数 |
| --- | --- | --- | --- |
| 密码login | 2／3,600 | 4／1,200 | 4,800 |
| code exchange | 4／7,200 | 8／2,400 | 9,600 |
| refresh | 4／7,200 | 8／2,400 | 9,600 |
| online introspect | 100／180,000 | 200／60,000 | 240,000 |
| login＋code＋refresh合计 | 10／18,000 | 20／6,000 | 24,000 |

bootstrap、preauth、authorize／consent、辅助code／在线确认、旧grant撤销及额外业务探针单列；不得加入主目标。R22只有6000认证＋60000authority主slot；R20已通过steady的18000／180000另列原来源。每阶段各主操作scheduled／sent／completed／success须与本阶段固定slot目标逐项相等；漏发、调度过迟、deadline、saturation、错误成功DTO、依赖失败或安全拒绝不计正向成功，也不catch-up／重放凑数。24000／240000仅为两个阶段计划覆盖合计，不生成不存在的同轮全程计数。每actor锁保护一次性refresh及当前原始source tuple；新login后新建flow，不把新Cookie与旧sid强行配对。

隔离目标沿用 [Q6建议](open-question-recommendations.md)：introspect p95≤100ms／p99≤300ms；code／refresh p95≤500ms／p99≤1000ms；密码认证单独p95≤1000ms。R15～17的发生器是32 workers／32 in-flight，R18起为100／100，R22固定burst继续100／100，仍有界排队并保持每actor串行锁，产品密码worker／队列不变。记录各操作／阶段成功和全尝试分位数、错误类别、调度不足、CPU／RSS、数据库／Argon2参数及实际观察时长；R20steady和R22burst分别判门槛，不合并两段样本制造未测的全程p95／p99。总期限3秒／连接上限1秒的既有权威预算继续作为边界；实际helper与端点计量据最终receipt核对，不以source设定冒称达到吞吐。

操作延迟范围须随报告解释：服务延迟从取得每actor锁后开始，锁前驱动等待另记`actor_lock_wait`；login服务延迟只计preauth＋password HTTP，不把随后辅助grant安装计入password分位数。code计authorize／consent／exchange及本地RS256验证、辅助在线proof，refresh计轮换／RS256验证及辅助在线proof，introspect是一次严格actor绑定的主在线检查。`schedule_lag`与`completion_lag`另外保留，后者包含调度、actor排队、服务及适用的辅助工作，不能用服务分位数隐藏等待。

**3秒绝对操作预算在取得actor锁前开始**，由锁等待、服务及同操作的辅助流程共同消耗；嵌套操作继承更早deadline，不重置预算。锁后预算已经耗尽时按失败／driverdeficit记录，后续HTTP不得获新的3秒；不能只报告锁后服务快就把排队超时算成功。原始302／303在验证issuer／state／精确callback前不直接分类成功；主操作最终成功与辅助HTTP状态分开。0.5秒最大调度lag、相同主请求RPS／数量及既定p95／p99门槛均未放宽。该计量修正在第14轮容量尚未开始时完成，[第04次54项Linux回归](../../../local-identity-sso-phase13-qa/load-pure-validation-round04.json)固定了边界；R15～17按此口径实测失败，原始部分计数保留。R18只将有界发生器资源32／32调为100／100并增加漏发细分类，绝对预算及判失败规则保持。任何缺发、非预期错误、计数不齐或候选延迟门禁未达仍记FAIL，不补发凑数。

上述steady与burst是TASK_OWNED容量候选。当前按用户要求分轮覆盖两阶段，不宣称同轮连续35分钟；这些结果都不证明滚动30天99.9%、生产容量或任意生产SLO。integration preflight跳过负载仍为NOT_ACCEPTANCE；本次定向重测有明确的历史通过项来源与真实300秒burst要求，不把其余功能通过换成容量通过。

## 完整双库与库外材料恢复

实现位置为 [恢复编排](../../../local-identity-sso-phase13-qa/recovery-rehearsal.py)、[恢复原语](../../ops/identity-recovery/recovery.py)及 [外部witness](../../ops/identity-recovery/witness.py)。第12／13轮分别获得过恢复子报告PASS140，但第12轮browser不完整、第13轮进程输出扫描失败，原调试结果不填充正式恢复。当前已有 [R22实际恢复PASS140](../../../local-identity-sso-phase13-qa/targeted-round-22-complete.json)、第一版UI restored247与完整退出／扫描证据；R23／R24按用户要求复用这些已通过项，不再执行恢复。当前第二版UI只取得R24 initial343现场结果，函数名或纯测试不作为恢复实测。

checkpoint在全部normal／notification／candidate停止、连接核验后捕获platform与portal两个数据库，保留各自owner／comment／ACL、canonical history及安全数据投影，连同签名keyset、历史公钥、两个BFF keyring、task CA／leaf和运行配置封入经过认证的加密材料。数据库dump是协调静止窗口，不声明跨库分布式原子快照。原名还原前先认证／解密全部必需材料，并核准确task、cluster、数据库名／marker和零活动连接；两个restore及库外材料均须比对，不能以Phase12单库restricted health代替。

checkpoint后实际制造密码／账号多代际、成员disable／delete／replace、client／application收窄、角色／data-grant撤销及业务标记变化。最新account highwater、deny inventory和业务标记写入**备份外**HMAC链式witness，准确绑定task／cluster／checkpoint／两个数据库，独立持有minimum sequence。验证签名有效但旧sequence的replay、内容篡改、缺失／不完整witness失败；HMAC本身不能阻止整套状态回退。本任务外部本地目录不等于异地持久存储或灾备冗余。

恢复完成但接流前必须：

- 所有还原账号先禁用并使旧password hash不可用；account generation提升到还原值与备份外highwater之上，检查溢出／精确单行与实际trigger行为，不能仅加固定常数。
- SSO、OIDC grant／access／refresh／code／interaction、平台session／ticket／psr_／legacy OAuth／device／app、两BFF session／flow／browser、客户opaque和pending account／OIDC命令全部失效，旧命令不自动重发。
- deny-only对账只缩小权限：备份后禁用／删除／替换的原成员不复活，不重建备份后新增的替换成员；client／app集合及redirect／有效期不扩大，角色、菜单、data-grant等撤销不回填。actor／owner业务账号字段不借此变成成员授权。
- 轮换身份service、两RP及其他还原secret-bearing登记、legacy confidential secret、受限DB runtime密码、BFF keyrings与签名配置；只对checkpoint与最新inventory均允许且未RETIRE的明确任务账号设置全新Argon2id密码，其余账号保持禁用。
- wrapper验证备份外最新witness与authenticated start receipt后才重启受限拓扑；还原ready marker本身不允许接流。关闭仅为新task恢复fixture明确开启的legacy capture窗口，不因authority故障开启fallback。

成员类型交集使用schema允许的固定顺序EMPLOYEE、CUSTOMER；角色／URL材料绑定准确task与阶段前缀，portal运行角色是phase13登记，不能沿用phase11名称。恢复后每个运行角色继续使用SCRAM路径，必须实测旧数据库密码拒绝；只在loopback trust连接上观察“可连”不能算轮换成功。旧第09轮折叠的数据库类别仍保持原样，源码修复与实际失败原因分别登记。

旧载体覆盖必须取“原本仍可使用”的实际值，恢复后逐项做HTTP拒绝及新登录正例；已过期和已被早先注销的值单独分类。旧ID Token离线签名可能仍成立，验收是它不能当业务Bearer且原在线grant／session已失效，不能声称数据库撤销了签名数学有效性。秘密见证只留私有内存／TEMP，公共报告只保存固定类别、字段、计数、时间、字节与digest。

任务RPO候选≤24h，以checkpoint、故障前最后业务commit／witness、实际恢复和丢失标记分别计算；认证状态主动废弃单列。任务RTO候选≤4h，从明确停止接流／源故障到旧密码／载体拒绝、新SSO及两RP精确业务可用，包含对账、reset与权限核验；restored浏览器／通知另保存真实闭环及时间。单次年轻备份不证明生产15min／60min、持续WAL／PITR、异地回读、每12h调度或固定保留周期。

第12轮实测RPO7.732秒来自故障时点与checkpoint时点差，报告同时保留最后business commit及marker确实丢失、认证状态主动废弃；RTO30.831882秒的准确scope是`FAULT_TO_OLD_DENIALS_AND_FRESH_DUAL_RP_BUSINESS`。第13轮对应值为7.214秒／30.680196秒，随后restored browser另获PASS247；两轮RTO计量scope均不包含整个浏览器轮次及最终输出／清理审查。这些值只适用于各自task，不证明生产PITR或可用性目标，也不能覆盖driver整轮FAIL。

## 通知恢复前后两轮

[notification-rehearsal.py](../../../local-identity-sso-phase13-qa/notification-rehearsal.py)负责明确EMPLOYEE application、专用只读role及安全控制；[notification-check.mjs](../../../local-identity-sso-phase13-qa/notification-check.mjs)负责真实Socket.IO与恢复前／后私有见证。平台HTTP持久收件箱是事实，通知进程不签发／refresh、不拿平台签名key、不扩大portal数据库权限。

两轮分别验证真实新通知、身份精确收件、同账号另一身份不继承、member disable／delete／replace及authority不可用后不投递、显式重连和新通知恢复；客户首版不开放通知。公开authenticate输入仍仅{token}、authenticated五字段／notification十二字段，私有context-proof与原recipient member／type／version逐条相交；旧空proof只归档。恢复重启进程后旧socket会断开，必须测旧token向新进程重连被拒与新登录实际新投递，不能声称socket跨进程存活或emit代表已读。

精确proof故障同时落在两个任务代理：Linux gateway使用任务私有`notification-proof-unavailable` marker阻断真实通知进程→console的 `GET /api/internal/notifications/context-proof`；Windows Node代理只在同console host／准确path镜像该故障，使浏览器探针也看到503。control回执须匹配准确operation／request ID、`enabled`布尔状态及 `scope=CONSOLE_EXACT_NOTIFICATION_CONTEXT_PROOF_ONLY`，验证通过后才更新Node代理状态，解除时对称恢复。不能只改一层代理、伪造另一路响应或靠短时间无事件判定生效。

真实通知进程的metrics是必需证据：先记录验证失败及hint计数，注入后用正常HTTP publication产生有效hint，等待hint接收目标达到、pending hints与active partitions排空并稳定；要求实际validation failures增加、目标连接关闭且禁止notification ID未投递。解除故障后以真实新登录、新socket ACK和新publication证明恢复，故障hint不能变成自动补投。浏览器探针503本身不够，只有metrics增加也不代表用户已读。

成员撤权使用新认证员工socket且ACK剩余至少4秒，计量`CONTROL_DISPATCH_TO_SOCKET_CLOSED`实际延迟；不能用自然到期替代撤权。重新启用员工后，callback先reconcile当前BFF状态，再capture session fence与支持的PLATFORM logout，然后正常统一登录及明确选成员，避免直接 `/auth/session`已清除旧context而coordinator仍持旧version。客户重新登录不全局注销员工，员工局部退出也不影响同账号客户RP。

initial与restored使用不同通知publication标签，私有旧Bearer见证固定在task TEMP、schema／task绑定，默认不序列化。第12轮initial故障探针失败保持原receipt；第13轮debug的 [initial browser](../../../local-identity-sso-phase13-qa/preflight-13-browser-initial.json)已PASS341并包含notification43检查，[restored browser](../../../local-identity-sso-phase13-qa/preflight-13-browser-restored.json)已PASS247。该轮因进程输出扫描FAIL且未完成当时完整35分钟计划，原FAIL保留，不填充当前交付。R22 initial通知PASS而browser整体FAIL247；其原task私有见证已用于实际恢复后的旧Bearer拒绝及新通知投递，第一版UI restored247／notification.after PASS。R23 initial FAIL247／notification PASS保留；R24第二版UI initial343／notification.before PASS使用自己的task／publication／私有见证，不伪称跨task socket或见证属于同一恢复周期，也不称第二版UI已重跑恢复后通知。

## 最终SQL与活动消费者审查

逐命中审查以当前生产源码为准，不用rg零命中代替分类；历史migration、cfg(test) fixture与业务ChatRoom／二元业务幂等保留原义。每项记录具体路径／行号、成员定位、局部／全局范围、现有回归引用和本阶段实际证据，最终表见 [验收记录](validation/phase-13.md)。已有测试名仅证明当前回归存在，执行结果按各阶段receipt引用，不能冒称本轮重跑。

重点核INSERT／UPDATE／DELETE／ON CONFLICT、JOIN／LIMIT1／fetch_one／first、角色祖先／superadmin／data grant、全载体partial proof、trigger／窄projection／bridge、默认广播／recipient和缓存namespace。当前 [消费者与SQL审查](phase13-consumer-audit.md)已核**29个生产区域含memberships字面量的文件**，其中含配置／路由名而非全为SQL；排除cfg(test)后5生产writer文件／11 SQL位置，两个成员ON CONFLICT均三元，无新生产SQL／接口安全阻断。账号级RETIRE按接受时(id,identityVersion)快照删原关系，无type过滤属于明确全账号操作。精确路径与R1～R10回归来源已入验收表；旧凭据live拒绝及最终source seal独立核对。

八仓当前消费者只读审查已完成、无新安全阻断：活动SDK3.0.0是REPOSITORY_CANDIDATE／private package，不冒称外部已发布；MicroProps2.0／Context1.0／Notification1.0独立。stores默认legacy、work-orders默认combined且失败不切路径；既有confidential MACHINE／LEGACY_MIXED client_credentials兼容，新bootstrap只准MACHINE＋空成员集合。平台psr_ refresh在/oauth/token独立分支，legacy OAuth与标准OIDC /oidc/token refresh分开；CLI只消费显式Bearer，playground是legacy OAuth演练消费者。精确路径／行号及既有回归已入验收表，源码／fixture不当本阶段运行或生产流量证据。

本阶段活动消费者实际执行 **417项合同测试全部通过：233项Node＋184项Rust普通测试**，另通过SDK20生成发行物及平台固定SDK校验，详见 [活动审查](phase13-consumer-audit.md)、[Node回执](../../../local-identity-sso-phase13-qa/consumer-node-contracts.json)与 [Rust／逐命中／源码回执](../../../local-identity-sso-phase13-qa/consumer-source-sql-audit.json)。[最终只读复核](../../../local-identity-sso-phase13-qa/final-consumer-source-verification.json)重新枚举806=803不变＋3准确变更，并保留417项既有通过的原始来源、明确未重跑；第二版UI另有55项门禁，恢复ops变更另有其封存与验证范围。ignored PostgreSQL测试不计入此数，也不将417项单测算成417个端到端场景或称其覆盖第二版UI／恢复ops变化。固定SDK／vendor字节按该最终复核独立登记。

## 正式验收聚合与完成边界

driver的单独exit0或恢复子报告PASS不代表全阶段完成。按用户最新授权，主执行者将R20已通过steady、R21协议／灰度／基线restore、R22 burst／完整恢复／第一版UI restored与R24第二版UI initial343逐项聚合；每项附原task、source／binary／配置、原始receipt与实际时点，原R22／R23 initial FAIL不改判。R23／R24复用项的 `NOT_EXECUTED`只有在准确历史来源通过校验时才作为复用记录，不能独立视为通过。R24 driver本身仍明确phase13_acceptance=false；[最终验收摘要](../../../local-identity-sso-phase13-qa/final-acceptance-summary.json)在独立逐图、source／runtime与当前root清理回执齐备后，才将下列标准聚合为用户授权TASK_OWNED分轮范围PASS，不回写driver原字段。

| 检查面 | 必需证据与停止条件 |
| --- | --- |
| 配置／权限／网络 | canonical20迁移history、准确两库marker与owner、身份／平台／portal／notification最小权限；运行SCRAM正确密码可连／错误密码拒绝及未改HBA。不得因缺权限扩大运行角色。 |
| 协议／灰度 | 复用R21协议221／canary15的原PASS、两个准确RP实际10／50／100%计数、candidate真实503、不自动fallback、显式set0与两RP新登录，保留rollback3.336875秒及原task。新任务仍执行所需fixture真实注册。非预期登录失败、回调错误、错误成功DTO或授权扩大立即记FAIL；不换issuer／schema。 |
| 容量 | R20完整1800秒steady与R22真实300秒burst已通过：R22 login1200／code2400／refresh2400／authority60000四计数逐项相等，零deficit／非预期错误，分位数／资源／队列完整且达到原门槛。分列两轮，24000／240000仅是阶段覆盖合计，不宣称同轮35分钟；R23／R24不重复容量。不得缩时、补发或合并分位数代替原阶段通过。 |
| 恢复 | R22原名双库restore、owner／comment／ACL／catalog／安全投影、库外材料、witness／start receipt、deny-only、旧密码／全载体拒绝与新两RP业务PASS140；RPO5.805s、RTO21.443394s及丢失分类完整，RTO准确scope不含整轮browser／最终清理。R23／R24引用这些原始事实，第二版UI未重跑恢复。 |
| 浏览器／通知 | R22／R23原initial FAIL247及各自notification.before PASS保留；R22第一版UI restored247／notification.after与launcher／完整扫描PASS。R24第二版UI initial343／notification.before、pageerrors0、launcher0、8PNG与当次完整扫描PASS；最终16图（各8图）及公开载荷边界已复核。browser-finished、R13调试PASS或部分计数不覆盖FAIL。 |
| 输出／源码／清理 | 原复用证据和本次过程输出分别完成known-secret／pattern扫描，历史任一capture未完成或命中仍保留原FAIL。R22的22进程＋gateway、R24的5进程＋gateway退出／扫描与各自两库／roles／hosts清理PASS；最终source按806=803+3及runtime568+2／7pins准确登记，610 seal与R24相同，2UI变化分别按43／55项门禁及各轮source追溯。当前root clearance、bridge／监听／进程0与Linux／Windows私有材料精确清理PASS，详见下列独立回执。 |

[最终只读runtime观察](../../../local-identity-sso-phase13-qa/diagnostics/final-runtime-observation.json)记录观察时任务DB／roles／connections0、24个runtime目录均有匹配terminal公开归档；它不签发清理许可。[root current clearance](../../../local-identity-sso-phase13-qa/diagnostics/final-current-clearance.json)随后PASS：任务进程与Windows443／38081～38084监听0，bridge22394实际exit0；R24 driver5进程及gateway退出／完整扫描PASS、launcher0，两Vite子进程close且全部stream reader完整。

[Linux精确清理应用](../../../local-identity-sso-phase13-qa/diagnostics/final-private-cleanup-applied.json)按plan SHA `2d58b8cda0809b71675b020cbd09ce66acb079a8ec6e8abe7047a76e825146f9`删除24任务目录及Phase13 PKI；[删除后验证](../../../local-identity-sso-phase13-qa/final-linux-private-cleanup-verification.json)任务目录0、PKI不存在，未读取私有内容或触及Phase09材料。[Windows清理](../../../local-identity-sso-phase13-qa/final-windows-private-cleanup.json)删除8任务目录及1 PKI目录内15文件，reparse0、无残留；两侧均无安全擦除声明。PG在观察后09:06:48 UTC正常exit0、非OOM，通用network-container dry-run因此失败；后续network-none文件清理成功，未重启DB、未重新执行已通过阶段，不将首次dry-run记为PASS。

公开收集器的JSON安全扫描与原进程完整流扫描分别记录：公开JSON无秘密不等于原进程流扫描已通过。失败输出只保存字节／SHA／固定类别，不能为排查抑制类别重新暴露原始SQL／环境／Cookie／token。历史失败轮次、用户优化中止和定向重测分开链接；同轮完整35分钟要求已由用户调整为成功项复用与失败项重测，原安全、延迟、准确计数和恢复标准未放宽。全部五项现已在用户授权分轮范围闭合，生产SLO／PITR与其他部署未由本任务验收。

第12／13轮的`platform-recovery-5`进程虽然exit0且binary未变，完整stdout扫描仍记录`known_runtime_credential`；第13轮stdout计314,011字节。源码审查发现旧`remember_tree`把任意`code`字段视为秘密，可能把公开application code或错误标签加入known集合；修复已收紧到真实凭据上下文及形状，[52项Linux回归](../../../local-identity-sso-phase13-qa/load-pure-validation-round03.json)直接覆盖公开code／appcode与日志相交，同时证明真实秘密仍登记。**原来命中的确切值没有保留，不能据源码发现就把旧扫描判为已证明的误报**。两轮原FAIL及固定类别保留。第14轮容量前主动中止，第15～19轮FULL容量实际失败，其已执行进程扫描与清理结果只归各自轮次，不替代尚未执行的browser／完整恢复。第15轮历史输入冻结记录为608文件、content-set SHA256 `252bbaca97393fb8c7639b7ddde227e321134ad9571b8b700d6bd4b4005f9ca8`；已结束的R17冻结为 [609文件snapshot](../../../local-identity-sso-phase13-qa/full-round-17-source-snapshot.json)及 `cd5d1c842acad7b3f4c1df5b700f3a0d5e9fe1ce375fc15f143d0c4aa454cce2`；已结束的R18输入为 [609文件snapshot](../../../local-identity-sso-phase13-qa/full-round-18-source-snapshot.json)及 `139abb88a721fccc9b6983697c5202e37ccf09eeafe7cca02b1a51f44c83e721`；已结束的R19输入为 [609文件snapshot](../../../local-identity-sso-phase13-qa/full-round-19-source-snapshot.json)及 `66dba13490fd979db12fa2308f4b2766ece2e3e484cdbfadeeb8881defc655e6`，均为各自历史输入。R20／R21／R22／R23／R24的610文件snapshot分别以上节准确seal归档，不能相互覆盖；最终仍核对本次结束后的相同selected字节、binary与发行物。known-secret检查与公开JSON安全规则未放宽；后续R24及最终复验／清理已按上列独立回执闭合，本任务最终聚合不修改上述历史输入或FAIL。

## 退役N/A与交付边界

本身份TASK_OWNED新库未导入历史账号／凭据，没有已登记待退役历史客户端；历史迁移／转换窗口／停用日期仅在这一清单范围记N/A，不推断其他环境没有旧部署。活动SDK、app派生、机器grant、CLI、playground、stores／work-orders legacy／combined及通知仍受支持，不能随历史N/A一起退役。真实对象以后出现时，必须另记owner、准确client／实际流量、迁移验证、最长凭据寿命、观察窗口及撤销方式后再决议。

当前文档同步反映SDK／工程事实、用户分轮授权、实际通过子项及两版门户UI修复／补测；固定发行字节、历史准备和原失败回执不改写。任务来源授权覆盖全部阶段，本次完成Phase13收口；正式创建与生产默认关闭，生产域名、人员owner、SLO／RPO／PITR不凭测试假设编造。R24 initial、最终视觉／source／扫描、全局清理回执及五项聚合现已完成，整体为用户授权TASK_OWNED分轮范围COMPLETED／PASS；活动legacy支持与本人资料首个客户业务范围不变。
