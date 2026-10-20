# Phase 8 执行与验证证据

日期：2026-10-15（Asia/Shanghai）。阶段：Phase 8，分离展示更新、授权续期和版本重挂。状态：**COMPLETED**；本阶段独立执行验收，不继承Phase7的测试通过结论。

## Executed Phase

用户明确要求执行Phase8，依赖Phase6、7均COMPLETED，台账与当前指针一致。遵循task-ledger-executor与D:/WorkSpace/rust/AGENTS.md；台账自身要求每阶段同步摘要、详情、证据、决策和指针。

实际起点为473项公共源码，SHA-256 `4a2f9ce6340c516fa5cee95e8a886c4c0a440febd3869b7b8cb434f03d3da611`，见`target/phase-08/baseline-inputs.json`。Git HEAD为`32956c4532bbd6eff53584fd0f5e9bb6e188437c`，保留此前全部未提交内容。起点摘要仅标识输入，不能作为本阶段验收通过证据。

## Execution Summary

本阶段只联动宿主、iframe bridge、工单客户端/现有UI的展示与真实授权更新分类，统一实时入口完整快照，并对临时传输/503故障作取消旧授权后的有界恢复。保留Phase7可信页面故障域、默认opaque iframe、授权期限和旧响应拒绝。

SDK1.0.0仍为未外部发行的仓库候选，当前候选生成物按既有sdk:sync更新；实际legacy bridge和历史发布字节保留，wire版本不变。不进行Phase9导航整合、Phase21工单Vite迁移、后端/数据库迁移、新依赖、外部发行或生产部署。

## Deliverables

- SDK共享`context-updates.mjs`提供带类型的完整授权签名与比较，含凭据的签名只留内存；未知字段保守替换，接受时缓存签名与signal，防原位修改漏判。bridge、通用client、宿主及Vue/jQuery共同消费，wire不增加字段。
- `runtime.ts`对完全一致且仍有效的上下文保留epoch/signal，真实替换取消旧工作；TRUSTED session缓存组合signal。临时NETWORK_ERROR/status0或503采用500/1000/2000ms最多3次退避，单尝试token加context共5秒、过程20秒，取消与序号拒绝不合作传输的迟到结果。
- `entry.ts`与容器从同一实时context解析完整运行/入口/版本tuple，profile变化只触发一次复核，异步边界与buildProps重新核对。恢复或入口替换同步abort、移除旧节点并清空快照，Phase7可信故障域仍保留。
- 工单client按稳定授权generation判活；app纯主题不取消写/重读列表/清403拒绝状态，路由读取有独立controller与序号。授权变更清服务端快照，草稿保留受新身份/授权及fresh detail复核约束；下级组织处理人目录仍使用工单归属组织。
- 未实现update的当前兼容生命周期，纯主题保留现实例与请求，主题延后到真正重挂；路由/授权仍沿用串行重挂的原兼容行为。当前工单及具备update的消费者按相关读取增量处理路由；保留的旧静态bridge不被覆盖，也不会自动得到新语义。

## 发现与先失败证据

只读起点探针`target/phase-08/reviewer/trusted-signal-baseline.json`核对runtime源码SHA为`84191bc6…`，同一输入signal经TRUSTED session的mount与theme update交付后，`sameDeliveredSignal`为false；旧signal未abort且可信domain仍healthy。它说明单纯让consumer严格比较signal仍会把展示变化误归为授权替换。该探针只记录起点行为，不计实现通过；相应修复需稳定缓存组合signal并由后续行为回归证明。

消费者修改生产源码前的`consumers/before.log`实际5/5 FAIL，覆盖theme取消/重取、清除403拒绝状态、移除read grant但仍有update时保留编辑器旧记录、client展示更新取消及invalid update未关闭旧上下文。中间审查另复现display复用后旧props引用使abort监听失效、无类型编码把不同grant形状判同；真实负例分别保存在`reviewer/client-display-abort-r1.json`与`signature-types-r1.json`。修后`recheck-r2.json`均通过；`client-signal-mutation-r1.json`执行前修复已到位，属于修后PASS，不冒称旧实现负例。

宿主最初3项Node失败保存在`host/old-host-negative.log`，当时未单独保存新增测试的SHA。补强的`old-host-negative-r2.log/json`使用只读loader加载已核对的Phase7 runtime，精确记录当前测试、旧模块、loader及日志SHA，3项真实FAIL；它是新测试与旧runtime的混合诊断，不声称完整473项起点门禁重跑，没有回退共享源码。

真实旧host/SDK浏览器负例使用Phase7已保存source的只读Vite覆盖，逐项核对473/4a2f与Phase8起点一致，再记录实际加载override路径/SHA：`old-host-entry-negative-r2.log`证明profilev1/contextv2却加载资源v1；`old-sdk-work-orders-negative-r3.log`证明真实Vue新建POST中切主题导致实际fetch signal被abort；`old-host-recovery-negative-r5.log`证明首次503后12秒无自动恢复。各negative-inputs及observed/build trace绑定实际混合输入。更早的Vite配置失败、缺菜单前置和StrictMode首实例取消诊断均保留，不计作有效功能负例。

完整Windows首轮在5组通过后因本地public的旧SDK bundle与新示例app不匹配失败，原始FAIL保留；执行同源build:all并逐对验证source/served摘要后重跑。第二轮通过前14组，在原isolated ESM拒绝场景等待旧mount层文案失败：实际新入口validator更早硬拒绝，iframe/脚本执行为0。只同步该用例一行精确文案，0iframe/脚本/child事件断言不变；完整478项对照确认旧冻结`aedb9015…`到新冻结仅此测试文件变化。旧冻结输入保存为`target/phase-08/freeze-1-inputs.json`，新输入下重新build并执行完整轮；不把两轮部分通过相加或改写成完整成功。

Windows完整r3在freeze2下47/47通过。随后原权限套件的第一个授权微应用正例暴露fixture缺少入口元组，因新的完整入口校验而拒绝挂载；仅补齐context.application的entryType/entryUrl/entryVersionId/deploymentVersion，原64组权限断言均保留。补齐后原脚本Windows r2实际64/64通过、零skip、exit0。freeze2保存于`target/phase-08/freeze-2-inputs.json`，freeze3相对freeze2仅该权限测试文件变化，生产实现未改。

三产品Linux browser r1均已通过10项安装/构建前置，但在freeze3确定后按核对过的各自owner/完整容器ID受控停止，保留原始FAIL/exit143及部分报告，不计作169组完成。停止前只清理各自容器内自有`/reports/browser/work`临时缓存；原始审计保留Chromium的Docker参数弃用提示导致的输出断言FAIL及实际stop0，续审计确认Chrome/Edge stop0。三者均无cleanupErrors，owner容器/网络/卷空、WSL lease停止且exit0，见`browser/route-and-r1-cleanup-result.json`与两份stop审计。Linux fast r1则自然完成23/23、契约221及全部Rust子集，在freeze2下保留完整PASS；它与Windows r3均为已被最终快照替代的证据，不充作freeze3验收。

最终收集器首轮把消费者先失败日志的Node spec reporter格式误按TAP读取，导致计数断言0≠5退出1；这是采集器格式错误，已在`reviewer/final-collector-format-failure.json`保存命令、旧收集器/日志摘要与原因。只修正该原始spec摘要的只读解析，严格核对tests5/pass0/fail5及零skip/cancelled/todo，原始日志字节和所有生产源码保持不变；最终门禁日志继续严格按TAP或各自声明格式检查。

## Actual Verification

| 检查 | 当前结果 | 证据/边界 |
| --- | --- | --- |
| 消费者最终r2专项 | PASS：97/97，零skip；SDK CHECK30通过 | `target/phase-08/consumers/result.json`；250项实际观察源/构建依赖前后相同，不与最终公共源码数量混算 |
| 宿主最终专项 | PASS：新增17、原context14、原故障域20，共51/51，零skip | `target/phase-08/host/result.json`；342观察输入前后相同，typecheck通过，相关ESLint零error/3条既有warning |
| Windows原权限套件 | PASS：64/64，零skip、exit0，原断言保持 | `target/phase-08/browser/windows-route-permissions-contract-r2.log`与`route-and-r1-cleanup-result.json`，源为最终478/b6e5ed… |
| Windows完整宿主/真实工单 | PASS：最终r4完整47/47，零skip、exit0；同源build:all及四对交付摘要通过 | `target/phase-08/browser/windows-result.json`与`windows-full-r4.log`；前后478项b6e5ed…一致，原29+新增18全部通过 |
| Linux fast | PASS：最终r2完整23/23步骤，mandatory零失败/跳过 | 契约221、部署36、CI策略49、dev/Vite19；Rust check及3/16/3/9/8单位回归，全部exit0 |
| Linux三产品完整矩阵 | PASS：Chromium/Chrome/Edge各169/169，共507，零fail/skip | 各8套件、相同478项b6e5ed…冻结；实际UA/能力、生命周期/入口/恢复/真实工单trace及源前后核验通过 |
| 资源与最终自检 | PASS：6项运行资源空查询，所有本阶段owner/lease已清理退出 | `target/phase-08/final-resource-audit.json`；逐引用SHA/37阶段状态/4项验收与下一指针见`target/phase-08/final-self-check.json` |

统一冻结输入已保存为[phase-08-inputs.json](phase-08-inputs.json)：478项公共源码，SHA-256为`b6e5edc5fd0642cdd4de241005dc125a7429396b3b0ae24a5f319e2841a20f4b`，相对473项起点变化45路径。`target/phase-08/boundary-check.json`逐项核验78项平台/业务backend、所有锁文件、legacy、wire schema/DTO/版本登记、部署及工具链字节不变；当前未发行候选SDK及其vendor是本阶段有意更新。宿主342/消费者250是各自观察依赖清单，不能与该478项公共输入合并为测试数量或源码总数。

## 实际命令、工具与产品矩阵

最终fast工具实际为Node v24.18.0、npm 11.16.0、rustc 1.97.0 (2d8144b78 2026-07-07)，Linux x64；Windows专项使用Node24.18.0/npm11.16.0。Linux验收由已授权的WSL Ubuntu隔离Docker执行，没有改动WSL全局配置。fast工具镜像固定为`sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7`，浏览器工具镜像固定为`sha256:66ac2ee1834bf8ccec2cfce67f5bd9f72a3382488ae170134fafc4cb493f3048`；这些是测试工具镜像，不是本阶段生产发行物。

| 产品 | 实际浏览器版本 | Playwright | 最终owner | 结果 |
| --- | --- | --- | --- | --- |
| chromium | 156.0.8078.4 | 1.64.1 | ci-1792068561265-28770c2d | 169/169 |
| chrome | 155.0.8059.79 | 1.64.1 | ci-1792068561365-5628fd7e | 169/169 |
| msedge | 155.0.4283.45 | 1.64.1 | ci-1792068561398-f6eb683c | 169/169 |

| 必需浏览器套件 | 每产品组数 | 产品数 | fail / skip |
| --- | --- | --- | --- |
| micro-host | 47 | 3 | 0 / 0 |
| notifications | 10 | 3 | 0 / 0 |
| platform-settings | 11 | 3 | 0 / 0 |
| regions | 12 | 3 | 0 / 0 |
| entities | 13 | 3 | 0 / 0 |
| route-permissions | 64 | 3 | 0 / 0 |
| departments | 11 | 3 | 0 / 0 |
| chat | 1 | 3 | 0 / 0 |

每产品169组是以上8个既有正式套件的计数，micro-host为原29+新增18。内部trace包含四个生产10秒生命周期截止、模块重求值、两个入口交错、十个恢复/拒绝场景、两个legacy场景，以及真实工单build和四类行为；trace不是额外测试组。Windows47、专项51/97、fast契约221和三产品重复执行存在覆盖交叉，不能相加宣称独立用例总数。消费者build2及两份static delivery各1仍是已有补充检查，未擅自加入正式计数。

本阶段最终实际入口如下；driver固定工具镜像、source manifest和owner，底层23个fast命令及原始日志逐项保存在[phase-08-runtime.json](phase-08-runtime.json)中。宿主/消费者专项的实际command、exit、TAP与输入依赖摘要也逐项绑定在同一JSON。

```text
npm run build:all
node tests/route-permissions-browser.mjs
node target/phase-08/run-windows-browser.mjs r4 b6e5edc5fd0642cdd4de241005dc125a7429396b3b0ae24a5f319e2841a20f4b
node target/phase-08/run-fast-linux-r2.mjs
node target/phase-08/run-browser-linux.mjs chromium D:/WorkSpace/rust/saas/rust-saas/target/phase-08/linux-browser-chromium-r2
node target/phase-08/run-browser-linux.mjs chrome D:/WorkSpace/rust/saas/rust-saas/target/phase-08/linux-browser-chrome-r2
node target/phase-08/run-browser-linux.mjs msedge D:/WorkSpace/rust/saas/rust-saas/target/phase-08/linux-browser-msedge-r2
node target/phase-08/collect-browser-matrix.mjs r2 b6e5edc5fd0642cdd4de241005dc125a7429396b3b0ae24a5f319e2841a20f4b
node target/phase-08/check-boundaries.mjs
node target/phase-08/audit-resources.mjs
node target/phase-08/collect-final.mjs
node target/phase-08/verify-final.mjs
git -c safe.directory=D:/WorkSpace/rust/saas/rust-saas diff --check
```

fast最终owner为`ci-1792068274241-842eaeaa`。每个浏览器容器限制4 CPU、无宿主挂载和公开端口、drop ALL与no-new-privileges；实际隔离信息、UA、能力、源快照和无剩余owner资源均在[浏览器矩阵](../../target/phase-08/browser-matrix-result.json)中。最终输入、runtime索引、原始结果和日志逐一核验SHA，6项只读资源查询覆盖saas.ci.owner与saas.phase08.owner两标签乘容器/网络/卷，未执行全局prune。fast及浏览器本轮临时缓存按owner清理，导出的证据与历史诊断继续保留。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 纯主题变化不取消业务提交、不重新请求列表；路由变化只影响相关读取 | PASS | 宿主/bridge/client同授权复用，真实Vue新建POST与编辑PATCH在切主题时完成；相关详情读取按路由取消、独立写保留；无update兼容应用纯主题保留实例并延后主题，路由仍按串行重挂fallback |
| 续期后旧响应不能回写，移除grant/member后旧内容不能继续操作 | PASS | 完整带类型授权投影、signal/epoch/期限共同判定；真实续期取消旧读写，旧响应/旧DTO不能污染新上下文；撤权清服务端快照并拒绝继续编辑 |
| profile/context激活或回滚竞态下资源入口与props版本一致 | PASS | context作为完整runtime/entry/version执行权威，profile触发一次复核；实际延迟入口/资源交错与激活、回滚均没有错配版本执行 |
| 临时503有界恢复且恢复前旧授权不可用；真实失权保持拒绝 | PASS | 503/网络错误白名单、500/1000/2000ms最多3次；每次token+context共5秒/过程20秒，旧授权立即失效；真实401/403/404、模式/契约错误及Phase7可信故障锁拒绝恢复 |

## Risks / Notes

旧token或已被业务后端接受的请求无法由浏览器追回；展示优化仍要求后端逐请求授权。可信执行故障须整页刷新，临时context恢复不能解除该锁。重试及生命周期deadline是调度上限，不是阻塞主线程下的硬实时中断。

未实现update的应用在纯主题变化时保留实例，主题延后到真正重挂；路由/授权仍沿用串行重挂兼容路径。当前工单和实现update的消费者支持相关读取增量更新。SDK1.0.0是未外部发行的仓库候选；保留的历史legacy字节、wire版本未改，已部署的旧静态bridge须通过新的前端版本接入这些行为。

真实浏览器加载当前宿主、SDK及Vue工单代码，平台HTTP/Socket.IO响应使用受控fixture。本阶段没有新增PostgreSQL数据库、OCI静态镜像、生产HTTPS、外部发行或生产部署验收，不以历史阶段结果冒充本阶段重跑。授权到期/撤销仍有后台实效边界；业务Rust迁移、工单Vite新UI、导航/跨页会话仍由后续阶段负责。

Linux fast的lint/构建及Rust检查已通过，日志保留既有warning（ESLint、Vite大chunk、Rust dead_code）；先前工程基线中的Rust fmt/Clippy等债务未在本阶段扩展修复，不能宣称仓库全量所有门禁均已通过。Phase7曾被策略拒绝删除的本地npm scratch仍保留且未重试/绕过，它与本阶段Docker运行资源全空是不同结论。

兼容/回退应将宿主、候选SDK与其consumer vendor作为同一前端变更组构建验证；不更改wire、后端、迁移或锁文件。不对活动生产版本执行回退或发布操作。

## Suggested Task Ledger Update

### Phase Status Update

Phase 8: COMPLETED。已按台账自身要求同步摘要、详情四项验收、证据、决策、变更记录和下一指针。

### Output Summary

- 宿主/bridge/client/现有UI共享展示与授权分类，稳定请求寿命、真实撤权清理；context完整入口权威与异步边界复核；fail-closed有界恢复。
- 宿主51/51、消费者97/97及SDK CHECK30；最终Windows47、Linux fast23步/契约221、三产品各169全部通过，零必需跳过。478项b6e5ed…最终输入与45路径变化、未变边界、原失败/旧快照及资源清理均可复核。

### Completed Items

- 四项Acceptance Criteria均PASS；README/架构/工程基线、候选SDK、CI必需入口、原始报告与汇总索引已同步。

### Remaining Items

- Phase8无未完成必需验收。Phase9及后续实施、外部发行和生产部署未执行。
- 旧无update生命周期和历史静态SDK的兼容边界、既有Rust债务及Phase7本地scratch保留已明确列出。

### New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 使用带类型完整授权投影，接受时缓存signature/source signal；同授权展示复用稳定controller | contextVersion或对象引用不足以区分真实授权变更与原位修改；单改UI不能控制bridge取消 | token相关比较仅在内存；旧响应和旧DTO拒绝，不增加wire字段 |
| context为完整运行入口权威，profile变化只触发一次复核 | profile/context会在激活或回滚时短暂错位 | 每异步边界与props前复核，绝不执行混合入口与版本 |
| 只对白名单临时传输/503恢复，5秒尝试/20秒过程、最多3次退避 | 无法证明授权时必须先拒绝，真实失权或永久执行故障不能被重试掩盖 | 每次重新获取token，取消及deadline拒绝迟到结果，Phase7故障锁不解除 |
| 以最终478项freeze3完整门禁收口，旧PASS与FAIL/143分别留存且不混算 | 两处旧fixture与新入口校验不匹配；完整成功也不能跨源快照继承 | 正式r2/Windows r4统一输入，下一阶段9 NOT_STARTED，无自动实施 |

### New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 无新的Phase8阻断问题 | 既有发行/生产环境及未来阶段问题保留在台账第7节 | 后续阶段负责人 |

### Next Recommended Phase

Phase 9: 统一菜单查询、路由策略与应用导航，NOT_STARTED。依赖4/6已完成；须新的明确执行请求，不自动推进。
