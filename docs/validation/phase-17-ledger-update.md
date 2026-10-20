# Suggested Task Ledger Update — Phase 17

客户端日期2026-10-17（Asia/Shanghai），原始ISO时间不改。本次采纳Phase17 `COMPLETED`：四项行为标准及范围内全部执行门禁PASS，[managed审计](../../target/phase-17/managed-audit.json)14项全PASS；完整共享fast两项外部失败和当前平台strict失败保留，不拼称一次全绿。来源采用[正式运行索引](phase-17-runtime.json)的PHASE17_GATES_PASS语义。本文件不直接修改正式台账。

## Phase Status Update

Phase17：`COMPLETED`。四条验收有同674项候选真实PG/真平台/计划/同源runtime支持，Phase17工单fmt/check/strictClippy/lib34/doc五个fast必需项均PASS，ci-policy亦PASS。仅推进17→18 `NOT_STARTED`，并行其他阶段已采纳结果及修改保持，本次不实施其它阶段。

## Output Summary

- Rust有界私有平台客户端，每请求Basic内省再同应用Bearer context；严格aud/active/身份/到期、permission/grant绑定和组织ceiling，无跨请求正向缓存。
- 参数化同scope列表/count/detail/筛选/分页与CSV，只读一致快照及查询后freshness；BOM/CRLF/UTF8/公式防护、1000行/4MiB、OPTIONS与错误CORS/requestId保留。
- 容器PG9步全PASS：平台74/迁移15/controlledHTTP+ownedPG13、真正平台1harness/13named均零失败/跳过；Linuxlib34/doc1/buildPASS。同源原ELF补runtime11PASS，原外层r2因宿主manifest漂移的FAIL继续保留。
- 24,008行8份完整EXPLAIN归档，实际scan/buffers/sort及0.095–40.713ms样本范围可复核；无新索引，不作生产SLA。
- 完整共享fast实际FAIL：50步48PASS/2FAIL，测试700PASS/1FAIL/0skip；两失败属于冻结并行CLI契约输入遗漏/工具crate格式，范围内Phase17门禁全部PASS。当前不同宿主补验contract5/5及工具fmt退出0单列，不能回写原fastPASS。
- Node授权参考明确退役，JS7仅兼容；17旧Node/SQL保持。原221依赖身份+32=253，license253声明齐全，无新参考crate/平台实现依赖。

## Completed Items

- 四条行为验收全部PASS；产物/细节见[Phase17报告](phase-17.md)、[正式输入](phase-17-inputs.json)、[边界](phase-17-boundaries.json)、[计划归档](phase-17-query-plans.json)。
- 同674项候选`59b0f8d12db41b566e42f59d0aeba38f2afe72c547a18ca3501dabb53a654028`的9步PG及额外lib34/doc1/build均PASS，原容器源before/after相同。
- 真正平台main/router+Rust工单进程双库的13named全部PASS，真实token/内省/context、scope与撤grant/退出/删成员新请求失权通过；双库/pool清理全true，业务进程停止。
- raw同源补验11PASS：ELF65,460,864bytes/`7155530bb405c655813c9edb6cfef132509cc8a007631298a03120f854974f15`，UID10001、无Node/hostports/mounts，rootfs可写；只证明401/503/CORS failclosed及健康/故障/启动锁/历史counter/退出，成功平台调用由真正联合证明。资源空、lease0。
- BOUNDARIES_PASS的19项检查确认23独占输入等价、17旧兼容及旧0001保持，旧helper注释外可执行尾不变，253license声明。shared宿主漂移不是本阶段产出。
- Windows早轮25/6与auth专项7/2 UNAVAILABLE保留，Linuxlib34全部执行补齐；当前宿主平台strict Clippy53error/新harness0诊断单列，不错误称旧38或平台全通过。独立工单strictClippy通过。
- 当前宿主work-orders CI定向6/6PASS、exit0、10输入稳定；原collector因spec/TAP计数差异生成FAIL，重析同一未改日志确认实际6通过，原FAIL保留、不重跑，不替代674候选/PG/fast。
- 外层r2仍FAIL：仅因容器成功后宿主共享manifest漂移，raw未创建；库/容器/网络清理与lease0记录独立保留，不能覆盖旧FAIL。
- Phase17的工单fast五步、ci-policy63及当前宿主CI定向6/6已通过；原完整fast50步48/2与700/1按实保留，两项外部失败的不同宿主补验5/5+fmt0不混入原计数。未跨阶段修改工具CLI源码，也未改受测674候选。
- [范围内fast派生](../../target/phase-17/focused-fast-result.json)为PHASE17_SCOPED_FAST_PASS：原9选中步骤与137测试（lib34/policy63/原contracts兼容40）全PASS，137已含lib34，不重复累计；derivationOnly、无测试重跑，不覆盖完整fast原FAIL。
- managed14项PASS，核对同674候选、PG/lib/doc/原ELF/driver及资源/库/pool/进程/lease清理、原FAIL保存；当前host策略6和external5+fmt分别采用，不把不同源补验改写为共享全fast通过。

## Remaining Items

Phase17范围内剩余：**无**。四项标准及相关执行门禁全部PASS；完整共享fast的范围外失败、Windows历史失败和平台strict失败作为明确限制继续保留，不改写为全仓通过。Phase18须新的明确请求，本次不实施。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| SELF/ORG/ORG_SUBTREE/TENANT及组织上限、其他租户、上级平级、无grant、错误受众均真实验证。 | PASS | controlledHTTP+ownedPG13及真正平台1harness/13named分别实测scope/ceiling、跨tenant/祖先平级、空grant、错误aud；不以raw故障探针充数。 |
| 平台不可达503、失效401、缺授权403、不可见404；客户端secret不进前端/错误。 | PASS | HTTP负向/lib34、真平台撤grant/退出/成员删除、private secret及脱敏通过；Windows7/2环境失败保留，Linux补齐。 |
| CSV范围与列表一致，注入/容量/编码回归通过；OPTIONS及错误响应CORS正确。 | PASS | 真实SQL集合一致，CSV1000行/4MiB/BOM/CRLF/UTF8/公式/控制前缀及OPTIONS/错误CORS通过。 |
| 极大page稳定拒绝/受限而非溢出；列表/count/导出同范围，计划及读取成本有可复核基线。 | PASS | page上限/checkedoffset验证；24,008行8份代表计划有完整SQL/binds/scan/sort/buffers，深页自身1602行扫描排序不代表亿级生产恒定成本。 |

## New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 每请求Basic内省再Bearer context，不正向跨请求缓存 | 语言迁移保持实时身份/受众/授权 | 限时/并发/body，平台不确定503；permission/grant独立 |
| 全读取共用tenant/ceiling/grant参数化谓词及freshness | 列表/count/detail/CSV必须一致，处理人不扩大SELF | 用户过滤只收窄，读快照与返回前到期检查 |
| CSV保留字节预算、完整拒绝和统一公式编码处理 | 转义膨胀/容量不能绕过导出边界 | 1000行/4MiB、BOM/CRLF/UTF8和所有文本快照防护 |
| controlledHTTP13、真正平台1/13、raw11、JS7分开计证据 | fixture及不可达探针不能证明真实平台成功 | 真平台双库/进程成功联合；raw只failclosed，旧JS只兼容 |
| 原外层FAIL保留，raw另行同源补验 | 宿主共享manifest漂移不等于容器PG失败，但原runner不可改写 | 同674输入/原ELF不变，PG不重复，原始状态与补验分列 |
| 以范围内门禁PASS采纳Phase17，完整fast外部FAIL明确披露 | 共享冻结候选包含并行CLI契约未选入依赖和工具fmt差异，非本阶段功能回归 | 原50步48/2、测试700/1不改；当前宿主5/5+fmt0不同源补验单列，不能称一次全绿 |
| Node授权参考退役，指向实际Rust模块，无新参考crate | 不继续推荐Node后台，不提前抽象通用库 | 注释/README/活动文档同步，原函数行为保持 |
| 代表计划只交付实际组件成本，不默认新增索引 | 8份模型不是每个HTTP SQL trace或全grant性能 | 保留现有org/owner索引；扫描/排序/buffers与样本限制如实记录 |
| 外部并行变化仅observed drift，不改其它状态 | 当前host包含并行其他阶段工作 | 23项本阶段独占字节等价单独证明，只推进17→18 |

## New Open Questions

无新增业务开放问题。原台账外部环境/生产政策及范围外共享CI限制按所属阶段保留，不因隔离验收宣称生产或全仓全部门禁通过。

## Next Recommended Phase

Phase18：实现 Rust 工单写入、幂等和短事务。`NOT_STARTED`，需新的明确请求，本次不实施。

## Ledger Evidence Note

本次采纳Phase17 `COMPLETED`：Rust逐请求平台授权、统一scope只读/CSV、分页/CORS/编码边界及代表计划四条全PASS。保留同674项`59b0f8d12db41b566e42f59d0aeba38f2afe72c547a18ca3501dabb53a654028`；PG9步平台74/迁移15/read13/真正平台1harness+13named均PASS，lib34/doc1/build及同原ELFraw11补验PASS，工单fast五项与ci-policy全PASS。原r2外层manifest漂移FAIL及完整fast50步48/2、测试700/1原样保留；外部CLI两失败当前宿主5/5+fmt0不同源补验，不回写原候选PASS。8计划0.095–40.713ms样本、现有索引不新增，不作生产SLA。Windows历史失败、current平台strict53/新harness0诊断保留，JS7仅兼容。采用PHASE17_GATES_PASS语义，仅推进18 NOT_STARTED，保留并行其他阶段结果与修改；本次无写入/提醒/切流/生产操作。
