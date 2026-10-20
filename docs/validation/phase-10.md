# Phase 10 执行与验证证据

日期：2026-10-16（Asia/Shanghai）。阶段：Phase 10，协调多标签页的会话轮换与上下文。状态：**COMPLETED**。

## Executed Phase

用户明确要求执行Phase10，依赖Phase4、9均COMPLETED，指针10与详情一致。遵循task-ledger-executor及D:/WorkSpace/rust/AGENTS.md；台账要求同步摘要、详情、证据、决策和指针。只实施共享登录/共享当前租户组织的跨页协调，不实施Phase11或后续阶段、BFF/HttpOnly或每页独立组织。

实际起点493项公共源码，SHA-256 `ca3df48588648d3dd686bddded772f08bfa84393eb0b5254b47142f2b2090ca0`，与Phase9最终输入逐项相同。见`target/phase-10/baseline-inputs.json`、只读`baseline-source`和`baseline-docs`；HEAD保持`32956c4532bbd6eff53584fd0f5e9bb6e188437c`，此前未提交内容保留，不继承此前通过结果。

## Execution Summary

已将access/refresh保存为一份带版本、逻辑会话身份和操作intent的原子记录。Web Locks协调刷新、登录替换、退出和组织切换，锁内重读最新记录；通知只携带版本。一次性操作发送后结果未知或owner页关闭时，不重用旧refresh/context，隔离旧服务器视图并明确重新登录恢复。

## Deliverables

- 会话协调与API/store集成，单页刷新单飞/一次重试/旧响应丢弃保留。
- 登录/退出/密码修改/切组织及启动、能力/未知状态提示；各页请求/通知/旧组织视图同步退役，草稿保持app/tenant/org隔离。
- 平台显式Bearer调用携带捕获会话条件，防止共享记录已变更但旧闭包仍持Bearer时绕过校验。真实工单回归发现正常平台token轮换误重建iframe，容器issue改在捕获的逻辑会话/epoch内使用当前凭据，effect不再随access轮换重建；其它授权/执行/截止/故障域/生命周期/恢复逻辑保持，credential ticket及应用token语义独立。最终容器窄差异和失败轨迹单列核验。
- 真实同源双页、Chrome/Edge各最近两稳定major与能力缺失回归；保留既有9套183组及必需CI入口，最终数量以新冻结manifest及原始日志为准。

## Actual Verification

最终实现已停止公共源码修改，freeze1保存于[phase-10-inputs.json](phase-10-inputs.json)及`target/phase-10/freeze-1-inputs.json`：499项，SHA-256 `d52dd7d940611ab69266f23803656db56c5d6442a1205187cd336dd493cc9b44`。与493项实际起点比较36路径变化、6份新文件；没有扩大公共快照roots、私有排除或依赖范围。`check-boundaries.mjs`逐项确认78后端、10锁、54 SDK/wire、6个其它micro文件、28工单文件和35根部署/工具配置不变。容器七个精确替换重建当前完整文件，其余文本不变，不把Phase9内层raw摘要用于本阶段新容器。

| 检查 | 当前结果 | 证据边界 |
| --- | --- | --- |
| 核心会话专项 | PASS：39/39，原refresh9+新coordinator30，零skip | `target/phase-10/session/result.json`；11观察输入前后一致，typecheck及范围lint0error/0warning |
| 消费者集成专项 | PASS：21/21，伴随查询/通知22/22，零skip | `target/phase-10/integration/result.json`；16观察输入前后一致，范围lint0error/8既有warning |
| 原查询/导航/通知/路由专项 | PASS：62/62，零skip，5原测试文件字节不变 | `target/phase-10/fixtures/result.json`；纯模块/stub回归，不冒称跨页证明 |
| 真实双页定点 | PASS：17/17，原通知10/10，同499/d52输入前后不变 | `browser/current-r7`及`existing-notifications-r2`；相同场景已在最终完整矩阵重跑通过 |
| 冻结CI政策/本地入口 | PASS：政策52，refresh60/auth181/local contracts326，零skip | `ci/policy-result-freeze1.json`及`ci/local-freeze1/result.json` |
| Windows正式contract选择诊断 | 严格FAIL：362项/361PASS/1SKIP/0FAIL，exit0 | `ci/contract-selection-freeze1.json`；既有打包symlink在Windows EPERM，不能当Linux通过 |
| 最终Windows完整验收 | PASS：buildall及10套200/200，零fail/skip | 同499项freeze1，逐套退出0，原47/64/14及新增17轨迹完整 |
| 最终Linux fast | PASS：23/23步，零必需失败/跳过 | 契约362、部署36、政策52、dev19及Rust3/16/3/9/8，原始日志与完整source逐项核验 |
| Linux完整三产品矩阵 | PASS：Chromium/Chrome155/Edge155各200，共600 | 每产品10套，不混入旧稳定版子集 |
| Chrome/Edge154兼容矩阵 | PASS：各17，共34 | 同一新增会话套，独立干净安装/SDK检查/构建/真实能力与版本 |
| 资源与最终自检 | PASS：六空查、各owner与lease正常退出 | 37阶段与4验收一致、证据引用重哈希，历史记录未改 |

正式contract数量362=原311（已含原refresh9）+新coordinator30+integration21。root与独立reviewer核对真实选择日志、专项结果和完整输入后批准`ci/formal-counts.json`，只代表运行计数配置已审核，不代表Linux运行已PASS。旧local-r1源码漂移严格FAIL保留。

## 先失败、诊断与兼容记录

旧基线双VM与真实双页均复现同一旧refresh被请求两次；真实双页还复现另一页新登录被旧refresh覆盖，以及一页退出后兄弟页仍持旧内存会话。浏览器负例前期环境/fixture错误分别保存，不计真实行为失败。集成旧选21项FAIL包含新gate缺失及实际旧消费者闭包，不能把所有失败都描述为独立真实浏览器缺陷。

真实工单定点`current-r6`在正常平台refresh时稳定丢失未保存标题，虽同逻辑session/epoch且仅一次refresh；修复容器effect的access-token依赖后，`current-r7`验证同一iframe marker、同草稿及实际5秒应用token计划续期使用新平台凭据。原通知10首次定点还发现socket拒绝取消有效HTTP fallback；修复后完整10组通过，同key正常轮换回调不取消新observer。原断言保留，修复前后日志分别保存。

核心中间写入失败/时序回归和scope-lint误cwd、integration r3源漂移FAIL、r4旧18项PASS及后续21项、CI local-r1源漂移FAIL均保留。最终专项绑定各自观察输入，完整最终门禁绑定同一499项freeze1；不将旧结果反填为新组数或新源码。最终首轮矩阵聚合在自身stdout写完前记录其空文件摘要，被root严格引用校验拒绝；原矩阵、日志及collector-full-r1失败记录均保留。采用同一冻结聚合器在扫描目录外记录当前stdout后，从未变的完整r1原始结果重新生成索引；没有重跑或拼接部分套件，最终引用逐项重哈希通过。

官方Chrome/Edge两个稳定major为155/154，隔离工具镜像与四产品能力探针已实际完成，记录下载来源、包SHA、镜像digest及真实version/UA。工具安装/能力探针仍不替代正式多页功能验收；Chromium156也不充当Chrome第二稳定major。原Phase9已完成证据的34份历史文件摘要另存`historical-evidence.json`，旧JSON中的逻辑源码路径保留原执行语义，不递归当新Phase10源码哈希。

## 最终工具、命令与范围

最终fast实际工具为Node v24.18.0、npm 11.16.0、rustc 1.97.0 (2d8144b78 2026-07-07)，Linux x64。固定fast工具镜像为`sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7`；两浏览器工具镜像分别固定155为`sha256:7ad323a47d6d5202da34beb0f315d24dd10102d9bebd0726260c189768d0094c`、154为`sha256:07d9eefdfbfd939de554091d9db362a8d5ce1b97b4d5edaf698eee603f989bde`。它们是隔离测试工具，不是发行镜像。

| 产品 | 实际版本 | Playwright | 范围 | pass |
| --- | --- | --- | --- | --- |
| chromium | 156.0.8078.4 | 1.64.1 | 完整10套 | 200/200 |
| chrome | 155.0.8059.84 | 1.64.1 | 完整10套 | 200/200 |
| msedge | 155.0.4283.60 | 1.64.1 | 完整10套 | 200/200 |
| chrome | 154.0.8037.92 | 1.64.1 | Phase10兼容会话套 | 17/17 |
| msedge | 154.0.4258.62 | 1.64.1 | Phase10兼容会话套 | 17/17 |

Windows实际chromium 156.0.8078.4，Playwright 1.64.1，完整200组另计。完整三产品600与旧稳定版会话34分别验收，同一新增17组在Chrome/Edge两个major均通过；重复运行不相加为独立测试用例。原9套183保持，新增会话17后每完整产品200。SDK、后台、锁、工单原文件和根部署配置与实际起点一致；容器仅七个已记录替换，不能声称整个MicroMountZone原字节不变。

正式lint实际0 errors / 61 warnings；Vite大chunk、Rust既有警告与基线fmt/Clippy债务分列，不宣称全仓所有可选门禁均通过。真实浏览器的refresh计数指受控HTTP fixture对一次性旧值的接收/消费模拟，不等于本阶段对平台生产DB执行了刷新或组织迁移。

所有正式执行输入均为499项freeze1/d52dd7d9…，before/after与复制source完整一致，driver/counts SHA不变。浏览器各4CPU、无宿主挂载与公开端口、drop ALL及no-new-privileges；own容器/网络/卷空，lease退出0。最后六项只读查询覆盖saas.ci.owner和saas.phase10.owner两标签乘容器/网络/卷，无全局prune。本阶段保留官方浏览器工具镜像/安装包与原始证据，历史Phase7 scratch删除拒绝未重试。

实际命令及全部底层23步、raw日志、trace、安装诊断、owner和清理在[phase-10-runtime.json](phase-10-runtime.json)逐项绑定：

```text
node target/phase-10/run-windows-browser.mjs r1 d52dd7d940611ab69266f23803656db56c5d6442a1205187cd336dd493cc9b44
node target/phase-10/run-fast-linux-r1.mjs docs/validation/phase-10-inputs.json target/phase-10/ci/formal-counts.json
node target/phase-10/finalize-fast.mjs target/phase-10/linux-fast-r1 docs/validation/phase-10-inputs.json target/phase-10/ci/formal-counts.json
node target/phase-10/run-browser-linux.mjs chromium D:/WorkSpace/rust/saas/rust-saas/target/phase-10/linux-browser-chromium-r1 d52dd7d940611ab69266f23803656db56c5d6442a1205187cd336dd493cc9b44 full
node target/phase-10/run-browser-linux.mjs chrome D:/WorkSpace/rust/saas/rust-saas/target/phase-10/linux-browser-chrome-r1 d52dd7d940611ab69266f23803656db56c5d6442a1205187cd336dd493cc9b44 full
node target/phase-10/run-browser-linux.mjs msedge D:/WorkSpace/rust/saas/rust-saas/target/phase-10/linux-browser-msedge-r1 d52dd7d940611ab69266f23803656db56c5d6442a1205187cd336dd493cc9b44 full
node target/phase-10/run-browser-linux.mjs chrome D:/WorkSpace/rust/saas/rust-saas/target/phase-10/linux-compatibility-chrome-154-r1 d52dd7d940611ab69266f23803656db56c5d6442a1205187cd336dd493cc9b44 compatibility
node target/phase-10/run-browser-linux.mjs msedge D:/WorkSpace/rust/saas/rust-saas/target/phase-10/linux-compatibility-msedge-154-r1 d52dd7d940611ab69266f23803656db56c5d6442a1205187cd336dd493cc9b44 compatibility
node target/phase-10/collect-browser-matrix.mjs r1 d52dd7d940611ab69266f23803656db56c5d6442a1205187cd336dd493cc9b44
node target/phase-10/audit-resources.mjs
node target/phase-10/collect-final.mjs
node target/phase-10/finalize-docs.mjs
node target/phase-10/verify-final.mjs
git -c safe.directory=D:/WorkSpace/rust/saas/rust-saas diff --check
```

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 双页过期只消费一次旧refresh；失败/迟到页不覆盖新会话 | PASS | 真实同context双页、Web Locks锁内重读完整pair，旧成功/400/401与新登录交错均受fence限制 |
| 登录替换、退出/切组织、owner关闭及未知结果不误删新凭据 | PASS | 首次await前捕获逻辑身份、持久intent、真实page.close锁释放和网络未知；隔离后重新登录恢复、不重放旧值 |
| 双页同一组织，旧请求/通知停止，草稿不跨组织显示或提交 | PASS | 旧fetch/socket退役、不合作200不回显、旧handler零POST；真实工单A草稿不进入B，正常refresh及计划app续期保留同iframe/草稿 |
| 单页单飞/一次重试/旧响应丢弃保留，能力缺失真实验证 | PASS | 原9项与核心30、集成21及完整正式362通过；Chrome/Edge155/154均执行新增17，缺锁/storage拒绝、BC缺失与focus恢复真实通过 |

## Risks / Notes

本阶段不改变后端一次性refresh或context撤销协议。发送后未知结果采用保守隔离，不能用猜测超时锁或固定等待证明安全。Web Locks或原子storage不可用时明确拒绝共享会话；BroadcastChannel缺失的storage事件与恢复重读路径已在真实浏览器验证。旧分离token键仅受锁一次性导入，正式新会话不双写。产品兼容承诺绑定实际安装来源/版本，不把Chromium当Chrome第二稳定版，不声称生产数据库、发布或部署已验收。

采用时必须关闭或重载仍运行旧宿主bundle的标签页，旧代码不理解原子记录及Web Locks，不能宣称新旧宿主混跑也具备跨页协调。回滚旧宿主应重新登录，不拆分原子记录双写旧键；旧静态微应用/SDK不可变产物保持，未执行生产采用或回滚。

## Suggested Task Ledger Update

### Phase Status Update

Phase 10: COMPLETED。已同步台账摘要、详情四项验收、证据、决策、Change Log及指针。

### Output Summary

- 单一原子会话记录/Web Locks/版本通知、持久intent与unknown隔离，条件化登录/退出/切组织及请求回调；同组织正常轮换保留工单实例/草稿，真正上下文替换退役旧内容。
- 专项39/21、真实多页17、本地60/181/326、正式fast23/契约362/政策52、Windows200、Linux完整600及兼容34均通过；499项/36变化路径、六资源空查和历史证据闭合。

### Completed Items

- 四项验收全部PASS，原9浏览器183保留并新增17；兼容策略、采用/回滚、源码和原始失败记录齐全。

### Remaining Items

- Phase10无未完成必需验收。旧宿主标签页需重载/关闭，回滚旧宿主需重新登录。
- 正式SDK发行、Firefox/Safari、生产环境和既有Rust债务仍见台账开放问题；不属于本次已完成声明。

### New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 单原子record与真实Web Lock，共享登录和组织 | 分离键和每页单飞无法保护一次性refresh | 通知只版本，旧键单向迁移，缺锁/storage明确拒绝 |
| 持久intent与unknown隔离 | 已发送请求/关闭页面无法证明服务端未提交 | 不重放旧refresh/context，新登录恢复，不修改后端协议 |
| 调用期逻辑fence与渲染期容器身份 | 等锁后重捕身份会串用旧草稿，access轮换不应卸iframe | 请求/通知旧结果退役，正常续期保留实例，真实切换卸载 |
| 两稳定major分列实测，旧宿主页重载采用 | Chromium不是Chrome第二稳定版，旧bundle不认识新协议 | 官方155/154版本与来源绑定，兼容34不冒充完整600 |

### New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 无新的Phase10阻断；原Q05剩正式SDK发行与Firefox/Safari等问题 | 最近两个stable major本轮已测，未来产品支持不据此虚报 | 后续阶段负责人 |

### Next Recommended Phase

Phase 11: 修复普通 OAuth 原子签发与实时失权，NOT_STARTED。须新的明确执行请求，本次不自动推进。
