# Phase Execution Result — Phase 21

## Executed Phase

Phase21：交付 Vue + Vite 工单前端与未知结果恢复，执行结论：**COMPLETED**。前置Phase6／8／9／10／15／17／18／19／20均已完成。三条验收由最终同源Linux fast、三个真实浏览器产品、独立源码构建、fresh边界与managed实际终态支持。

本次只执行Phase21。Phase22–25已有COMPLETED状态保持，last_completed_phase记录本次21；下一未完成主线为Phase26“建立简单门店 Rust API 与独立业务库”／NOT_STARTED，不实施Phase26。

## Execution Summary

前端独立锁定Vue3.5.43与Vite8.3.4，标准index.html→薄src/main.mjs入口同步登记MicroIframe.connect监听，在iframe加载移交唯一MessagePort前准备接收；lifecycle shell再异步加载真实app chunk，调用导出的createLifecycle(bridge)工厂。Vue模块替代window.Vue，真实split point保留。shell沿用SDK已有await mount／update及dispose abort语义，加载后复验fresh signal，防加载取消后的迟到mount；首个上下文在import期间取消时，后续fresh update仍执行第一次实际Vue mount，unmount保持幂等。Vite base为相对 `./`，入口、动态chunk、CSS、公开config和classic bridge位于同一不可变版本目录。不读宿主node_modules、不取远程CDN，登记SDK1.0.0的classic bridge按原字节输出，micro-sdk通过模块消费，无SDK/vendor生产字节或协议变更。

构建wrapper保留公开version／allowed-host-origin／api-base／out-dir参数，独立npm ci后通过标准Vite生成资源。asset-manifest记录实际entry／imports／CSS，build-info绑定应用／SDK／契约／Vue／Vite／Node版本、独立lock和逐文件source摘要。配置只公开版本、精确宿主origin与API地址，不自动注入VITE_*、secret、DB URL或平台session。生产仅静态Nginx，模块／动态chunk／classic／CSS需正确MIME及匿名CORS，缺失资源真实404；dev／preview只用于本地工具。

客户端继续用SDK1.0.0的requestWithContext，以当前应用Bearer、UUID requestId、credentials:omit和Origin:null消费Rust。创建显式Idempotency-Key header；提醒JSON仅idempotencyKey。预期HTTP状态与业务DTO必须共同验证：创建201／OPEN／version1、更新200／请求id／version+1、列表与目录分页及逐条记录、详情id、删除200／ok:true、提醒200五字段回执。畸形2xx状态或JSON结构归PROTOCOL，写入按对应unknown保留事件，不能清intent或提示成功；额外成功字段投影丢弃。503只读取有界8KiB的固定业务code白名单，远端正文不进入UI提示。

首次创建固定不可变键和完整DTO，发送及unknown时锁定字段，用户显式恢复同事件，不自动换键或重发。提醒按工单保持独立事件，区分PREPARATION_UNKNOWN的本次零平台POST与RESULT_UNKNOWN的可能已发布；409先刷新确认当前目标，再由用户明确创建新事件。PATCH／状态／DELETE未知先读回核对，不自动mutation；编辑409保留草稿、读最新并明确采纳version。删除未确认状态按工单保留，切到另一条记录或核对另一条不会解除原记录的未确认状态。

前端草稿／intent只在同一仍存活iframe模块内存中按appCode／tenantId／organizationId／userId分区，不保存token／完整props、不使用opaque localStorage或新SDK持久化能力。跨route、同业务身份授权续期及切组织返回可恢复相应域内的合法草稿和事件，刷新／关闭／iframe销毁／换发布实例不保证保留，UI提示保持页面直至结果核对完成。它与后端0003持久意图和创建receipt是不同保证，不能宣称浏览器持久恢复。

纯主题不多取列表或取消写入，路由离开取消相应读取并拒绝迟到回写；真实授权替换、到期、切组织及401立即取消旧请求并隐藏服务器内容。read403／404隐藏对应记录，写403暂停相应操作；已unknown事件仍留原key／body，必须当前有效授权才能恢复。创建 `/micro/work-orders/ticket-create` 与详情 `/micro/work-orders/tickets/:id` 继续分别要求精确MENU，功能权限与独立grant不附送，旧/tickets/new只作详情标识。CSV仍经宿主downloadCsv broker，不增加same-origin或下载sandbox权限。

## Deliverables

- 前端标准index、vite.config、同步登记bridge的src/main.mjs、惰性src/lifecycle.mjs、导出生命周期工厂的Vue app、client、内存intents、独立package／lock与兼容参数build wrapper。
- 构建asset-manifest／build-info及相对版本资源，保留登记SDK／classic bridge字节、独立依赖、secret哨兵与输出目录保护。
- 前端VM／客户端／构建HTTP回归、`tests/phase21-work-orders-browser.mjs` 与 `tests/helpers/phase21-work-orders-services.mjs` 的真实平台／Rust／双PG／Nginx浏览器验证，原Phase8轻量竞态保持独立。
- 8份必要文档：根README、architecture、engineering-baseline、Rust baseline、work-orders README、backend README两处窄同步、frontend README和deploy README；仅描述当前实现和验证边界，不改台账或旧证据。
- 本轮正式inputs／boundaries／runtime／报告／ledger-update由统一发布流程产生；target保留所有采用和独立开发观察的原始来源、命令、日志、driver与清理结果。

- [采用候选](../../target/phase-21/inputs-candidate-r3.json)、[最终边界](../../target/phase-21/boundaries-final-r3.json)、[managed汇总](../../target/phase-21/managed-audit-r3.json)、[独立构建](../../target/phase-21/independent-candidate-r3/report.json)。
- [fast child完整summary](../../target/phase-21/linux-fast-r3/reports/fast/summary.json)和[未包装fast gate summary](../../target/phase-21/linux-fast-r3/reports/fast-gate/summary.json)：保留每个实际argv、exit／timeout、counts与日志。
- [chromium真实Rust／Nginx报告](../../target/phase-21/linux-chromium-r3/reports/browser/work-orders/result.json)、[chrome真实Rust／Nginx报告](../../target/phase-21/linux-chrome-r3/reports/browser/work-orders/result.json)、[msedge真实Rust／Nginx报告](../../target/phase-21/linux-msedge-r3/reports/browser/work-orders/result.json)：包含工具／TLS公指纹、两份实际ELF、102项宿主静态文件与9项工单Vite资源、release manifest及逐项业务／transport／清理。
- [Chromium实际受测UI截图](../../target/phase-21/linux-chromium-r3/reports/browser/work-orders/screenshots/phase21-real-ticket.png)。
- 正式发布路径为docs/validation/phase-21-inputs.json、phase-21-boundaries.json、phase-21-runtime.json、phase-21.md、phase-21-ledger-update.md，由root统一写入并核对摘要；原始target证据保留。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 真实opaque iframe可加载module/动态chunk，Origin:null请求Rust API且无Cookie凭据。 | PASS | 三产品各自从同候选fresh构建宿主和两份Rust ELF，实际opaque sandbox为allow-scripts allow-forms，Vue已挂载；module与真实动态chunk为200／JavaScript MIME／匿名CORS，Origin字符串null。所有native attempt均credentials:omit，有应用Bearer及UUID requestId、无Cookie；逐请求typed证明和静态404回归通过。 |
| 主题无多余列表请求、失权隐藏服务器内容、409保留草稿；commit后丢响应不重复创建。 | PASS | 三产品主题列表计数均1→1，真实撤权403后旧行不可见；409保留草稿，显式采用version3后成功。创建真实201提交后丢响应，同key／完整body显式恢复，自动重试0，SQL各仅1条ticket与1条receipt；提醒同样真实200后恢复，仅1次publication／delivery。 |
| 创建与详情MENU互不扩大授权；普通用户可完成真CRUD/导出/提醒。 | PASS | 三产品普通用户均非superAdmin且无管理权限、具直接membership；创建／详情精确MENU在context及Rust实际403下互不授权。真实Rust创建／成员目录／分配／修改／状态筛选／软删、宿主CSV broker下载、持久提醒及收件箱核对全部通过，sandbox权限保持。 |

## Verification Layers and Source Provenance

sourceRevision：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`；inputsSha256：`675b639c3dd5b9e547b21c3b684167b6524855f192f252043d34cca8e8f992b0`；输入726项／闭包316条。最终采用`target/phase-21/inputs-candidate-r3.json`，evidenceClass为ADOPTABLE_CANDIDATE；四个requiredRuns均为同源ADOPTABLE_CANDIDATE_GATE，开发capture与旧候选保持原类别。

| Adopted run | Actual steps | Actual tests or groups | Scope | Evidence |
| --- | --- | --- | --- | --- |
| 完整Linux fast | 55 | 832 PASS／0 FAIL／0 SKIP | 1921 filtered；必需失败／跳过0 | [outer原始终态](../../target/phase-21/linux-fast-r3/linux-runner-result.json) |
| chromium | 14 | 36 PASS／0 FAIL／0 SKIP | Phase8 4＋Phase10 17＋真实业务 15 | [outer原始终态](../../target/phase-21/linux-chromium-r3/linux-runner-result.json) |
| chrome | 14 | 36 PASS／0 FAIL／0 SKIP | Phase8 4＋Phase10 17＋真实业务 15 | [outer原始终态](../../target/phase-21/linux-chrome-r3/linux-runner-result.json) |
| msedge | 14 | 36 PASS／0 FAIL／0 SKIP | Phase8 4＋Phase10 17＋真实业务 15 | [outer原始终态](../../target/phase-21/linux-msedge-r3/linux-runner-result.json) |

fast的832项是本轮完整当前manifest实际测试，包含契约378、工单Vite／静态／生命周期11、CI策略120、开发／Vite20、工单Rust library56等；构建全前端、lint／typecheck、Rust check和既有注册测试也均实际通过。1921 filtered是各选定Rust过滤命令的排除数，未混作执行PASS或SKIP。完整55步及每个原始命令见fast summary，不继承Phase20计数。

三个浏览器各14步均含独立fresh安装／宿主build／两个cargo build、Phase8四组、Phase10十七组及真实业务十五项；每产品36是不同层级的验收组数，未与832个fast测试相加冒充同一种测试。真实launcher本身各15个工具／TLS／browser命令均exit0。轻量HTTP fixture只证明受影响的上下文／生命周期行为，真实业务层另用生产宿主、opaque Vue／Vite、两个Rust服务、两座owned库及Nginx／TLS运行。

Node24.18.0／npm11.16.0／Rust1.97.0／Playwright1.64.1，Nginx实际执行二进制1.28.0，运行OpenSSL3.0.22；系统包查询中的nginx1.22.1是bookworm基础包元数据，实际二进制为官方固定digest stage的1.28.0，nginx -V及ldd逐轮留档。浏览器分别为Chromium156.0.8078.4、Google Chrome155.0.8059.84、Microsoft Edge155.0.4283.62。PostgreSQL17.6运行于owned tmpfs容器，平台库与业务库独立。

同candidate r3的独立构建实际3步PASS，15项frontend源码从冻结目录复制至全新临时目录，无root依赖回退；固定Node／npm执行npm ci --ignore-scripts --no-audit --no-fund及build。9个静态文件均不包含四个secret／DB哨兵，公开config恰allowedHostOrigins／apiBase／version三个字段，公开证据17项；sourceStable与temporaryRemoved均true。

最终fresh边界8项PASS、88项owned输入、hostDrift为空；平台／工单Rust生产代码、SQL迁移及Cargo文件与经start摘要保护的Phase20输入完全相同。另逐条核对SDK／vendor48项path／bytes／SHA相同；这是字节不变性核对，不增加测试数。

managed实际23项检查全部PASS，公开产物1467项、合计2816013492 bytes，artifactsSha256为`5f960d2f0bb6ccc0cad5ae8e6543a4115b438e06e808e2c3e4786c1891fd4d4b`。development记录45条，其中12条JSON status为FAIL；native原始日志按RAW_LOG_RETAINED保存其实际内容，未将失败重标。正式runtime索引单独列其公共归档清单；这里的1467只指managed artifacts，不预算runtime的artifacts.length。

## Actual Browser Transport and Business Evidence

所有请求按UUID requestId逐条绑定native fetch选项／取消／settlement、page请求method／path／frame／安全头布尔观察与终态。已确认HTTP响应、SDK对非OK错误正文主动取消、实际GET signal＋native AbortError取消、两次已提交后故意丢响应分别分类；缺wire确认的取消不计真实HTTP响应，缺失Origin不伪填字符串null，未知或未闭合事件拒绝通过。丢响应forwarder使用独立空Cookie jar、正常TLS，保留原header／body／key和requestId，真实201／200响应及SQL提交后才abort。

| Product | Actual browser version | Native attempts | Page events | Confirmed HTTP | Proved GET cancellation without wire confirmation | Committed-response faults | Actual HTTP count | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| chromium | 156.0.8078.4 | 31 | 31 | 25（含错误正文取消2） | 4 | 2 | 27 | [完整producer报告](../../target/phase-21/linux-chromium-r3/reports/browser/work-orders/result.json) |
| chrome | 155.0.8059.84 | 31 | 31 | 26（含错误正文取消2） | 3 | 2 | 28 | [完整producer报告](../../target/phase-21/linux-chrome-r3/reports/browser/work-orders/result.json) |
| msedge | 155.0.4283.62 | 31 | 31 | 26（含错误正文取消2） | 3 | 2 | 28 | [完整producer报告](../../target/phase-21/linux-msedge-r3/reports/browser/work-orders/result.json) |

Chromium的四条取消、Chrome／Edge的三条取消是各自实测时序结果；三产品native31条均显式omit，未继承诊断r6的3取消数。三个产品各自主题列表1→1、冲突409保draft并显式采version3、read撤权403隐藏旧内容；创建与提醒恢复均原key／body、零自动重试，创建SQL仅1条receipt／ticket，提醒SQL仅1次publication／delivery并在接收者持久inbox可见。实时通知服务本轮不启动，持久提醒成功以数据库和inbox为准。普通用户、MENU互斥、成员目录／分配／CRUD／筛选／CSV broker／软删均在各自真实十五项中完成。

## Cleanup and Publication Boundary

四轮每轮都实际执行两个owner标签×containers／networks的四个精确空查询，共16次exit0且空输出；四个WSL lease均ready／stopped／exit0，无unexpectedLeaseExit，cleanupErrors为空。每轮执行source前后摘要一致，executionSourceUnchanged=true、hostDrift=[]。三产品各自十项inner清理均true，平台／业务进程、三个Nginx master和按PID／startTime登记的worker、双业务库、浏览器及临时目录全部退出／删除；launcher privateWorkRemoved与sourceUnchanged均true，cacheCleanup PASS。保留本地工具镜像缓存不等于遗留运行容器、网络或生产部署。

所有最终driver、原始summary／logs、截图、静态清单、TLS公证书指纹与实际原ELF均进入managed公开清单，私钥／NSS profile／数据库状态、node_modules／构建缓存和运行private work不进入公开归档。最终fresh边界8项PASS、88项owned输入、hostDrift为空；平台／工单Rust生产代码、SQL迁移及Cargo文件与经start摘要保护的Phase20输入完全相同。另逐条核对SDK／vendor48项path／bytes／SHA相同；这是字节不变性核对，不增加测试数。

## Development Findings and Original Failures

本机真实浏览器r1／r2均保留原FAIL日志：[native-browser-compat-r1.log](../../target/phase-21/native-browser-compat-r1.log)、[native-browser-compat-r2.log](../../target/phase-21/native-browser-compat-r2.log)。原日志记录iframe application handshake timed out及等待UI超时；同时出现的未启动通知服务连接拒绝／WebSocket错误保持原样，不据它们将持久提醒或握手问题混为同一原因。

已定位的入口问题是顶层等待动态app导入后才connect，可能错过iframe load时唯一MessagePort移交。修复改成薄main同步connect、shell惰性加载真实业务chunk与工厂，补fresh signal、首次实际mounted和幂等unmount边界；没有修改SDK/vendor。开发前端完整测试及后续浏览器尝试保持各自来源，candidate r2的两轮outer FAIL与最终candidate r3完整矩阵／managed分别保留；不以修复存在、开发PASS或后续通过改写r1／r2原失败，也不把开发计数加入最终门禁。

工具镜像r1原FAIL为legacy Docker builder拒绝`--progress`参数、build exit125，尚未执行镜像层；r2原FAIL为Nginx1.28.0的最小配置检查缺少`/var/cache/nginx/client_temp`目录、build exit1。两轮各自资源空查与WSL lease终态保留在原result，不将工具失败写成应用浏览器失败或取消。r3镜像构建与工具探针PASS只证明本地Linux测试工具可用；保留为tool cache，不代表生产镜像发行、真实业务或最终门禁通过。各轮源码输入和driver按自身报告绑定。

独立开发记录还包括前端完整测试72 PASS／0 FAIL／0 SKIP、源码临时复制后独立安装与build r1／r2 PASS、native r3 Phase8四组及native session r1十七组。它们与最终候选分列；shared CI局部测试每轮实际数量和source取各自report，不拼成同一候选。仅capture而没有执行终态的开发快照按`NO_EXECUTION_REPORT_FOUND`保留，不能伪作PASS、FAIL或取消。原native inspection临时目录经九个明确文件逐项删除及两个空目录删除后已不存在，`native-inspect-cleanup.json`记录先前递归删除被策略拒绝及最终非递归清理成功；没有遗留该目录。

Linux开发readiness按自己的冻结输入和原始终态保留：

| Attempt | Actual outcome | Root cause and cleanup |
| --- | --- | --- |
| readiness r1／development r4，source `92f8bc4e…7856` | outer／child FAIL，9步中8 PASS／1 FAIL，未到真实业务浏览器 | 宿主Vite构建copyfile logo.svg报EPERM。host-copy diagnostic在同cap-drop ALL约束下复现uid0执行、uid1000源文件copyFile失败，字节读写及同执行uid源文件copy成功。只规范owned容器精确manifest公共文件／目录到实际uid／gid，保留mode／bytes／SHA且拒绝链接；不增加capability。该修复shared CI r6实际118 PASS／0 FAIL／0 SKIP，source前后`eca62999…adff`不变。readiness r1的四项资源空查、lease退出0及source稳定原结果均保留。 |
| readiness r2／development r5，source `eca62999…adff` | outer／child FAIL，12步中11 PASS／1 FAIL，真实浏览器0项通过 | 源码ownership与宿主构建已通过，两份Rust ELF及TLS／NSS十四项准备命令通过；Nginx编译默认日志路径EACCES、生产`/dev/stderr`重开Node管道socket触发ENXIO，服务启动即失败。inner temporaryDirectoryRemoved原为false，其余九项清理为true；launcher清除private work，outer销毁容器后四项空查／lease0为PASS。保留内外层区别，不把外层清理改写成inner全true。 |
| readiness r3／development r6，source `b4bed9c5…ddb2` | outer／child FAIL，12步中11 PASS／1 FAIL，真实浏览器0项通过 | Nginx edge的`/`readiness请求未声明Accept:text/html，命中生产静态真实404并等待超时。原inner十项清理均true，外层四空查／lease0为PASS。hostDrift原记录两个后续修正文件，executionSourceUnchanged仍为true，不把它合并为候选r2通过。 |
| nginx-accept diagnostic r1 | 原FAIL，诊断服务未进入请求观察 | 诊断脚本重复添加daemon指令导致edge自身启动失败；该脚本缺陷单独记录，两个master／登记worker及临时目录清理完成，外层四空查／lease0为PASS。 |
| nginx-accept diagnostic r2 | OBSERVATION_RECORDED，四个真实HTTP观察 | host／edge各自`/`无Accept得404，明确Accept:text/html得200；Node默认TLS验证使用owned CA，无忽略TLS。按PID／startTime登记的worker均已消失，两个master退出0、无强杀、临时目录删除，外层四空查／lease0为PASS。该最小诊断不计入真实业务15项。 |
| candidate r1完整fast／linux-fast-r1 | 原FAIL，55步中54 PASS／1 FAIL，必需skip0 | dev-and-vite的旧vite-example-delivery调用build:all缺少工单现已必需的精确allowed-host-origin，子组20项中19 PASS／1 FAIL。outer另保留冻结后owned helper变化的driverError，不掩盖为单纯测试失败；四项资源空查／lease0仍PASS。原candidate r1和其独立build均不采用到candidate r2。 |
| candidate r2 Chromium／linux-chromium-r2 | outer／child FAIL；Phase8四组、Phase10十七组各PASS；真实producer为15 PASS／1 FAIL／0 SKIP | 十五个具名业务场景均已记录PASS，结尾的全请求transport聚合断言另FAIL，不能把15 PASS改成整轮成功。child14步中13 PASS／1 FAIL、必需skip0，失败step按协议拒绝导入producer部分计数。inner十项cleanup均true、三个Nginx master／登记worker确认退出、outer四空查／lease0、执行source前后稳定；全保留原始FAIL。 |
| candidate r2 fast／linux-fast-r2 | child55／55步PASS，830 PASS／0 FAIL／0 SKIP；outer FAIL | child gateExitCode0、source前后6f89ad4a…cc54不变、cacheCleanup PASS；filtered1921与已执行830项分列。outer在终态发现`tests/phase21-work-orders-browser.mjs`已偏离冻结字节，以driverError拒绝采用；四项资源空查／lease0为PASS，不将child成功覆盖outer失败。 |
| readiness r4／development r7，source `635e6d8a…0d7b7` | outer／child FAIL；真实producer仍15 PASS／1 FAIL／0 SKIP，十项inner cleanup均true | 复用candidate r2原ELF的诊断保留30条浏览器请求事件与31条fetch观察。精确三条GET（tickets两条、assignees一条）为net::ERR_ABORTED、无HTTP response，未观察到Origin头；其余浏览器事件Origin均为字符串`null`，30条均无Cookie、有Bearer和requestId。未获得HTTP响应的事件不能充作已确认wire请求，也不能直接断言没有发出请求；原聚合失败保持。outer四空查／lease0、source稳定均确认，复用ELF仅是诊断，不是新候选重建证据。 |
| readiness r5／development r8，source `24ca882c…b35b3` | outer／child FAIL；真实producer15具名PASS／1聚合FAIL／0 SKIP | typed诊断已排空，31条native与31条page事件配对；仍失败于有真实409／403响应但page终态ERR_ABORTED的情况。登记SDK原逻辑对非OK响应主动body.cancel，native fetch已fulfilled而错误正文取消，旧parser误要求所有有response事件finished。inner十项cleanup／privateWorkRemoved、outer四空查／lease0／source稳定均确认；保留原FAIL，不改SDK来迎合测试。 |
| readiness r6／development r9，source `675b639c…992b0` | outer／child实际PASS，真实producer15 PASS／0 FAIL／0 SKIP | 复用原ELF的诊断闭合31条native＝31条page：26个已确认HTTP响应（含409／403两条SDK错误正文取消）、3条有signal＋native AbortError配对的GET取消、2条独立空Cookie jar／正常TLS转发后丢响应故障。native31条均显式omit；actualRequestCount28不含三条无wire确认的取消。inner十项cleanup、privateWorkRemoved／sourceUnchanged、outer四项空查／lease0／hostDrift0均确认。该轮明确DEVELOPMENT_READINESS，复用ELF，不计candidate r3 fresh正式矩阵。 |

readiness修正只针对测试服务启动和记录边界：Nginx日志采用owned常规文件描述符及`-e`，不改生产renderer配置；readiness明确Accept:text/html，保持真正缺失资源404；按`/proc`中的PID和startTime证明master与登记worker退出，避免缺KILL权限时误认存活状态。helper `74b6819094b58073b951feeca94cbd489c49c06c8d8ed3eb888fefd4b06e7b85`先对完整输出脱敏再截断，避免截断切开秘密值。TLS验证、现有cap边界和业务／SDK生产语义不放宽。

旧Vite交付fixture补入构建时的精确allowed-host-origin后，交付与宿主静态fixture两次开发命令均取得exit0的shell终态，但当时未持久保存日志／report；不为它们补造原日志，也不冒充shared CI r6涵盖它们。candidate r2完整fast已持久保存对应dev-and-vite子步PASS原始日志，但该轮outer仍因host producer漂移FAIL，不能采用为最终同源矩阵。

针对transport聚合缺口，后续producer／parser改成按UUID requestId完整配对native fetch开始／signal取消／settlement和浏览器请求事件终态，逐条保留method／path／opaque frame／头部布尔观察；所有native attempt必须显式credentials:omit。分类明确区分有HTTP响应、发出浏览器事件前取消、已有临时事件但缺wire确认的取消，以及创建／提醒两次已提交后主动丢响应。只有GET、实际signal已取消且native AbortError才能进入取消分类；有事件的取消另要求ERR_ABORTED／无response，不能丢弃未分类或未知失败，也不能将缺失Origin填成`null`。两次丢响应通过独立空Cookie jar、正常TLS的API context转发原header／body，记录真实201／200提交及对应requestId后abort；不借浏览器Cookie jar、不改业务请求参数或SDK。

`shared-ci-transport-local-r1/report.json`与`tests.tap`实际7 PASS／0 FAIL／0 SKIP，exit0、source前后`24ca882c57e9e813c92dab065d1e76be346c38103267480838863393735b35b3`不变，覆盖typed分类、计数闭合及缺字段／错配／凭据／错误分类／故障转发负例。这是局部开发验证；其后的同源readiness r5最终15具名PASS／1聚合FAIL已原样保留，局部通过不能替代真实诊断结果。

对SDK原非OK正文取消行为新增独立`browser-error-body-cancelled`分类，仍要求真实400–599响应、Origin字符串`null`、native fulfilled／无native错误、同UUID／method／path，以及page ERR_ABORTED，计入confirmedWire而不当成缺wire的GET取消；其余未知失败继续拒绝。`shared-ci-transport-local-r2/before-fix.json`及before-fix.tap先实际复现8项中7 PASS／1 FAIL，after-fix.tap和report.json再取得8 PASS／0 FAIL／0 SKIP，exit0且source前后`675b639c3dd5b9e547b21c3b684167b6524855f192f252043d34cca8e8f992b0`不变。原RED与后续GREEN分列，SDK／vendor与生产行为未为测试改变。

development r1／r2／r3只有724项输入／311条边的capture，尚未找到对应执行报告；development r4对应readiness r1，r5对应readiness r2，r6对应readiness r3，r7对应transport readiness r4，r8对应失败readiness r5，r9对应成功readiness r6。九份capture都保留原evidenceClass和source摘要，r9与candidate r3的inputs SHA相同也不改变其诊断类别，不重标最终候选或为未运行快照制造取消结果。

## Recovery and Compatibility Boundaries

| Boundary | Current implementation |
| --- | --- |
| 内存域 | app／tenant／organization／user四元隔离；当前iframe内跨route／续期／切组织返回恢复，刷新／销毁不保证保存。 |
| 不确定创建／提醒 | 不变键／完整正文；合法成功status＋DTO才settle，畸形2xx／network／protocol／5xx留unknown。当前授权不足不能让历史事件扩权。 |
| 版本写入 | 409保留draft，读最新并显式采version；未知先读回、不自动mutation。删除unknown按工单保留。 |
| 服务器内容 | 真实授权变化／切组织／401清可见快照；迟到结果不得回写新route或身份，纯主题不触发额外列表。 |
| 持久后台语义 | Phase18 receipt与Phase20 0003仍由Rust负责，本阶段不改schema、receipt／intent或通知平台语义。 |
| SDK／iframe | 登记1.0.0、context1.0／MicroProps2.0保持，无新localStorage／SDK协议、same-origin或下载权限；当前应用Bearer／omit Cookie。 |
| 同步握手／惰性UI | main先注册SDK监听，shell再加载真实chunk；fresh signal复验、第一次实际mount与幂等unmount防迟到挂载，SDK已有异步生命周期协议不变。 |
| 发布／回退 | 新不可变版本，保留旧chunk；回退前核对在途事件，旧无创建header前端不是等价Rust消费者。换实例不会恢复已销毁iframe内存，不删receipt／intent或改checksum。 |
| 阶段边界 | 不做视觉重设计、不改后台、不执行生产部署／镜像发行／流量切换；既有22–25保持，下一仅建议26。 |

## Risks / Notes

本阶段范围内验收已全部PASS。草稿／intent只保证仍存活iframe的四元内存恢复，刷新／关闭／销毁／换发布实例不保证保存；UI要求unknown核对期间保持页面。后端Phase18 receipt与Phase20持久intent及跨库窗口保持既有语义，不将前端显式恢复扩大为跨刷新持久化。

本轮未重跑完整历史PostgreSQL、通知、交付或无关browser gate，不把旧结果计为本次PASS；实际PG业务验证为三个产品真实双库链与明确SQL断言。未宣称最近两个稳定浏览器版本、Firefox／Safari、新SDK、生产镜像发行、外部registry发布、生产部署或切流完成。正式工具镜像、ELF和静态产物为本地验证输入／归档。

## Suggested Task Ledger Update

Phase21 COMPLETED，三条原文验收均PASS；同步摘要表、Phase21详情／Expected Outputs／Notes、Change Log和指针。证据采用candidate r3与上述四requiredRuns、独立3、fresh boundary和managed，保留所有原始失败／诊断分类。Phase22–25及其他36个阶段记录保持；下一Phase26 NOT_STARTED，新的执行请求前不实施。
