# Phase 13 灰度、容量与身份感知恢复

状态：**IN_PROGRESS**。本操作手册描述当前已实现的隔离运行、灰度回退、容量与身份感知恢复流程；全阶段完成状态以 [实施台账 Phase 13](task-ledger.md)及 [Phase 13 验收](validation/phase-13.md)的最终聚合证据为准。正式fresh全链尚未据完整receipt验收，不因preflight局部通过提前封账。Phase 1～12 已交付；[Phase 12](validation/phase-12.md)的221协议、214浏览器记录和单共享库restricted identity健康不能替代本阶段的新证据。

## 依据与证据等级

当前 helper 接线合同为 [QA13 integration-context](../../../local-identity-sso-phase13-qa/integration-context.md)。以下四份QA12文档是归档时点的准备材料，保留原来的NOT_STARTED／NOT_ACCEPTANCE及旧源码行号，不将其当作本阶段运行证据：

| 准备材料 | 当前用途 |
| --- | --- |
| [只读总体准备](../../../local-identity-sso-phase12-qa/phase13-readonly-preparation.md) | 容量、灰度、恢复、SQL与消费者审查的起点。 |
| [负载接入准备](../../../local-identity-sso-phase12-qa/phase13-load-implementation-preparation.md) | 协议actor、主／辅助计数和真实BFF接线依据。 |
| [恢复实施准备](../../../local-identity-sso-phase12-qa/phase13-recovery-implementation-preparation.md) | 双库、库外材料、外部高水位与deny-only恢复顺序。 |
| [通知实施准备](../../../local-identity-sso-phase12-qa/phase13-notification-implementation-preparation.md) | 精确员工通知、逐条proof与恢复前后两轮检查。 |

当前源码说明实现边界，纯测试、AST、镜像与PKI只证明各自准备。已执行的多轮integration preflight、失败与清理保存在 [preflight历史](../../../local-identity-sso-phase13-qa/preflight-failure-history.md)，原始receipt逐轮保留。它们均属 **NOT_ACCEPTANCE**；正式验收须用另一fresh TASK_OWNED任务实际执行完整1800＋300秒，不使用fake clock、缩短时长、辅助请求或此前阶段计数补足。

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

R16仍使用100 actor、1800＋300真实秒、相同主RPS与24000／240000目标、32 worker／in-flight、0.5秒调度lag、锁前开始的3秒绝对预算及原p95／p99门槛；不降低Argon2成本，不扩大密码worker或队列，不更换issuer，不补发凑数。所有后续browser、恢复、通知、扫描与清理必须在同轮重新验收；当前状态继续为 **IN_PROGRESS**，R15不改判PASS，R16实际失败另列如下。

### FULL 第16轮实际失败与诊断边界

第16轮任务 `01a17174-9993-7abc-b702-16d0b2e15bee` 的 [安全聚合回执](../../../local-identity-sso-phase13-qa/full-round-16-failed.json)为 `FAILURE_PARTIAL`，driver仍在 `ACTUAL_35_MINUTE_CAPACITY_REHEARSAL` 以 `PHASE13_CAPACITY_FAILED` 结束，容量类别 `driverdeficit`。主执行者记录在主计时约120秒主动SIGTERM停止；原回执最后周期资源样本为120.022秒，含bootstrap总耗时139.313013秒。完整1800＋300秒未完成，不能将这次短轮作为容量验收。完整摘要见 [R16失败分析](../../../local-identity-sso-phase13-qa/full-round-16-failure-analysis.md)及 [机器可读投影](../../../local-identity-sso-phase13-qa/full-round-16-failure-analysis.json)。

| R16 steady主操作 | scheduled／sent／completed／success | driverdeficit／protocol | 成功服务延迟p95／p99，毫秒 |
| --- | --- | --- | --- |
| login | 243／203／199／199 | 40／0 | 668.223797／727.561329 |
| code | 485／344／340／340 | 141／0 | 1442.288846／1601.485701 |
| refresh | 485／257／257／257 | 228／0 | 571.828644／626.720303 |
| introspect | 12121／9518／9498／9472 | 2603／26 | 316.068466／376.839211 |

R16的release identity与candidate在进程receipt中均命中 `3534e49b…c8dd3`。bootstrap login p95为85.334121毫秒，steady主login p95为668.223797毫秒，相比R15部分样本有所降低；但是code／refresh p95、introspect p95与p99仍超过原门槛，主计数严重不齐，introspect另有26条 `protocol`／状态0记录，不能当作实际HTTP状态码或授权拒绝。steady各HTTP端点p95为315.851611～373.026690毫秒，跨多个端点的延迟上升仍需定位；这些不同短轮的部分样本不能证明release或TLS调整的独立因果效果，也不能据此认定单一根因。

本轮221协议、15灰度检查及100独立sid／grant bootstrap均通过；六个进程停止／完整输出扫描PASS，两任务库与portal／notification／deployment角色删除、container hosts恢复均为true。burst、initial／restored browser及完整身份感知恢复未执行；不借前轮结果补齐。该轮失败后的诊断范围为严格TLS路径与driver资源／排队瓶颈，正常CA／SNI／Host验证与真实在线authority保持；诊断、工具修复及R17实际启动的证据另列如下，不降低安全验证、缩短时长或增加重放来制造通过。Phase 13继续 **IN_PROGRESS**。

### FULL 第17轮启动与工具修复证据

第17轮fresh FULL任务UUID为 `01a1718b-f219-7c01-af6a-03177ea58091`，原主执行会话为91744，现已停止FAIL，结果见下节。该轮使用release identity `3534e49b983cb56b5375a59d9eea5bf44b680bac91b90fabd35e4f6d895c8dd3`，SCRAM路径和既定协议／成员边界保持。准确运行源码封存于 [R17 source snapshot](../../../local-identity-sso-phase13-qa/full-round-17-source-snapshot.json)：609文件，content-set SHA256 `cd5d1c842acad7b3f4c1df5b700f3a0d5e9fe1ce375fc15f143d0c4aa454cce2`。该snapshot只绑定R17输入，不代表完整35分钟、browser、恢复或通知通过；R15／R16的失败也不改判。

R17开始前完成两项测试工具修复，产品源码与安全／容量门槛未据此放宽：

- 严格TLS gateway移到全新独立进程，通过唯一继承的私有socketpair FD接收控制，两个输出流均纳入完整capture；将负载client与TLS server从同一Python进程中隔离，继续保持新TCP／TLS连接、task CA／hostname／SNI／Host验证。独立worker、控制通道及接线的 [21项gateway回归](../../../local-identity-sso-phase13-qa/gateway-worker-pure-validation.json)与 [第06次55项load回归](../../../local-identity-sso-phase13-qa/load-pure-validation-round06.json)均通过，receipt明确为helper／纯诊断，不是live验收。
- 输出scanner将“每个日志块遍历全部历史known credentials”改为12字节suffix索引筛选，再做完整凭据字节匹配；完整流digest／字节数、通用patterns、64KiB tail和晚注册秘密的回查语义保持。该修改没有取消known-secret检查，也没有把suffix命中直接当作完整凭据命中。[13项scanner回归](../../../local-identity-sso-phase13-qa/capture-scanner-validation.json)含500组确定性随机oracle比较、边界与碰撞／晚注册／流完整性检查，全部通过；测试在network-none环境运行，未访问产品服务、私有fixture或原始日志。

[严格TLS合成诊断](../../../local-identity-sso-phase13-qa/tls-echo-microbenchmark-results.json)以32 workers、每变体320次全新严格TLS／TCP请求访问临时echo listener，不接产品端口。同进程变体约172.98～181.19 RPS，独立进程变体约362.35～367.30 RPS；该对照支持工具隔离的选择，不证明生产或R17容量达标，也不认定R16只有这一原因。[scanner合成性能回执](../../../local-identity-sso-phase13-qa/capture-scanner-performance.json)在100000个合成known值、69632字节文本上记录新索引五次扫描约10.819489～11.220961毫秒；旧遍历基线中位数约794.709／1145.211毫秒，来源及形状限制见scanner验证回执。这些是未做CPU隔离的工具算法测量，不能代替实际主操作分位数或证明日志已在R17完整扫描通过。

R17原计划为100 actor、1800＋300真实秒、相同主RPS、24000认证及240000 authority主目标、32 worker／in-flight、0.5秒最大调度lag和锁前起算3秒绝对操作预算，并沿用原p95／p99门槛。该轮实际未完成完整35分钟及后续initial／restored两轮browser、身份感知双库与库外材料恢复、通知，不能因工具回归和部分延迟改善而封账。Phase 13继续 **IN_PROGRESS**。

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

R18将**负载发生器**从32 workers／32 in-flight调整为与100 actor相配的100／100，并增加有界的漏发原因分类与计数，以定位和区分driver端不足。它调整测试发生器可并行执行的资源，不扩大identity密码worker、密码队列或任何产品运行配置，不把R17及此前失败改判。100 actor、1800＋300真实秒、相同主RPS及24000／240000目标、0.5秒最大schedule lag、锁前起算3秒绝对预算、每actor串行锁与原p95／p99门槛全部保持；任何漏发或非预期错误仍FAIL，不补发追数。R18的准确输入冻结与实际启动见下节，最终同轮两次browser、恢复、通知、扫描、source／binary复验和清理仍待实际receipt，当前状态继续 **IN_PROGRESS**。

### FULL 第18轮启动证据

R18 fresh FULL任务UUID为 `01a17197-a691-7dcd-8087-8d534fe67368`，原主执行会话2424，现已自动异常结束FAIL，终态安全聚合见下节。该轮输入为 [R18 source snapshot](../../../local-identity-sso-phase13-qa/full-round-18-source-snapshot.json)中的609文件，content-set SHA256 `139abb88a721fccc9b6983697c5202e37ccf09eeafe7cca02b1a51f44c83e721`，不再视为活跃运行。release identity使用 `3534e49b983cb56b5375a59d9eea5bf44b680bac91b90fabd35e4f6d895c8dd3`，SCRAM及独立严格TLS gateway、完整输出scanner边界保持。

本轮发生器资源是100 workers／100 in-flight，5类scheduler deficit为 `admission_saturation`、`schedule_late`、`worker_start_late`、`window_expired`、`unfinished`，各类计数及最多64条固定安全字段样本用于定位漏发／未完成slot；这不是重试、catch-up或主请求失败分类的替代。[独立R18工具回执](../../../local-identity-sso-phase13-qa/load-generator-r18-validation.json)记录Linux **59／59项纯回归通过**、exit0，原始完整测试日志以SHA256绑定；该回执明确 `PURE_PREPARATION_ONLY_NOT_LIVE_ACCEPTANCE`、未启动产品服务、未改产品binary／主计数／延迟门槛，也未改R17结果。R18实际未完成35分钟及所有后续browser／恢复／通知，不能把启动、snapshot或59项工具测试改写为验收PASS。Phase 13继续 **IN_PROGRESS**。

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

本轮221协议、15灰度检查通过；六个进程停止与完整两流扫描PASS，独立gateway退出0、graceful RPC close=true、forced termination=false、两流0字节且扫描完整。两任务数据库、portal／notification／deployment角色删除及container hosts恢复均为true，清理范围不扩写为Windows外部资产。burst、两轮browser、完整身份感知双库恢复与恢复后通知未执行。R18原FAIL及内层异常未知的事实保持；随后修复与R19实际启动证据如下。Phase 13继续 **IN_PROGRESS**。

### FULL 第19轮实际启动与等待竞态回归

R19 fresh FULL已启动，task UUID为 `01a171b0-87e2-7f9d-a3b5-228ce0070515`，主执行会话46886。[R19 source snapshot](../../../local-identity-sso-phase13-qa/full-round-19-source-snapshot.json)绑定609文件，content-set SHA256为 `66dba13490fd979db12fa2308f4b2766ece2e3e484cdbfadeeb8881defc655e6`；[execution-state](../../../local-identity-sso-phase13-qa/execution-state.json)记录当前指针。该轮继续release identity `3534e49b983cb56b5375a59d9eea5bf44b680bac91b90fabd35e4f6d895c8dd3`、100 workers／100 in-flight、100 actor及完整原容量合同，完整35分钟及随后browser／恢复／通知尚待实际结果。

[两项旧负sleep复现](../../../local-identity-sso-phase13-qa/load-generator-r19-negative-sleep-reproduction.json)以原R18负载源码分别覆盖per-slot和observation-window等待路径，受控得到预期`ValueError`／exit1；scope明确不是对R18现场异常的归因。修复在这两个等待点均于harvest之后重新计算remaining，并只在remaining严格大于0时sleep，避免检查与等待之间跨越due／结束窗口产生负值。另新增安全异常诊断：仅输出allowlist异常类，未知类归`UnexpectedError`，最多12条经过自有源码过滤的basename／line／function frames；不输出异常text／args、URL、源码行或完整路径。R18当次内层类仍未知，原FAIL不改判。

[R19独立Linux工具回执](../../../local-identity-sso-phase13-qa/load-generator-r19-validation.json)为 **69／69项纯回归PASS**，其中含上述两项防竞态回归；[五种完整计划压力模式回执](../../../local-identity-sso-phase13-qa/load-generator-r19-long-plan-stress.json)覆盖exact fake clock、floating boundary、harvest跨due、clock-read推进及刻意产生五类deficit的模式，每模式保持264000个不可变主slot／模拟2100秒，验证分类及等待边界。故障模式的测试PASS表示其预期错误／不足被正确记录，不表示该模式所有业务slot都成功。整个纯测试suite真实计算耗时42.199秒、runner耗时44.7094264秒；模拟2100秒与这些真实计算时间均不是产品真实35分钟容量。回执明确无网络／产品服务、无真实sleep、无产品binary或密码池／数据库参数变化，不补发／重放、不改主计数与延迟门槛。

R19的实际验收仍要求1800＋300真实秒、原各操作RPS和24000认证／240000 authority目标、0.5秒最大lag、锁前起算3秒绝对预算、每actor串行锁及原p95／p99门槛，随后同task完成两轮browser、身份感知双库及库外材料恢复、通知、全部输出扫描、source／binary复验与清理。最终私有材料helper独立于609文件live seal，按届时实际19任务目录动态枚举准确UUID、绑定新鲜clearance及新dry-run plan SHA；当前仍未apply。完整链路最终receipt就绪前保持 **IN_PROGRESS**，不以69项纯测试或压力模式提前封账。

## 隔离拓扑与授权范围

公开地址固定为 `https://auth.saas.test`、`https://console.saas.test`、`https://customer.saas.test`，issuer／subject／精确callback与schema保持稳定。identity、platform、portal及独立notification按各自受限角色运行；认证和平台共用平台数据库，门户有独立会话数据库。candidate identity在固定38086使用同一已验证normal binary，不能把test executable、旧无类型实现或另一个issuer作为回退目标。

任务管理连接仍限定loopback及准确cluster；**所有identity／platform／portal／notification运行角色与恢复后的运行连接均走固定容器地址 `172.20.0.2` 的SCRAM认证路径**。driver用每个准确运行角色分别证明正确密码可连、错误密码被拒，恢复后再验证轮换前密码拒绝。loopback的既有trust行为只属于任务管理连接，不能拿它证明运行密码轮换有效；本流程不修改共享`pg_hba.conf`，receipt记录`shared_hba_changed=false`。该地址属于当前固定测试集群，不能复制成生产网络假设。

所有账户、两RP、客户resource client、通知EMPLOYEE application、三种内部service用途及临时角色都由任务bootstrap明确登记。测试成员生命周期开关不代表生产开放；首次客户业务仍为本人四字段资料，客户通知与员工管理／目录不扩权。不得触碰预览UUID专项数据库、改历史migration或SDK archive／vendor。

[QA13工具镜像](../../../local-identity-sso-phase13-qa/tools-image-result.json)在已验收PG17 client镜像上仅加入Debian Argon2 CLI，产品依赖未变；[独立PKI准备](../../../local-identity-sso-phase13-qa/pki-result.json)为task CA／CA:false leaf、三个准确SAN与独立BFF keyrings。准备PASS不代表live或生产DNS／OS信任已验收。

## 运行入口与证据归档

从平台仓根目录运行 [run-live.ps1](../../../local-identity-sso-phase13-qa/run-live.ps1)。开始前核对工具镜像、正常二进制manifest、任务集群／卷／端口、私有环境文件及PKI pins；保留产品默认关闭开关，只由任务clean environment显式启用fixture。共享正常二进制在运行期禁止重建；同一集群的live driver串行执行。

```powershell
# 正式fresh运行：固定100 actor、1800＋300真实秒，再执行两轮浏览器与恢复。
.\local-identity-sso-phase13-qa\run-live.ps1

# 接线预检：明确跳过完整容量，始终NOT_ACCEPTANCE。
.\local-identity-sso-phase13-qa\run-live.ps1 -IntegrationPreflight

# 仅预检可用：额外跳过完整protocol，用真实管理API创建恢复所需receipt。
.\local-identity-sso-phase13-qa\run-live.ps1 -IntegrationPreflight -DebugSkipProtocol
```

`-DebugSkipProtocol`单独使用立即拒绝。debug分支将protocol记为`NOT_EXECUTED`／`DEBUG_INTEGRATION_ONLY`，通过真实 `POST /api/oidc/clients`建立准确task创建回执，secret只进内存与已注册的输出scanner；它解决恢复fixture对真实receipt的依赖，不能代替221项协议／故障／密钥检查。integration preflight的capacity明确`NOT_EXECUTED`，不是缩短容量。正式命令无此两个开关，计时长度不能从命令行调小；`--hold-seconds 1800`是每轮等待浏览器的期限，与35分钟负载计时分开。

driver输出`PHASE13_BACKEND_READY`后，按 [copy-browser-fixture.ps1](../../../local-identity-sso-phase13-qa/copy-browser-fixture.ps1)核task UUID和两个公共证书SHA，将私有fixture只放Windows任务TEMP。再用 [run-browser.ps1](../../../local-identity-sso-phase13-qa/run-browser.ps1)启动真实浏览器。可复用公开PKI pins及runner生成的task ID：

```powershell
$taskId = (Get-Content -LiteralPath .\local-identity-sso-phase13-qa\active-task-id.txt -Raw).Trim()
$taskPki = Get-Content -LiteralPath .\local-identity-sso-phase13-qa\pki-result.json -Raw | ConvertFrom-Json
$taskCopy = .\local-identity-sso-phase13-qa\copy-browser-fixture.ps1 -TaskId $taskId -CertificateSha256 $taskPki.leaf_sha256 -CaCertificateSha256 $taskPki.ca_sha256 | ConvertFrom-Json
.\local-identity-sso-phase13-qa\run-browser.ps1 -FixturePath $taskCopy.fixturePath
```

等待`PHASE13_RESTORED_BACKEND_READY`后，以同task ID和pins再次copy并追加`-AfterRecovery`，再执行第二轮browser runner。更新后的fixture含新受控凭据／运行配置，不能沿用initial私有副本。WSL loopback bridge只覆盖38081～38084并使用已核对的任务容器地址；Node代理精确拒绝不匹配的Host／SNI，记录Origin是否存在及是否与该host一致，并将Origin转发Rust后端执行实际验证，不能将这些观测字段描述成Node已统一拒绝非法Origin。Socket.IO专用upgrade路径另按其精确Host／SNI／Origin规则处理；浏览器API只能到对应真实后端。bridge／Windows临时资产由主执行者独立清理，不能以driver的container-only hosts回执代替。

每轮立即将driver、browser、launcher、process／protocol／canary／capacity／recovery安全报告另名归档；initial与restored截图分别保存，不覆盖失败轮次。`browser-finished`只是控制面握手，即使它存在或driver某子报告PASS，也须检查实际browser结果。用 [collect-live-evidence.py](../../../local-identity-sso-phase13-qa/collect-live-evidence.py)只收固定公开JSON，失败仅在driver确为FAIL时允许`--allow-partial-failure`；缺少报告保留missing清单，不读私有fixture补充。

## 执行顺序与最终操作要求

1. 创建带准确task／cluster／comment绑定的两个新库，唯一canonical job执行平台schema；完成bootstrap、三service登记、两RP与通知application，再交接六角色、portal及notification必要权限。
2. 启动独立normal进程，重做协议／故障／key检查及共享平台库基线restore，保留完整扫描和pins。
3. 对本task两个RP的可识别请求做10／50／100%分流；实际暂停candidate，确认503与不自动fallback，然后显式回到baseline。
4. 正式fresh任务执行100独立OIDC actor的实际35分钟负载，报告主／辅助请求、失败类别、分位数、资源和队列数据。
5. 运行initial浏览器双RP与独立通知生命周期；输出安全报告和initial截图，私有通知见证留task TEMP。
6. 浏览器全局注销后另外新取仍有效的旧载体，协调停止所有进程，捕获双库与库外材料checkpoint；恢复源服务后施加post-backup安全收窄，封存备份外witness，再执行源故障／原名双库restore、fence与deny-only对账、凭据轮换和受控reset。
7. 检查旧密码／旧载体拒绝、新SSO及两个RP业务通过；使用更新后的私有配置运行restored浏览器和通知第二轮，不能复用已注销载体作为唯一恢复防线证明。
8. 最终重新审查source、SQL与活动消费者，登记计数／原始时点／限制，停止全部normal／candidate／notification／恢复进程后清理任务库、角色、hosts与bridge。仅完整真实receipt后将五项验收改PASS。

## 请求灰度与显式回退

[canary-rehearsal.py](../../../local-identity-sso-phase13-qa/canary-rehearsal.py)和 [gateway](../../../local-identity-sso-phase13-qa/auth-gateway.py)只接受本task准确的console／customer两个RP client ID。gateway在auth host解析唯一一致的query／Basic／form client标识，以100个eligible请求为周期，配置10／50／100%到candidate；其余请求走baseline。登记的tenant是fixture上下文，gateway不按tenant字段路由，因此本轮是**指定task RP请求比例**演练，不声明生产多租户扩围、用户比例或真实版本升级。

每档计划各50次code exchange与50次authority检查，记录eligible／baseline／candidate差值且准确相加；100%时baseline须为零，两个真实BFF均须重新登录及选精确成员。candidate实际SIGSTOP后，受影响请求必须得到真实503，配置仍维持100%，不得用自动路由切换掩盖故障。随后显式set_canary(...,0)，以全新操作重验两个RP及BFF，不重试未知code／refresh；finally恢复并停止candidate。

回退种类是 `SAME_VERIFIED_BINARY_ROUTING_CONFIGURATION`：同normal binary、同身份感知schema和数据库、同issuer与callback的路由配置回退。它不证明跨版本二进制降级，更不能降到user＋org二元唯一、缺member_type／代际证明、客户混员工授权或无在线撤销的旧版本。已有preflight记录实际比例、503、显式回退耗时与candidate停止；正式fresh运行仍须独立记录，不继承旧轮次成功计数。

## 实际35分钟容量计划

[load-rehearsal.py](../../../local-identity-sso-phase13-qa/load-rehearsal.py)固定1800秒steady＋300秒burst；[protocol-client.py](../../../local-identity-sso-phase13-qa/protocol-client.py)通过真实SSO／PKCE／consent／code／refresh／introspect操作建立100独立actor，50 EMPLOYEE、50 CUSTOMER。各actor独立CookieJar、sid、grant、refresh和锁，使用固定两RP，不要求新增100个账号。重复登录可能留下仍有效旧sid／grant，100指受控actor／当前会话，不伪称数据库历史session恰好100。

`bootstrap_actors`在主计时前先串行完成100次真实密码登录，再用最多8个worker对100个actor各做一次有界refresh及一次当前proof在线内省。它验证100个不同sid／grant保持原绑定，全部新proof有效，最早access到期距离计时开始仍至少30秒。串行登录耗时可能使最早access自然到期，报告单列该数量；用原refresh正常轮换恢复当前access，不能重试未知refresh。初始化后头部不足30秒、任何失败或绑定改变即停止，不以先启动主计时、增加TTL假造初始化成功。100 login、100 refresh和100proof均归bootstrap／辅助，不计主目标。

| 主操作 | steady RPS／1800秒 | burst RPS／300秒 | 计划主请求总数 |
| --- | --- | --- | --- |
| 密码login | 2／3,600 | 4／1,200 | 4,800 |
| code exchange | 4／7,200 | 8／2,400 | 9,600 |
| refresh | 4／7,200 | 8／2,400 | 9,600 |
| online introspect | 100／180,000 | 200／60,000 | 240,000 |
| login＋code＋refresh合计 | 10／18,000 | 20／6,000 | 24,000 |

bootstrap、preauth、authorize／consent、辅助code／在线确认、旧grant撤销及额外业务探针单列；不得加入24,000／240,000主目标。主操作scheduled／sent／completed／success须与固定slot目标逐项相等；漏发、调度过迟、deadline、saturation、错误成功DTO、依赖失败或安全拒绝不计正向成功，也不catch-up／重放凑数。每actor锁保护一次性refresh及当前原始source tuple；新login后新建flow，不把新Cookie与旧sid强行配对。

隔离目标沿用 [Q6建议](open-question-recommendations.md)：introspect p95≤100ms／p99≤300ms；code／refresh p95≤500ms／p99≤1000ms；密码认证单独p95≤1000ms。R15～17的发生器是32 workers／32 in-flight；R18及当前R19均使用100／100，仍有界排队并保持每actor串行锁，产品密码worker／队列不变。记录各操作／阶段成功和全尝试分位数、错误类别、调度不足、CPU／RSS、数据库／Argon2参数及实际观察时长。总期限3秒／连接上限1秒的既有权威预算继续作为边界；实际helper与端点计量据最终receipt核对，不以source设定冒称达到吞吐。

操作延迟范围须随报告解释：服务延迟从取得每actor锁后开始，锁前驱动等待另记`actor_lock_wait`；login服务延迟只计preauth＋password HTTP，不把随后辅助grant安装计入password分位数。code计authorize／consent／exchange及本地RS256验证、辅助在线proof，refresh计轮换／RS256验证及辅助在线proof，introspect是一次严格actor绑定的主在线检查。`schedule_lag`与`completion_lag`另外保留，后者包含调度、actor排队、服务及适用的辅助工作，不能用服务分位数隐藏等待。

**3秒绝对操作预算在取得actor锁前开始**，由锁等待、服务及同操作的辅助流程共同消耗；嵌套操作继承更早deadline，不重置预算。锁后预算已经耗尽时按失败／driverdeficit记录，后续HTTP不得获新的3秒；不能只报告锁后服务快就把排队超时算成功。原始302／303在验证issuer／state／精确callback前不直接分类成功；主操作最终成功与辅助HTTP状态分开。0.5秒最大调度lag、相同主请求RPS／数量及既定p95／p99门槛均未放宽。该计量修正在第14轮容量尚未开始时完成，[第04次54项Linux回归](../../../local-identity-sso-phase13-qa/load-pure-validation-round04.json)固定了边界；R15～17按此口径实测失败，原始部分计数保留。R18只将有界发生器资源32／32调为100／100并增加漏发细分类，绝对预算及判失败规则保持。任何缺发、非预期错误、计数不齐或候选延迟门禁未达仍记FAIL，不补发凑数。

这组35分钟是TASK_OWNED容量候选，不能据此认定滚动30天99.9%、生产容量或任意生产SLO。preflight跳过负载即NOT_ACCEPTANCE；不把其余真实功能通过换成容量通过。

## 完整双库与库外材料恢复

实现位置为 [恢复编排](../../../local-identity-sso-phase13-qa/recovery-rehearsal.py)、[恢复原语](../../ops/identity-recovery/recovery.py)及 [外部witness](../../ops/identity-recovery/witness.py)。第12／13轮已分别实际执行该模块并获得子报告PASS及各140项检查，详见 [原始第12轮](../../../local-identity-sso-phase13-qa/preflight-round-12.json)及 [第13轮](../../../local-identity-sso-phase13-qa/preflight-round-13.json)；第12轮browser不完整、第13轮进程输出扫描失败，正式fresh全链须重新执行。函数名或纯测试不作为恢复实测。

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

initial与restored使用不同通知publication标签，私有旧Bearer见证固定在task TEMP、schema／task绑定，默认不序列化。第12轮initial故障探针失败保持原receipt；第13轮debug的 [initial browser](../../../local-identity-sso-phase13-qa/preflight-13-browser-initial.json)已PASS341并包含notification43检查，[restored browser](../../../local-identity-sso-phase13-qa/preflight-13-browser-restored.json)已PASS247。该轮仍因进程输出扫描FAIL，且没有完整35分钟负载，不能封账。纯测试不替代这两轮live事实，debug成功也不替代正式fresh全链。

## 最终SQL与活动消费者审查

逐命中审查以当前生产源码为准，不用rg零命中代替分类；历史migration、cfg(test) fixture与业务ChatRoom／二元业务幂等保留原义。每项记录具体路径／行号、成员定位、局部／全局范围、现有回归引用和本阶段实际证据，最终表见 [验收记录](validation/phase-13.md)。已有测试名仅证明当前回归存在，执行结果按各阶段receipt引用，不能冒称本轮重跑。

重点核INSERT／UPDATE／DELETE／ON CONFLICT、JOIN／LIMIT1／fetch_one／first、角色祖先／superadmin／data grant、全载体partial proof、trigger／窄projection／bridge、默认广播／recipient和缓存namespace。当前 [消费者与SQL审查](phase13-consumer-audit.md)已核**29个生产区域含memberships字面量的文件**，其中含配置／路由名而非全为SQL；排除cfg(test)后5生产writer文件／11 SQL位置，两个成员ON CONFLICT均三元，无新生产SQL／接口安全阻断。账号级RETIRE按接受时(id,identityVersion)快照删原关系，无type过滤属于明确全账号操作。精确路径与R1～R10回归来源已入验收表；旧凭据live拒绝及最终source seal独立核对。

八仓当前消费者只读审查已完成、无新安全阻断：活动SDK3.0.0是REPOSITORY_CANDIDATE／private package，不冒称外部已发布；MicroProps2.0／Context1.0／Notification1.0独立。stores默认legacy、work-orders默认combined且失败不切路径；既有confidential MACHINE／LEGACY_MIXED client_credentials兼容，新bootstrap只准MACHINE＋空成员集合。平台psr_ refresh在/oauth/token独立分支，legacy OAuth与标准OIDC /oidc/token refresh分开；CLI只消费显式Bearer，playground是legacy OAuth演练消费者。精确路径／行号及既有回归已入验收表，源码／fixture不当本阶段运行或生产流量证据。

本阶段活动消费者实际执行 **417项合同测试全部通过：233项Node＋184项Rust普通测试**，另通过SDK20生成发行物及平台固定SDK校验，详见 [活动审查](phase13-consumer-audit.md)、[Node回执](../../../local-identity-sso-phase13-qa/consumer-node-contracts.json)与 [Rust／逐命中／源码回执](../../../local-identity-sso-phase13-qa/consumer-source-sql-audit.json)。这些是实际当前消费者合同，未将ignored PostgreSQL测试或417项单测算成417个端到端场景；后续恢复／helper修复由各自门禁另验，所有最终实际源码与固定发行字节仍须统一封存。

## 正式验收聚合与停止条件

driver的单独exit0或恢复子报告PASS不代表全阶段完成。主执行者必须将同一fresh task的全部下列事实连同测试轮次、准确pins和实际时点聚合后，才能更新台账五项标准：

| 检查面 | 必需证据与停止条件 |
| --- | --- |
| 配置／权限／网络 | canonical20迁移history、准确两库marker与owner、身份／平台／portal／notification最小权限；运行SCRAM正确密码可连／错误密码拒绝及未改HBA。不得因缺权限扩大运行角色。 |
| 协议／灰度 | 完整协议PASS，两个准确RP实际10／50／100%计数、candidate真实503、不自动fallback、显式set0与两RP新登录。非预期登录失败、回调错误、错误成功DTO或授权扩大立即记FAIL并执行已验证路由回退；不换issuer／schema。 |
| 容量 | 实际1800＋300秒、100独立actor就绪、24,000认证＋240,000权威主slot四计数逐项相等；分位数／资源／队列／错误类别完整且达到本任务候选阈值。NOT_EXECUTED、缩时或补发均不通过。 |
| 恢复 | 两个原名restore、owner／comment／ACL／catalog／安全投影、库外材料、外部witness及start receipt、deny-only、全载体／旧密码拒绝、新两RP业务全部真实通过；保存实测RPO／RTO及丢失分类。还原失败不接流，保持fence并保留诊断。 |
| 浏览器／通知 | initial与restored均PASS，通知模块两轮实际收发／局部撤权／故障关闭／恢复，新旧凭据和客户隔离、逐张视觉审查及公开载荷边界完整。browser-finished或部分检查计数不能覆盖FAIL。 |
| 输出／源码／清理 | 全部过程输出完成known-secret及pattern扫描，任一capture未完成或有类别命中必须保留FAIL；最终源码／binary／migration／SDK归档及vendor字节复核，全部task库／角色／进程／candidate／container hosts与外部bridge／临时资产清理有准确回执。 |

公开收集器的JSON安全扫描与原进程完整流扫描分别记录：公开JSON无秘密不等于原进程流扫描已通过。失败输出只保存字节／SHA／固定类别，不能为排查抑制类别重新暴露原始SQL／环境／Cookie／token。历史失败轮次与最终成功轮次分开链接，任何未验证项保持进行中。

第12／13轮的`platform-recovery-5`进程虽然exit0且binary未变，完整stdout扫描仍记录`known_runtime_credential`；第13轮stdout计314,011字节。源码审查发现旧`remember_tree`把任意`code`字段视为秘密，可能把公开application code或错误标签加入known集合；修复已收紧到真实凭据上下文及形状，[52项Linux回归](../../../local-identity-sso-phase13-qa/load-pure-validation-round03.json)直接覆盖公开code／appcode与日志相交，同时证明真实秘密仍登记。**原来命中的确切值没有保留，不能据源码发现就把旧扫描判为已证明的误报**。两轮原FAIL及固定类别保留。第14轮容量前主动中止，第15～18轮FULL容量实际失败，其已执行进程扫描与清理结果只归各自轮次，不替代尚未执行的browser／完整恢复。第15轮历史输入冻结记录为608文件、content-set SHA256 `252bbaca97393fb8c7639b7ddde227e321134ad9571b8b700d6bd4b4005f9ca8`；已结束的R17冻结为 [609文件snapshot](../../../local-identity-sso-phase13-qa/full-round-17-source-snapshot.json)及 `cd5d1c842acad7b3f4c1df5b700f3a0d5e9fe1ce375fc15f143d0c4aa454cce2`；已结束的R18输入为 [609文件snapshot](../../../local-identity-sso-phase13-qa/full-round-18-source-snapshot.json)及 `139abb88a721fccc9b6983697c5202e37ccf09eeafe7cca02b1a51f44c83e721`，均为各自历史输入。当前R19准确输入为 [609文件snapshot](../../../local-identity-sso-phase13-qa/full-round-19-source-snapshot.json)及 `66dba13490fd979db12fa2308f4b2766ece2e3e484cdbfadeeb8881defc655e6`；不跳过known-secret检查或放宽公开JSON安全规则，正式结果及结束后的相同字节复验完成前仍未验收。

## 退役N/A与交付边界

本身份TASK_OWNED新库未导入历史账号／凭据，没有已登记待退役历史客户端；历史迁移／转换窗口／停用日期仅在这一清单范围记N/A，不推断其他环境没有旧部署。活动SDK、app派生、机器grant、CLI、playground、stores／work-orders legacy／combined及通知仍受支持，不能随历史N/A一起退役。真实对象以后出现时，必须另记owner、准确client／实际流量、迁移验证、最长凭据寿命、观察窗口及撤销方式后再决议。

当前只修正文档的SDK旧标签与主方案工程／阶段陈旧事实，不修改固定发行字节或历史准备记录。任务来源授权覆盖全部阶段，当前只实施Phase13；正式创建与生产默认关闭，生产域名、人员owner、SLO／RPO／PITR不凭测试假设编造。所有五项实际验收完成并可追溯后才封账。
