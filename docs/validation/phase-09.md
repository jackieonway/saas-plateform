# Phase 9 执行与验证证据

日期：2026-10-15（Asia/Shanghai）。阶段：Phase 9，统一菜单查询、路由策略与应用导航。状态：**COMPLETED**。

## Executed Phase

用户明确要求执行Phase9，依赖Phase4、6均COMPLETED，执行指针9与阶段详情一致。遵循task-ledger-executor和D:/WorkSpace/rust/AGENTS.md；台账自身要求每阶段同步摘要、详情、证据、决策和指针。本阶段不实施Phase10跨页会话或后续业务迁移。

实际起点为478项公共源码，SHA-256 `b6e5edc5fd0642cdd4de241005dc125a7429396b3b0ae24a5f319e2841a20f4b`，与Phase8最终输入逐项相同；见`target/phase-09/baseline-inputs.json`与只读`baseline-source`。HEAD保持`32956c4532bbd6eff53584fd0f5e9bb6e188437c`，此前未提交内容全部保留。起点及此前通过不作为本阶段验收证据。

## Execution Summary

将授权菜单查询的key/options/epoch/signal与失效规则统一为共享工厂，使导航入口和直达守卫共用类型化策略；应用选择通过代次和当前会话复核拒绝迟到跳转。工单创建路径选定`/micro/work-orders/ticket-create`，与详情`/tickets/:id`分离，并同步当前前端、manifest和校验器。

保留全局cancel/clear和旧epoch拒绝；旧key分裂作为一致性与重复请求问题处理，不宣称已经证明跨租户漏洞。菜单授权和data grant继续独立，未知授权拒绝。不改变SDK/wire、后端数据模型/历史迁移、锁文件、生产发行或部署。

## Deliverables

- `auth-queries.ts`集中profile/menus/permissions的key、20秒freshness/refetch、有界读取重试与signal；请求前后复核epoch、身份、应用授权与取消。`use-auth-data.ts`的单应用、聚合与命令式调用共享工厂；同epoch正常token轮换保持缓存身份，失效前缀及组织切换cancel/clear保留。
- `route-permissions.ts`声明类型化路由，`route-policy.ts`统一展示与守卫。个人工作台、个人中心和本人通知是登录用户页面；宿主页要求已知permission与精确MENU，平台根限定保持；micro只认本应用显式模式，embed认精确URL菜单，未知拒绝。成功匹配优先于无关应用错误，error保留的旧data不能继续授权。
- `application-navigation.ts`与共享hook在rail、工作台、命令面板、最近访问和直达意图间共用选择代次；旧A结果、A→B→A首轮A、手动路由及旧会话结果不再改变新意图。当前微应用菜单按需，工作台按实际候选路径所属应用加载，命令面板打开后才查全应用菜单；sidebar/TopBar/quick actions均使用同一策略。
- 工单创建统一为`/micro/work-orders/ticket-create`，详情`/tickets/:id`保持独立。旧`/tickets/new`仅作为详情标识new，不打开创建表单。bootstrap保留七个稳定key和UUID映射，read/create分别转为隐藏详情MENU和可见创建MENU，四个其它动作BUTTON保留；校验器、授权计划、SQL形状守卫和三份组件README同步。旧菜单冲突在写入前停止，采用和回滚需人工核对，不执行自动授权或数据库修改。
- 正式CI接入新增query/policy/guard/navigation回归与原有管理员/授权工具测试。六份既有公开部署工具依赖首次精确纳入源码快照，sourceRoots与排除规则保持，未知文件、私有配置和业务DB没有扩大纳入。该范围扩展和原478项起点分别记录。

## 先失败与诊断记录

`auth-queries/before-final.log`对只读基线运行同一组3项回归，实际3FAIL，覆盖key分裂、信号缺失及不合作传输的旧epoch结果。策略旧守卫在受控hook下为3FAIL/3PASS，覆盖无关503阻断已知页面、未知路径被放行和个人工作台被permission错误阻断。这些是定点行为/组件诊断，不冒称完整基线门禁重跑。

独立`reviewer/create-overlap-baseline.json`从478项实际保存源加载真实路由匹配与工单domain，证明详情read MENU会放行旧tickets/new，而旧domain将其解释为create；ticket-create在相同详情菜单下不获准。`reviewer/dashboard-navigation-baseline.json`用TypeScript AST提取保存源中的真实enterApp、注入受控菜单Promise，记录B先返回后A晚返回导致B/home→A/home；属于真实函数控制流探针，不计浏览器。

创建子任务的新回归在修改前有效执行68项中7FAIL；更早一次fixture断言错误另存诊断。管理员工具原本不属于478项快照，补充来源`creation/baseline-deploy/provenance.json`明确区分五份HEAD规范化重建文本（初始clean status及本人改动核对）和两份直接复制，不能将其说成原始478项字节的直接捕获，也不以重建文本替代当前工具真实执行。

浏览器保存两项真实负向：完整基线frontend只读覆盖复现关闭命令面板仍加载其它应用且同一system出现两份有效菜单请求；单独旧Dashboard覆盖复现真实A晚响应将URL从B拉回A。各自的源码/实际加载覆盖与构建输入摘要独立记录。中间并行Vite诊断的动态import失败、最近访问标题fixture预期错误都原样保留，改为串行并按实际path验证，不作为功能通过证据。

原route64首轮在56组通过后等待旧“无访问权限”文本失败：Phase8的401/403/404退出逻辑本就导航到`/`，新个人dashboard策略使出口成为个人工作台。`MicroAppContainer.tsx`两侧SHA均为`436c89ac1e35c910b8fa43f33f3bbd4b1ead884df24922a11fd7485d7e8a47a7`，生产文件未改。两组live-context/revocation精确改验`/dashboard`及工作台可用，并保留context拒绝、entry0、卸载与entry不增长；另1组dashboard直达改为个人工作台正向，5组sidebar布局正向补齐明确system:organization权限，权限拒绝负向保留。原64组数量不变，修后preliminary64/64、零skip、exit0；它仍须在最终冻结完整门禁中重跑。

Windows一次prefreeze contracts选择实际308PASS、1SKIP、0FAIL、exit0：跳过为既有打包器符号链接测试的EPERM，收集器严格记为FAIL，不削弱必需零skip规则。该轮当时309项，后续新增一项跨实例共享代次回归后，声明先变为310；不能把旧309轮次改写成310成功。正式Linux须完整执行并零skip。

freeze1为493项`b93769f4…`、38路径变化。首次完整Windows r1在buildall通过、micro-host实际21组通过后，于原Phase7永久unmount场景失败：新应用probe-static已被选中，61ms后旧isolated又写回selected，目标导航被阻止，实际URL留在isolated/detail。原47组测试和预算没有改，源前后相同，原始FAIL/21PASS及日志完整保存。随后两次加setter栈的窄诊断分别1PASS与4PASS，未重现该时序，保留其PASS诊断，不冒称失败已消失。

真正的确定性负例提取实际MicroAppContainer重复route→selection effect，与真实共享选择事务和held菜单组合：先选择next后重放旧route effect，实际最终为old且无导航；去掉重复写入则导航next。删除子层重复selector/effect，由AppLayout唯一负责epoch/path/匹配owner ID推断，selected-ID骨架门禁与MicroMountZone/授权/生命周期代码保留；并补同epoch新增授权及重新授权的父层正向边界，避免无匹配时永久记住旧推断状态。现有卸载、旧路由回写和晚grant断言纳入16项导航回归，最终正式contracts相应为311。每次负例的实际测试/源字节和旧PASS元数据分别归档，不将当前测试摘要反填进旧执行记录。

freeze2为修复后的493项`551d1efb…`、39路径变化。冻结与一条过期ESLint注释清理并发，Windows r2和CI预检实际观察到`e56f503d…`，均在任何build/browser/policy执行之前正确拒绝；原始退出1及完整输入对照保留，没有Linux资源或部分测试被改写为通过。选择保留已清理的源码，未为迎合摘要恢复过期注释。全部owner停止源码修改后重新冻结为freeze3：493项`e56f503d13fd0892f18216495644fde692d305a75fc84a3cfd393c2e8406301c`，与freeze2仅AppLayout该注释差异。

Windows r3完成buildall与原micro-host前25组，其中Phase7四截止和模块重求值均PASS，随后真实Vue创建流程已正确导航ticket-create，但Phase8 addon的一处`routePath('new')`仍等待旧tickets/new，因陈旧期望超时退出1。只同步该等待和之前的MENU路径两处字面量，整份addon其余字节/四组行为断言不变；全面核对活动创建引用后，原4组真实工单专项实际4PASS/0skip、exit0，见`browser/canonical-work-orders-result.json`。freeze4相对freeze3仅此测试文件变化，最终493项`ca3df48588648d3dd686bddded772f08bfa84393eb0b5254b47142f2b2090ca0`、39路径（含6范围扩展），正式完整门禁使用freeze4；此前三轮Windows结果/拒绝及各冻结输入均保留，不混算计数。

## Actual Verification

| 检查 | 当前结果 | 证据边界 |
| --- | --- | --- |
| 查询专项 | PASS：新增13+原session refresh9，共22，零skip | `target/phase-09/auth-queries/result.json`；19项观察源/依赖前后相同 |
| 路由策略专项 | PASS：纯策略18+实际守卫6，共24，零skip | `target/phase-09/policy/result.json`；旧守卫真实3FAIL/3PASS独立保存 |
| 工单路由与工具 | PASS：client19/app15/build2/admin21/grants13，共70，零skip | `target/phase-09/creation/result.json`；25项观察输入前后相同，build2属于补充检查 |
| 新本地授权入口 | PASS：`npm --prefix frontend run test:auth`实际121/121、exit0、零skip | `target/phase-09/ci/local-auth-entry-freeze4.json`；确认frontend cwd调用成立，源绑定493/ca3df485…保持 |
| 导航专项 | PASS：最终16/16，零skip；typecheck通过，范围lint零error/3条既有warning | `target/phase-09/navigation/result.json`；174项观察输入前后相同，旧源码/卸载/子层回写/晚授权负例留存 |
| Windows完整浏览器 | PASS：最终r4 buildall及9套183/183，零fail/skip，全部实际exit0 | `target/phase-09/browser/windows-result.json`；493项freeze4前后相同，逐套日志/版本/Phase7–9 trace齐全 |
| Linux fast | PASS：最终r1完整23/23，零必需失败/跳过 | contracts311、部署36、CI策略51、dev/Vite19、Rust check及3/16/3/9/8单位回归，均exit0 |
| Linux三产品完整矩阵 | PASS：Chromium、Chrome、Edge各183/183，共549次执行，零fail/skip | `target/phase-09/browser-matrix-result.json`；各9套、同493项freeze4，真实UA/能力/行为trace及源码前后核验 |
| 资源与最终自检 | PASS：六项运行资源查询空、所有本阶段owner/lease退出 | `target/phase-09/final-resource-audit.json`及`final-self-check.json`；三项验收、37阶段与证据摘要逐项一致 |


最终输入见[phase-09-inputs.json](phase-09-inputs.json)：493项，SHA-256 `ca3df48588648d3dd686bddded772f08bfa84393eb0b5254b47142f2b2090ca0`；与478项起点对照39路径变化，包含6份既有工具首次纳入。`boundary-check.json`确认78项平台/业务后端、全部锁、SDK/wire/legacy、现有session API/store、根deploy/、Compose与工具链保持原字节。微应用容器仅删除外层重复选中selector/effect；其余完整文本经规范化逐项相同，MicroMountZone及后续执行代码原字节摘要为`570a0287fb985e04d44c64cddb445de4766e931563364d9e8ab3d82cbf37bc97`，Phase7截止/故障锁与Phase8授权/恢复逻辑保留。19/25/174观察依赖是专项输入清单，不能与493项公共输入或测试数量混算。

## 实际命令、工具与口径

最终fast工具实际为Node v24.18.0、npm 11.16.0、rustc 1.97.0 (2d8144b78 2026-07-07)，Linux x64；Windows使用Node24.18.0/npm11.16.0。已授权WSL Ubuntu隔离Docker执行，无WSL全局配置修改。fast工具镜像固定`sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7`，browser工具镜像固定`sha256:66ac2ee1834bf8ccec2cfce67f5bd9f72a3382488ae170134fafc4cb493f3048`，均为测试工具镜像。

| 产品 | 实际版本 | Playwright | 最终owner | 结果 |
| --- | --- | --- | --- | --- |
| chromium | 156.0.8078.4 | 1.64.1 | ci-1792073723067-973082b3 | 183/183 |
| chrome | 155.0.8059.79 | 1.64.1 | ci-1792073723145-1e407f45 | 183/183 |
| msedge | 155.0.4283.45 | 1.64.1 | ci-1792073723178-2d16612b | 183/183 |

| 必需浏览器套件 | 每产品组数 | Linux产品数 | fail / skip |
| --- | --- | --- | --- |
| micro-host | 47 | 3 | 0 / 0 |
| notifications | 10 | 3 | 0 / 0 |
| platform-settings | 11 | 3 | 0 / 0 |
| regions | 12 | 3 | 0 / 0 |
| entities | 13 | 3 | 0 / 0 |
| route-permissions | 64 | 3 | 0 / 0 |
| phase9-navigation | 14 | 3 | 0 / 0 |
| departments | 11 | 3 | 0 / 0 |
| chat | 1 | 3 | 0 / 0 |

每产品183=原169+新增Phase9导航14；Windows同样完整执行9套183，原micro-host47与route64数量不变。实际轨迹验证查询共享、命令面板按需加载、选择交错/手动导航、已知授权与无关错误、平台根/个人/未知策略、工单创建及不合作旧组织200迟到回包；Phase7/8生命周期/授权恢复/入口和真实工单trace保留。

菜单请求收益以真实请求观察说明：旧源会在命令面板关闭时请求无关A/B，并对system建立两份有效查询；新源关闭时仅当前应用有效请求，面板打开后才补其它应用，同app/epoch共享一个查询/缓存。React StrictMode可能先发出并取消首个原始HTTP尝试，因此计数明确区分raw attempts、响应前已取消和有效未取消请求，不承诺原始HTTP启动次数恒为1。API fixtures与观察详情在矩阵的navigationSemantics中。

contracts311相对旧221增加90：查询13、策略/守卫24、导航16、工单client/app净增3、原管理员/授权工具首次纳入34。后34项并非新建测试。专项22包含原会话刷新9，creation70包含build2补充检查，Windows/Linux也重复覆盖同用例，不能相加为独立用例总数。

实际入口如下，23个fast底层命令及全部日志、专项命令/退出/观察输入和历史诊断均在[phase-09-runtime.json](phase-09-runtime.json)逐项引用。Windows先同源buildall并验证四对served/canonical字节，再执行全部套件；Linux每产品也从同一干净快照安装/检查/构建。

```text
npm run build:all
node target/phase-09/run-windows-browser.mjs r4 ca3df48588648d3dd686bddded772f08bfa84393eb0b5254b47142f2b2090ca0
node target/phase-09/run-fast-linux-r1.mjs docs/validation/phase-09-inputs.json
node target/phase-09/finalize-fast.mjs target/phase-09/linux-fast-r1 docs/validation/phase-09-inputs.json
node target/phase-09/run-browser-linux.mjs chromium D:/WorkSpace/rust/saas/rust-saas/target/phase-09/linux-browser-chromium-r1 ca3df48588648d3dd686bddded772f08bfa84393eb0b5254b47142f2b2090ca0
node target/phase-09/run-browser-linux.mjs chrome D:/WorkSpace/rust/saas/rust-saas/target/phase-09/linux-browser-chrome-r1 ca3df48588648d3dd686bddded772f08bfa84393eb0b5254b47142f2b2090ca0
node target/phase-09/run-browser-linux.mjs msedge D:/WorkSpace/rust/saas/rust-saas/target/phase-09/linux-browser-msedge-r1 ca3df48588648d3dd686bddded772f08bfa84393eb0b5254b47142f2b2090ca0
node target/phase-09/collect-browser-matrix.mjs r1 ca3df48588648d3dd686bddded772f08bfa84393eb0b5254b47142f2b2090ca0
node target/phase-09/check-boundaries.mjs
node target/phase-09/audit-resources.mjs
node target/phase-09/collect-final.mjs
node target/phase-09/verify-final.mjs
git -c safe.directory=D:/WorkSpace/rust/saas/rust-saas diff --check
```

fast owner为`ci-1792073719565-70579731`。浏览器各4CPU、无宿主挂载和公开端口、drop ALL及no-new-privileges，实际隔离/UA/能力/无剩余owner资源记录完整。最终六项只读查询覆盖saas.ci.owner与saas.phase09.owner两标签乘容器/网络/卷，所有WSL lease正常退出；只清理本阶段各owner的临时工作缓存，无全局prune。历史Phase7策略拒绝删除的本地scratch未重试/绕过，与本阶段运行资源结论分列。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 同一app/epoch共享菜单查询；A→B快选后A晚返回不能导航回A | PASS | 单一options/key/signal；真实请求dedup/lazy；共享代次覆盖rail、工作台、A→B→A、其它导航与布局卸载，单一父层推断不覆盖新选择 |
| 入口展示与直达守卫一致；无关应用菜单失败不阻断明确授权页面，未知授权拒绝 | PASS | Sidebar/Palette/工作台/最近访问/TopBar和guard共用策略；已知成功证据优先，无权限/MENU/平台根/未知/错误缓存均不放行，个人页面显式允许 |
| 只有详情读权限、无创建MENU时拒绝创建路径；跨组织迟到结果不回显 | PASS | ticket-create与tickets/:id不重叠，旧tickets/new无创建行为；manifest/校验/授权计划同步；真实忽略abort的旧组织200回包未回写菜单或触发旧应用entry/context |

## Risks / Notes

菜单展示/导航授权不替代后台逐请求权限和独立data grant。工作台是登录用户个人页，监控数据仍由服务端授权；未知路径和错误证据拒绝。已有cancel/clear与API epoch校验继续保留，未把旧key差异包装为已证实的跨租户漏洞。

已有read/create BUTTON形状须人工审阅采用新MENU类型、路径和打开方式，保留UUID/既有授权受众；工具遇冲突在写入前停止。新菜单必须配合采用ticket-create的新前端版本，旧tickets/new前端不是直接兼容回滚对象；既有不可变产物保留，本阶段未对生产执行变更、授权SQL、激活或回滚。

六份部署工具是现有文件首次纳入公开源范围；五份前态文本是明确标记的HEAD规范化重建，不能声称是原478项字节的直接捕获。各历史JSON保留原执行路径语义，归档源/日志在其r1/r2保存位置验证；不把旧摘要用于当前源码。

真实浏览器加载当前宿主与微应用字节，HTTP/Socket.IO使用受控fixture；不宣称真实业务数据库、实际成员删除、OCI镜像构建发行、生产HTTPS或生产部署已验收。Phase10跨页会话、后续Rust服务/新工单ViteUI迁移均未执行。当前大小写/编码变体可能被微应用容器后续更严格拒绝，标准manifest使用规范小写路径，不能由显示层normalize推断更多资源执行能力。

正式lint为0 errors / 61 warnings，构建及Rust检查已通过；既有Vite大chunk及Rust dead_code警告保留。原工程基线的Rust fmt/Clippy等债务未扩展修复，未宣称全仓所有可选门禁均通过。可信执行截止与恢复仍是调度预算，无法强杀主线程同步阻塞或追回已发token/后台已接受写入。

## Suggested Task Ledger Update

### Phase Status Update

Phase 9: COMPLETED。已按台账自身要求同步摘要、详情三项验收、证据、决策/变更记录及下一指针。

### Output Summary

- 统一auth query与类型化route policy、带共享代次的导航、单一父层应用推断；非重叠工单创建MENU/前端/工具校验闭环。
- 专项22/24/70/16、Windows183、Linux fast23/契约311、三产品各183通过；493项freeze4/39路径（含6既有工具范围扩展），原始失败/拒绝和清理证据齐全。

### Completed Items

- 三项Acceptance Criteria均PASS；现状文档、组件README、必需CI入口与可复核证据已同步。

### Remaining Items

- Phase9无未完成必需验收。旧部署菜单采用、既有Rust债务和历史scratch限制明确列出。
- Phase10及后续实施、外部发行和生产部署未执行。

### New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 同一epoch查询工厂，共用显式路由策略与个人白名单 | 缓存/取消/展示/直达原有多套口径，正向授权不应被无关错误阻断 | global cancel/clear保留、未知拒绝、data grant独立；不声称旧key已证明串租户 |
| 共享选择代次，AppLayout为epoch/path/owner唯一推断者 | 晚响应、卸载与旧微应用route effect可覆盖新选择；同epoch授权也需收敛 | 删除子层重复写入，保留执行/授权/故障域及selected-ID门禁 |
| 创建固定ticket-create，保留7key/UUID并转换read/create为明确MENU | tickets/new会匹配详情读参数；单补菜单不解决重叠 | 同步前端/manifest/工具，旧形状冲突人工采用，无自动授权/迁移 |
| 六份既有工具精确加入公开源，最终freeze4完整门禁 | 干净快照原本不含这些实际验证依赖；不同快照/部分/skip不能冒充最终PASS | 493项、39变化路径与各历史来源分列，完整183/311及清理核验，不跨阶段 |

### New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 无新的Phase9阻断问题 | 既有真实发行/生产环境与未来阶段问题继续保留在台账第7节 | 后续阶段负责人 |

### Next Recommended Phase

Phase 10: 协调多标签页的会话轮换与上下文，NOT_STARTED。依赖4/9已完成，须新的明确执行请求，不自动推进。
