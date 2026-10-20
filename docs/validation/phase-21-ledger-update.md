# Suggested Task Ledger Update — Phase21

## Phase Status Update

Phase21“交付 Vue + Vite 工单前端与未知结果恢复”：**COMPLETED**。本轮全部必需验收已取得同一采用候选的实际PASS。

Phase22–25既有COMPLETED状态保持；本次last_completed_phase记21。下一未完成主线为Phase26“建立简单门店 Rust API 与独立业务库”／NOT_STARTED，不重置22–25、不自动实施26。其他36个阶段与全局缓存原记录保持。

## Output Summary

- 独立Vue3.5.43／Vite8.3.4模块工程、自己的npm lock；薄main同步登记MicroIframe.connect后由lifecycle shell惰性加载真实app chunk／createLifecycle(bridge)工厂，保留相对ESM／动态chunk／CSS。shell复验fresh signal、支持取消后新上下文首次实际mount及幂等unmount，不改SDK/vendor协议；无全局Vue、root node_modules或远程CDN依赖。
- 登记SDK1.0.0和classic bridge保持生产字节；公开config、asset-manifest与build-info绑定实际版本／资源／source／lock，不注入VITE_*、secret、DB URL或平台session。生产仅静态Nginx，正确MIME／匿名CORS／真实404，dev／preview限本地工具。
- WorkOrdersClient用当前应用Bearer、UUID requestId及credentials:omit消费Rust，创建Idempotency-Key header与提醒JSON事件键分别保存；预期status及完整合法DTO才结束原事件，畸形2xx／网络／协议／5xx归对应unknown，错误正文不进入UI。
- 创建原事件键与完整冻结DTO在发送／unknown期间锁定，用户显式恢复，不自动换键；提醒区分preparation零POST与result未知，409先刷新确认目标再显式新事件。版本409保留输入并明确采新version，PATCH／状态／删除未知先读回，删除unknown按工单保留。
- draft／intent只在同一存活iframe内按app／tenant／org／user隔离，跨route／续期／切组织返回恢复相应合法状态；刷新／关闭／销毁／换发布实例不保证保留，未知时保持页面直至核对完成。无opaque localStorage／新SDK能力，不等同于后端0003持久意图。
- 纯主题不多取列表或取消写入，真实失权／到期／切组织取消旧请求并隐藏服务器内容，迟到结果不回写新route或身份。创建与详情精确MENU继续独立，普通用户功能权限／grant和CSV宿主broker不扩权，不增加iframe同源／下载权限。
- 8份文档同步当前行为、独立构建、真实浏览器与资源边界，后台生产语义／schema／SDK不改，无视觉重设计或生产部署切流。

## Completed Items

- sourceRevision：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`；inputsSha256：`675b639c3dd5b9e547b21c3b684167b6524855f192f252043d34cca8e8f992b0`；输入726项／闭包316条。最终采用`target/phase-21/inputs-candidate-r3.json`，evidenceClass为ADOPTABLE_CANDIDATE；四个requiredRuns均为同源ADOPTABLE_CANDIDATE_GATE，开发capture与旧候选保持原类别。
- 完整Linux fast实际55步PASS，832 tests／832 PASS／0 FAIL／0 SKIP，filtered1921单列；build／lint／typecheck／Rust check及当前注册的选定测试均实际执行，未把历史Phase20或开发结果继承为本轮通过。
- chromium：14步PASS，Phase8四组／Phase10十七组／真实业务十五项各零FAIL／SKIP，实际浏览器156.0.8078.4。
- chrome：14步PASS，Phase8四组／Phase10十七组／真实业务十五项各零FAIL／SKIP，实际浏览器155.0.8059.84。
- msedge：14步PASS，Phase8四组／Phase10十七组／真实业务十五项各零FAIL／SKIP，实际浏览器155.0.4283.62。
- 同candidate r3的独立构建实际3步PASS，15项frontend源码从冻结目录复制至全新临时目录，无root依赖回退；固定Node／npm执行npm ci --ignore-scripts --no-audit --no-fund及build。9个静态文件均不包含四个secret／DB哨兵，公开config恰allowedHostOrigins／apiBase／version三个字段，公开证据17项；sourceStable与temporaryRemoved均true。
- 最终fresh边界8项PASS、88项owned输入、hostDrift为空；平台／工单Rust生产代码、SQL迁移及Cargo文件与经start摘要保护的Phase20输入完全相同。另逐条核对SDK／vendor48项path／bytes／SHA相同；这是字节不变性核对，不增加测试数。
- managed实际23项检查全部PASS，公开产物1467项、合计2816013492 bytes，artifactsSha256为`5f960d2f0bb6ccc0cad5ae8e6543a4115b438e06e808e2c3e4786c1891fd4d4b`。development记录45条，其中12条JSON status为FAIL；native原始日志按RAW_LOG_RETAINED保存其实际内容，未将失败重标。正式runtime索引单独列其公共归档清单；这里的1467只指managed artifacts，不预算runtime的artifacts.length。
- 四轮每轮都实际执行两个owner标签×containers／networks的四个精确空查询，共16次exit0且空输出；四个WSL lease均ready／stopped／exit0，无unexpectedLeaseExit，cleanupErrors为空。每轮执行source前后摘要一致，executionSourceUnchanged=true、hostDrift=[]。三产品各自十项inner清理均true，平台／业务进程、三个Nginx master和按PID／startTime登记的worker、双业务库、浏览器及临时目录全部退出／删除；launcher privateWorkRemoved与sourceUnchanged均true，cacheCleanup PASS。保留本地工具镜像缓存不等于遗留运行容器、网络或生产部署。
- 8份必要源码说明已同步当前行为、独立构建和部署／恢复限制。无Rust生产代码／SQL／SDK／sandbox扩权，无视觉重设计、生产发布部署或切流。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 真实opaque iframe可加载module/动态chunk，Origin:null请求Rust API且无Cookie凭据。 | PASS | 三产品各自从同候选fresh构建宿主和两份Rust ELF，实际opaque sandbox为allow-scripts allow-forms，Vue已挂载；module与真实动态chunk为200／JavaScript MIME／匿名CORS，Origin字符串null。所有native attempt均credentials:omit，有应用Bearer及UUID requestId、无Cookie；逐请求typed证明和静态404回归通过。 |
| 主题无多余列表请求、失权隐藏服务器内容、409保留草稿；commit后丢响应不重复创建。 | PASS | 三产品主题列表计数均1→1，真实撤权403后旧行不可见；409保留草稿，显式采用version3后成功。创建真实201提交后丢响应，同key／完整body显式恢复，自动重试0，SQL各仅1条ticket与1条receipt；提醒同样真实200后恢复，仅1次publication／delivery。 |
| 创建与详情MENU互不扩大授权；普通用户可完成真CRUD/导出/提醒。 | PASS | 三产品普通用户均非superAdmin且无管理权限、具直接membership；创建／详情精确MENU在context及Rust实际403下互不授权。真实Rust创建／成员目录／分配／修改／状态筛选／软删、宿主CSV broker下载、持久提醒及收件箱核对全部通过，sandbox权限保持。 |

## Adopted Verification Matrix

| Adopted run | Actual steps | Actual tests or groups | Scope | Evidence |
| --- | --- | --- | --- | --- |
| 完整Linux fast | 55 | 832 PASS／0 FAIL／0 SKIP | 1921 filtered；必需失败／跳过0 | [outer原始终态](../../target/phase-21/linux-fast-r3/linux-runner-result.json) |
| chromium | 14 | 36 PASS／0 FAIL／0 SKIP | Phase8 4＋Phase10 17＋真实业务 15 | [outer原始终态](../../target/phase-21/linux-chromium-r3/linux-runner-result.json) |
| chrome | 14 | 36 PASS／0 FAIL／0 SKIP | Phase8 4＋Phase10 17＋真实业务 15 | [outer原始终态](../../target/phase-21/linux-chrome-r3/linux-runner-result.json) |
| msedge | 14 | 36 PASS／0 FAIL／0 SKIP | Phase8 4＋Phase10 17＋真实业务 15 | [outer原始终态](../../target/phase-21/linux-msedge-r3/linux-runner-result.json) |

[采用候选](../../target/phase-21/inputs-candidate-r3.json)、[最终边界](../../target/phase-21/boundaries-final-r3.json)、[managed汇总](../../target/phase-21/managed-audit-r3.json)、[独立构建](../../target/phase-21/independent-candidate-r3/report.json)。三个browser各自fresh cargo build后使用并归档两份实际ELF，host静态102项／工单Vite9项按摘要留存；各自正常TLS验证和owned CA／NSS只用于本轮私有环境。Node24.18.0／npm11.16.0／Rust1.97.0／Playwright1.64.1，Nginx实际执行二进制1.28.0，运行OpenSSL3.0.22；系统包查询中的nginx1.22.1是bookworm基础包元数据，实际二进制为官方固定digest stage的1.28.0，nginx -V及ldd逐轮留档。浏览器分别为Chromium156.0.8078.4、Google Chrome155.0.8059.84、Microsoft Edge155.0.4283.62。PostgreSQL17.6运行于owned tmpfs容器，平台库与业务库独立。

所有请求按UUID requestId逐条绑定native fetch选项／取消／settlement、page请求method／path／frame／安全头布尔观察与终态。已确认HTTP响应、SDK对非OK错误正文主动取消、实际GET signal＋native AbortError取消、两次已提交后故意丢响应分别分类；缺wire确认的取消不计真实HTTP响应，缺失Origin不伪填字符串null，未知或未闭合事件拒绝通过。丢响应forwarder使用独立空Cookie jar、正常TLS，保留原header／body／key和requestId，真实201／200响应及SQL提交后才abort。

| Product | Actual browser version | Native attempts | Page events | Confirmed HTTP | Proved GET cancellation without wire confirmation | Committed-response faults | Actual HTTP count | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| chromium | 156.0.8078.4 | 31 | 31 | 25（含错误正文取消2） | 4 | 2 | 27 | [完整producer报告](../../target/phase-21/linux-chromium-r3/reports/browser/work-orders/result.json) |
| chrome | 155.0.8059.84 | 31 | 31 | 26（含错误正文取消2） | 3 | 2 | 28 | [完整producer报告](../../target/phase-21/linux-chrome-r3/reports/browser/work-orders/result.json) |
| msedge | 155.0.4283.62 | 31 | 31 | 26（含错误正文取消2） | 3 | 2 | 28 | [完整producer报告](../../target/phase-21/linux-msedge-r3/reports/browser/work-orders/result.json) |

各产品计数均取自己的producer：Chromium4条取消，Chrome／Edge各3条，三者各2条错误正文取消计入confirmed HTTP。不可把诊断r6的31＝26＋3＋2复制为三个正式产品的同一结果。三产品主题1→1、409显式version3、创建／提醒同key／body显式恢复且SQL唯一、普通用户MENU互斥及真实CRUD／CSV／inbox均已验收；阶段内无剩余实现或必需验证项。

## Remaining Items

Phase21范围内无剩余项。内存恢复、后端跨库窗口和回退限制作为已公开兼容边界保留；它们不触发本次扩架构或自动实施下一阶段。root按统一发布与台账终态验证流程采纳这份更新，保留公共证据摘要及其他阶段原字节。

## Original Development Results

采用终态与开发观察分别归档。[managed development原始分类](../../target/phase-21/managed-audit-r3.json)保留45条记录，包括JSON status FAIL的12条及native原始日志；仅capture未执行的r1–3不造取消或PASS。

| Original source or attempt | Preserved result and meaning |
| --- | --- |
| native browser r1／r2 | 握手及UI等待原FAIL日志保留；薄main同步connect＋惰性工厂修复后r3 Phase8四组与session r1十七组另列，不改SDK/vendor。前端完整开发72 PASS及独立build r1／r2另列，native inspection临时目录九文件非递归清理已完成。 |
| toolimage r1／r2／r3 | r1 build125拒绝legacy --progress，r2 build1缺Nginx client_temp目录，原FAIL与各自资源／lease保留；r3工具PASS只作环境就绪和本地缓存。 |
| candidate r1／independent r1 | source eca62999…adff，独立build原PASS但未采用；fast原55步54 PASS／1 FAIL为旧fixture缺精确宿主origin，outer另有owned helper漂移。旧boundary／capture／driver各保留。 |
| candidate r2／independent r2 | source 6f89ad4a…cc54，独立build原PASS但未采用；Chromium十五具名PASS后transport聚合1 FAIL，Phase8四组／Phase10十七组分别PASS。fast child55／55、830 PASS但outer冻结后producer漂移FAIL，不能由child或清理成功覆盖采用失败。 |
| readiness r1–3 | r1 hostcopy EPERM；只规范owned精确manifest公共输入uid／gid、mode／bytes／SHA不变，CI局部118 PASS。r2 Rust／TLS已通过后Nginx默认日志EACCES／Node socket重开ENXIO，inner临时目录清理原false、outer后续成功分列；r3 edge缺Accept导致生产404超时，inner十项清理true。三轮FAIL不改。 |
| hostcopy／nginx diagnostics | hostcopy复现跨uid copyFile问题。Nginx最小r1自身daemon重复原FAIL；r2无Accept404／Accept:text/html200、默认TLS／worker退出／资源清理成功，仅观察记录。helper常规FD／-e、完整脱敏后截断、PID＋startTime证明退出，不放宽生产config／caps／TLS。 |
| readiness r4 | 30 page／31 fetch，三条GET无response／ERR_ABORTED／未观察到Origin（tickets2／assignees1）；其余Origin字符串null、全部无Cookie／有Bearer和requestId。15具名PASS后聚合1 FAIL，取消不冒充确认wire。原ELF复用仅诊断。 |
| typed局部r1及readiness r5 | typed局部7 PASS后，r5实际31 native＝31 page但15具名PASS后聚合1 FAIL。SDK原非OK body.cancel令真实409／403有response且native fulfilled、page ERR_ABORTED，旧parser误要求finished；不修改SDK迎合测试。 |
| typed局部r2 | before-fix原8项7 PASS／1 FAIL，修后8 PASS／0 FAIL／0 SKIP；新增严格browser-error-body-cancelled并计confirmedWire，未知失败继续拒绝，原RED／GREEN日志与source分别保留。 |
| readiness r6／development r9 | source675b639c…992b0，原ELF复用诊断实际15 PASS／0 FAIL／0 SKIP，31 native＝31 page＝26 confirmed（含2 error body cancel）＋3 proven GET cancel＋2 fresh fault；inner十项、privateWorkRemoved／source／outer四空查／lease0均确认。类别仍DEVELOPMENT_READINESS，不纳入最终fresh矩阵。 |

所有失败与诊断的详细命令、来源、清理见各自原始记录及phase-21.md，不跨来源拼接绿色结果。旧Vite交付／宿主静态的两次开发shell exit0当时无持久log，不补造日志或归入CI118；最终fresh fast的dev-and-vite20项实际PASS原日志已独立归档。

## New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| Vue模块／Vite标准入口和独立lock | 仅改script名称无法证明标准模块、动态chunk或独立依赖 | 真实ESM split point、asset-manifest／build-info与生产静态Nginx验证。 |
| bridge监听同步登记，UI工厂惰性加载 | iframe load只移交一次MessagePort，顶层等待业务chunk会产生真实握手竞态 | 保留动态chunk且SDK不变；shell按fresh signal／实际mounted／幂等unmount防取消后迟到挂载，r1／r2原FAIL独立保留。 |
| 内存vault按完整业务四元域隔离 | 令牌续期不是新事件，切组织／身份不能串用草稿／key | 同iframe跨route／续期／切回恢复，刷新或销毁不保证；不引入localStorage或SDK持久化。 |
| 状态与合法成功DTO共同校验后才settle | 2xx错误状态／畸形JSON可能掩盖已提交但未确认结果 | 写入归unknown并保留原事件，不能提示成功或换键；额外字段投影丢弃。 |
| 用户显式恢复创建／提醒，版本写入先读回 | 自动重发或换键会重复业务，旧version不能覆盖并发变更 | 创建冻结原文，提醒409先确认目标，409保draft，删除unknown按工单保留。 |
| 维持已有SDK／sandbox与后台契约 | 前端接入不应扩OAuth／iframe能力或改Phase18–20持久语义 | 1.0.0／Origin:null／omit Cookie／CSV broker保持，后端0003与内存意图分别说明。 |
| 顺序21→26，保留已完成22–25 | 默认顺序选择下一未完成主线，不能撤销其他聊天成果 | last_completed_phase为本次21，仅建议26，不实施下一阶段。 |
| typed transport逐requestId闭合并保留错误正文取消 | page事件可能在native fulfilled后因SDK原body.cancel报告ERR_ABORTED，缺wire的GET取消也不能当HTTP响应 | producer和parser严格分类／计数／负例验证；SDK与请求凭据不改，三产品各用自己的实测计数。 |
| 采用candidate r3四轮fresh结果及独立build | 旧候选outer失败或复用ELF诊断不满足最终同源要求 | fast55／832、三产品各4＋17＋15、独立3／15／9、fresh边界8及managed23分别留档，旧失败不重标。 |

## New Open Questions

无本阶段新增阻塞问题。前端draft／intent只在存活iframe内按app／tenant／org／user隔离，跨route／续期／切组织返回可恢复；刷新／关闭／销毁／换发布实例不保证保留，unknown期间保持页面直至核对。后端receipt／0003持久intent及跨库窗口保持既有边界，不宣称浏览器跨刷新持久化。

本轮不继承或宣称完整历史PG／通知／交付／无关browser、两个稳定浏览器版本或Firefox／Safari验收；没有外部镜像发行、registry发布、生产部署或流量切换。

## Next Recommended Phase

Phase26：建立简单门店 Rust API 与独立业务库，**NOT_STARTED**。跳过已COMPLETED22–25，本次只完成21，保留其他阶段状态；新的执行请求前不实施26。

## Ledger Evidence Note

2026-10-17按明确授权完成Phase21，独立Vue3.5.43／Vite8.3.4与SDK1.0.0相对ESM／动态chunk、真实Rust UI、四元内存草稿／事件和显式unknown恢复。最终采用source675b639c3dd5b9e547b21c3b684167b6524855f192f252043d34cca8e8f992b0（726输入／316边），fresh fast55步832 PASS／0 FAIL／0 SKIP，Chromium／Chrome／Edge各14步、Phase8四组＋Phase10十七组＋真实双Rust／双PG／Nginx十五项零FAIL／SKIP；独立源码15输入／3步／9静态秘密哨兵／17产物PASS。fresh边界8 PASS／88 owned／hostDrift0，managed23 PASS／1467公开产物，四轮每轮四资源空查／lease0、三browser inner十项与private work清理均成功。三条原文验收PASS，保留原握手／工具／readiness／旧候选失败、局部RED／GREEN及复用ELF诊断，不继承历史计数。不改后台生产schema／SDK／sandbox或生产部署，内存不保证跨刷新。只同步21 COMPLETED，last_completed_phase21，22–25保持COMPLETED，下一26 NOT_STARTED。
