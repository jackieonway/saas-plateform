# Phase 7 执行与验证证据

日期：2026-10-15（Asia/Shanghai）。执行阶段：Phase 7，隔离可信微应用永久挂起的故障。状态：**COMPLETED**，三项验收均PASS；本文件记录本阶段实际运行，不继承Phase6通过结果。

## Executed Phase

用户明确要求执行Phase7。依赖Phase4、6均COMPLETED，台账与指针一致。遵循 `task-ledger-executor` 与 `D:/WorkSpace/rust/AGENTS.md`；台账自身要求每阶段同步状态、证据、决策及下一指针。

Git HEAD保持 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，保留Phase1–6与用户未提交内容。本次起点472项公共源码摘要为 `9c967cda2b728de9f675f8537ce2c6cb3279bd4026020c77a081b550c9a0e576`，记录在 `target/phase-07/baseline-inputs.json`。它只标识起点，不能充当本阶段测试通过证据。

## Execution Summary

起点 `MicroLifecycleSession` 对load/mount/update/unmount无限等待，容器的全局 `disposalBarrier` 又让全部后续实例等待旧实例。永久pending的可信生命周期因此同时阻塞另一个可信应用与可独立销毁的iframe。

本阶段仅增加可信生命周期截止、页面可信故障锁存、运行模式调度分离及受控重载恢复。保留正常串行、即时授权取消、opaque iframe/MessageChannel与SDK1.0.0；不实现Phase8展示/授权更新语义、不改后台、业务数据库、公开SDK协议或依赖，不进行外部发行/生产部署。

## Deliverables

- `runtime.ts`：可信页面域串行队列，四阶段分别10秒执行预算，排队不消耗下一操作预算；超时永久锁存，最终unmount reject或宿主cleanup throw保守锁存OPERATION_FAILED。stop立即取消交付，不能用取消证明在途执行已结束；迟到结果不复活、不解锁且拒绝被观察。
- `MicroAppContainer.tsx`：trusted-only最终卸载barrier，iframe不等待；故障清空当前snapshot并取消entry/props授权，跨组件保持故障卡片。普通重挂禁用，明确“重新加载宿主”才更换document。
- `loader.ts`：实际execScripts执行Promise不再被abort race掩盖，执行前仍检查取消，保持原加载缓存/静态安全边界。
- 新确定性runtime故障回归、本地/正式CI接入及真实浏览器四阶段永不完成、iframe/可信边界、晚结果、切组织与整页恢复场景。最终输入见 [phase-07-inputs.json](phase-07-inputs.json)，机器汇总与原始证据SHA见 [phase-07-runtime.json](phase-07-runtime.json)。

## 发现、修正与失败保留

1. 旧实现的真实Chromium挂起mount等待18秒仍未取消signal，负例日志在 `target/phase-07/browser/old-runtime-negative-mount.log`，两个原生产文件SHA在 `negative-inputs.json`。这是旧源码+新增场景负例，不能把起点完整输入清单混称为该负例的完整快照。
2. Node旧实现负例17项FAIL记录于 `target/phase-07/runtime/old-runtime-negative.log`；首项直接证明deadline后pending mount未结束。随后真实锁定import-html-entry求值/导出thenable证明loader取消race不能证明脚本结束，已增加实际wrapper回归。
3. 新stage包装在load后/实际调用前多出微任务空隙，独立探针复现取消后仍mount/update；`microtask-race-negative.log`保留先失败回归。修复在实际执行前复核session/domain/捕获signal，并只在真正开始mount时标记mounted；19项中间回归包含76种微任务取消时点。该问题属于本阶段实现审查修复，不归因为旧基线已存在。
4. Windows第一轮新增导航fixture曾用所有应用token签发总数归因另一个TRUSTED，实际差值来自iframe初始路由重建；改为按appCode记录计数，保留固定安全断言。完整28组首轮随后因在iframe挂载的base→detail导航完成前再次选择应用而没有到达目标TRUSTED路由失败；改为等待目标真实URL及选中状态后再执行下一步，原快速选择专项保持。
5. 实际Windows HMR及独立同realm模块重新导入探针表明：模块级domain会在热替换后重建为healthy，让旧pending存在时启动另一个TRUSTED。这是开发HMR中的Phase7必修边界，不能仅以“测试污染”关闭；生产bundle没有该HMR路径。已解除首个473项 `d4e762…` 冻结，修为跨模块保留页面domain/队列/fault并补回归。基于旧冻结的正式fast/browser r1按精确owner核验后受控停止，原始部分报告/退出码/清理均保留，不作为最终验收通过。
6. 新增场景的导航helper现等待实际目标应用状态栏对应节点，原子核对URL、选中应用、opacity为1及位移归零。r4虽29组通过，trace仍有旧节点/未完成动画采样，故最终证据采用r5；原有快速选择专项未改。本次只修正测试步骤的就绪观察，没有修改AppLayout或宣称完成Phase9生产导航优化。

## Actual Verification

### 最终验证记录

| 检查 | 当前结果 | 证据 |
| --- | --- | --- |
| 最终runtime专项 | PASS：新增20/20、原micro-context14/14，零skip；20项中包含76个微任务时点与跨模块/新realm/非法占位边界 | `target/phase-07/runtime/result.json`，10项实际输入前后相同 |
| 静态检查 | PASS：宿主typecheck、runtime eslint零warning；容器/loader eslint零error、4条既有warning | `target/phase-07/runtime/`、`container-{typecheck,eslint}-r2.log` |
| Windows真实挂起专项 | PASS：load/mount/update/unmount四组，使用生产10秒截止 | `target/phase-07/browser/windows-hanging-r3.log` |
| Windows完整micro-host | PASS：最终r5全部29组、零skip，5份成功trace原子绑定实际目标节点；前后473项摘要相同 | `target/phase-07/browser/windows-full-r5.log` |
| 首轮正式Linux fast/browser | 保留FAIL，因必修页面domain修复撤销d4冻结而按owner停止；导出为PARTIAL，不计最终验收 | `target/phase-07/linux-*-r1/`及controlled-stop记录 |
| 最终完整Linux fast r2 | PASS：23/23步，契约189、部署36、CI49、dev/Vite19；Rust config/archive/maintenance/public/transport分别3/16/3/9/8，零fail/skip | `target/phase-07/linux-fast-r2/`；exit0、输入前后相同、owned cache移除及owner资源空查均通过，lease退出0 |
| 最终完整Linux三产品 r2 | PASS：Chromium/Chrome/Edge各151组，共453 PASS、0 FAIL、0 SKIP | `target/phase-07/browser-matrix-result.json`；各轮exit0、cleanupErrors为空、输入相同、owner资源空查及lease退出0 |
| 最终源码/资源审计 | PASS：473项最终输入，11路径变化；后端/SDK/业务/锁零漂移；6项Docker标签查询均为空 | `phase-07-runtime.json`、`target/phase-07/final-resource-audit.json`；审计自己的lease也退出0 |
| 台账/证据最终自检 | PASS：37阶段摘要/详情一致，Phase7三项PASS，下一指针8 NOT_STARTED，84份证据引用SHA相同；HEAD不变、diff检查退出0 | `target/phase-07/final-self-check.json`；diff检查仅提示已有CRLF规范化信息，无空白错误 |

同一行为在不同操作系统、重复子集或不同快照的计数不得相加为唯一测试总数。当前工具仍为Node24.18.0/npm11.16.0、Rust1.97.0、Playwright1.64.1，WSL Ubuntu Docker29.1.3；固定fast工具镜像42e1…与浏览器镜像66ac…只提供隔离测试环境，不是新交付的应用镜像。

最终冻结输入：[phase-07-inputs.json](phase-07-inputs.json)，共473文件，SHA-256为 `4a2f9ce6340c516fa5cee95e8a886c4c0a440febd3869b7b8cb434f03d3da611`。相对472项起点仅11路径变化：宿主runtime/container/loader与package测试脚本、CI manifest/README/入口断言、浏览器manifest/README/micro-host及新runtime故障测试。collector已逐项复核73项backend、42项SDK及全部业务消费者/锁文件与起点相同；最终fast、Windows和三产品的实际快照/前后输入均与当前473项相同。本阶段未运行PG/静态镜像或对外发行。

| Linux正式运行 | 实际版本 | PASS / FAIL / SKIP | Owner |
| --- | --- | --- | --- |
| fast r2 | Node24.18.0 / npm11.16.0 / Rust1.97.0 | 23步PASS，必需失败/跳过均0 | `ci-1792062977350-7239484a` |
| Chromium r2 | 156.0.8078.4 / Playwright1.64.1 | 151 / 0 / 0 | `ci-1792063103347-c2e276ec` |
| Chrome r2 | 155.0.8059.79 / Playwright1.64.1 | 151 / 0 / 0 | `ci-1792063103466-9c965d7a` |
| Edge r2 | 155.0.4283.45 / Playwright1.64.1 | 151 / 0 / 0 | `ci-1792063103572-65803fae` |

每个产品均执行同样8套件：micro-host29、notifications10、platform-settings11、regions12、entities13、route-permissions64、departments11、chat1。每个实际产品均留下4份四阶段生产10秒截止trace、1份模块重求值trace及13份原子导航就绪采样；实际UA、安全上下文及client/iframe/host能力另行观察，不替代151组原正式套件。Windows最终完整29组使用Chromium156.0.8078.4。

正式门禁使用未改写的项目入口，完整命令与计数记录于各轮summary：

```sh
node scripts/ci/run.mjs --gate fast --out-dir /reports/fast --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c
node scripts/ci/run.mjs --gate browser --out-dir /reports/browser --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --browser chromium
node scripts/ci/run.mjs --gate browser --out-dir /reports/browser --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --browser chrome
node scripts/ci/run.mjs --gate browser --out-dir /reports/browser --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --browser msedge
```

fast固定镜像为 `sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7`，浏览器固定镜像为 `sha256:66ac2ee1834bf8ccec2cfce67f5bd9f72a3382488ae170134fafc4cb493f3048`。实际外部driver位于 `target/phase-07/`，管理本轮WSL stdin lease、原始runLinux、前后输入核对与owned reports/work缓存移除；没有改正式命令/通过条件，没有宿主home、源码、缓存或Docker socket挂载。浏览器追加真实UA/三模式能力观察，不能替代原151组。通用干净重跑流程见 [CI README](../../scripts/ci/README.md)，输出必须选择新目录。

最终collector重新解析fast原始TAP/Cargo日志，核对23个精确步骤ID和各组计数；核验runtime20+14及3份先失败证据、各产品矩阵和Windows最终日志SHA，检查所有当前输入边界。fast r2的11份脱敏npm debug日志与native依赖观察同时归档。报告、驱动和日志引用均记录SHA-256，原始本机运行目录位于`target/phase-07/`，未发布外部渠道。

四轮最终runner均exit0且清理无错误，逐owner容器/网络/卷均为空，WSL lease正常停止退出0。之后根审计只读查询`saas.ci.owner`及`saas.phase07.owner`两种标签下的容器/网络/卷，6项均为空；未执行全局prune或修改WSL全局配置。r2各自容器内owned reports/work缓存先移除再复制报告；中断r1留存的4个本地work目录单独列于机器汇总，不计作已清理。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| mount、update、unmount各自永不完成时进入稳定故障，token/signal取消 | PASS | 同时覆盖load；Node20项覆盖确定性截止/取消，Windows/三Linux产品实际使用生产10秒截止，清空当前token快照并abort。只表示逻辑取消，不宣称终止原任务 |
| 后续iframe可挂载，另一TRUSTED不能绕过安全barrier | PASS | iframe在旧可信截止前可挂载；新可信脚本被阻止。跨组件/同realm模块重求值保留活动队列与故障，只有完整document重载恢复 |
| 迟到resolve/reject、快速切组织/应用及正常最终卸载通过 | PASS | Node新增20项、原micro-context14项通过；Windows与三Linux产品的micro-host各29组通过。76个取消微任务时点、迟到拒绝观察与原快速选择专项保留 |

## Risks / Notes

可信代码与宿主处于同一JavaScript执行域。deadline只能诊断、取消宿主持有的授权工作并禁止复用该可信域；它不能证明原Promise终止，不能撤回已经泄露或已发出的token，也不能打断同步无限循环。整页重载建立新执行域；若同步代码阻塞事件循环，需使用浏览器级刷新。iframe自身的销毁边界保持独立，但不把iframe称为能够阻止任意可信脚本对宿主造成影响。

10秒为每阶段的调度预算，排队等待不消耗下一阶段预算；事件循环阻塞或浏览器后台调度会推迟计时器执行，不构成硬实时保证。故障信息只保留固定code/stage/reason，不回显任意可信应用错误正文；现有后台runtime-event协议未扩展到新stage，页面诊断与后续观测阶段分别管理。

首次将仍运行旧module-only runtime的开发页面升级到页面domain方案时，须先整页刷新。新实现不能追溯取得旧模块私有队列；本阶段新测试从新document启动，再验证新实现之间的同realm模块重新求值保留同一domain。真实浏览器验证的是该边界，不宣称涵盖全部Vite HMR网络传输组合。

真实浏览器验证使用Vite/React及受控API/Socket.IO fixture；不代表生产鉴权或业务服务迁移。Phase7没有后端变更，后端输入与既有迁移已逐项核对相同，不用Phase6历史PG报告冒充本阶段重跑。原有全仓Rust fmt/严格Clippy债务继续按Phase6披露，本次通过的是项目声明的选定fast门禁；其他浏览器版本、Firefox/Safari与新生产HTTPS/静态镜像未验收。

中断fast r1的 `target/phase-07/linux-fast-r1/reports/fast/work` npm临时缓存因自动执行策略拒绝递归删除而保留，原始原因仅为 `blocked by policy`，详见该轮 `controlled-stop.json`。已核实路径属于本次target且无reparse point，没有改用其他shell绕过限制。它与容器/网络/卷/lease清理分别报告，不冒称scratch全部删除。

## Suggested Task Ledger Update

### Phase Status Update

Phase 7: COMPLETED。已按台账自身要求同步摘要、详情三项验收、证据、决策、变更记录及下一指针。

### Output Summary

- 四阶段可信生命周期截止、页面可信队列/故障锁存、iframe独立销毁和明确整页恢复UI；loader等待真实执行、调用前复核取消、跨模块稳定域。
- runtime20+14、Windows29、Linux fast23步/契约189及三产品各151通过；473项最终输入、11路径变化、后端/SDK/业务/锁不变，原失败与清理例外保留。

### Completed Items

- 三项Acceptance Criteria均PASS；文档、CI入口、正式浏览器矩阵与可复核证据已同步。

### Remaining Items

- Phase7无未完成必需验收。本地r1公共npm临时缓存按策略限制保留；不影响运行资源清理或三项功能验收。
- 后续阶段、外部发行和生产部署未执行。

### New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 四阶段执行各10秒；无法证明结束则永久锁存可信页面域，完整重载恢复 | 取消race与超时不能杀死真实脚本，队列也不能在模块重求值后重置 | iframe独立销毁；另一个TRUSTED继续fail closed |
| 实际调用前复核取消，实际脚本Promise完整受控，故障类型跨模块识别 | 真实thenable、微任务空隙和HMR探针均先复现缺陷 | 保留正常串行/最终卸载，无SDK或后端协议改版 |
| 以最终统一473项r2收口，保留中断r1与scratch策略拒绝 | 不合并不同快照/不完整套件或改写原失败 | 本机证据与生产/外部发行明确分列 |

### New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 无新的Phase7阻断问题 | 既有生产环境、版本矩阵与未来更新语义仍由对应阶段管理 | 后续阶段负责人 |

### Next Recommended Phase

Phase 8: 分离展示更新、授权续期和版本重挂，NOT_STARTED。依赖6/7已完成，须新的明确执行请求，不自动推进。
