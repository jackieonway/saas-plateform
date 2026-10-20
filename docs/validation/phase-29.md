# Phase 29 — 收敛宿主加载、失败恢复和高复杂页面边界

## Executed Phase / Execution Summary

2026-10-19（Asia/Shanghai），本阶段 COMPLETED。依赖 Phase 3、4、7、9 在台账中均为 COMPLETED。只实施 Phase29；没有推进Phase30、修改Rust/API/SQL/SDK契约、远端workflow、真实外部发行或生产部署。

实际工作树基于 Git `32956c4532bbd6eff53584fd0f5e9bb6e188437c`；851个公开输入的SHA-256为 `300df64b7a9cb1fcff4aa3371f8a657e0149cf0ad10cb8c4bbc9905124012456`。HEAD不代表未提交实现；逐文件清单见[inputs](phase-29-inputs.json)。124份既有正式证据逐文件复核未改动。开始时的四个被改源码、完整baseline生产dist和模块图保留在 `target/phase-29/source-before`、`target/phase-29/before`；不把最终工作树摘要冒充baseline源码摘要。

实际工具：Node v24.18.0、npm 11.16.0、TypeScript 5.9.3、Vite 6.4.3、React 19.3.0、React Router 7.18.4、ESLint 10.12.0、Playwright 1.64.1。浏览器为 Chromium 156.0.8078.4、Chrome 155.0.8059.80、Edge 142.0.3595.94，不冒称最新两个稳定major矩阵。

## Deliverables

- App.tsx先用轻量登录守卫处理匿名深链，之后才导入AppLayout。公开页面保留根加载/恢复边界。
- CommandPalette由点击搜索或Ctrl/Meta+K触发导入；快捷键监听移到壳中，关闭/重新打开、菜单授权和搜索行为保留。面板有独立加载/错误边界。
- Outlet局部Suspense/ErrorBoundary保留AppRail、AppSidebar、TopBar；render throw可重试或用实际侧栏导航离开，lazy拒绝显示明确刷新入口。没有自动reload，自动次数为0。普通路由按epoch/path重置；micro路由仍按epoch/appCode保持容器生命周期，子路径变化继续update。
- 选取ApplicationsPage，因为它原本在同一组件中耦合schema、创建/编辑状态、immutable入口字段、mutation结果与列表/删除/发布引导。仅将该用例拆为use-application-editor和ApplicationEditorDialog，列表/删除/发布编排保留。HTML快速创建、编辑已发布入口、缺失ESM模块和创建后发布引导均有真实UI回归；API请求体逐字段一致。
- 生产Nginx缺失assets返回404纯文本/no-store；命名错误位置清空扩展名MIME映射，既不返回SPA，也不伪装成有效脚本。真实只读非root容器验证7个请求，保留baseline旧入口资源且SHA一致，清理成功。

## Measurement / Frozen Budget

先构建并测baseline，再修改实现。同口径为5个全新浏览器context、1365×900、reduced motion、4倍CPU限速、40ms网络延迟、1.6Mbps下载、gzip HTTP和固定公开branding fixture。Resource Timing transferSize含浏览器计入的头部开销；encodedBodySize是压缩body，不是抓包的精确链路字节。等待网络/字体就绪后输入固定账号/密码，保存LCP和Event Timing原始条目。最终性能运行在本阶段其他验证进程退出后单独执行；诊断after-r1保留且不用于正式时延结论。

| 指标 | Before | After | 变化 |
| --- | ---: | ---: | ---: |
| 初始传输字节（各5次一致） | 404515 | 323890 | 下降19.93% |
| 初始压缩body字节 | 400915 | 311290 | 下降22.36% |
| 实际初始JS gzip字节（不含CSS/字体/HTML） | 324941 | 235230 | 下降27.61% |
| 初始JS请求数 | 8 | 38 | 更多共享小chunk，请求开销已计入 |
| LCP中位数（ms） | 3984 | 3260 | 下降18.17% |
| LCP原始5次（ms） | 4332, 3688, 4260, 3984, 3964 | 3320, 3260, 3260, 3284, 3236 | 非线上p75 |
| 受控输入最大交互时延原始5次（ms） | 96, 64, 56, 56, 72 | 48, 56, 56, 48, 40 | 实验室INP代理 |
| 全部路由JS gzip字节总和 | 731836 | 787302 | 增加7.58%，不是初始传输指标 |

完整逐资源URL/长度/时序、LCP条目、interactionId/时长和初始模块列表见[原始性能数据](phase-29-performance.json)。实际加载的模块中不存在AppLayout、CommandPalette、AppRail、AppSidebar、TopBar、use-notification-socket或socket.io-client；三个产品的production专项与测量产物的全部JS/CSS逐文件摘要一致。全路由gzip总和增加55466字节，独立chunk的压缩/加载边界存在成本，不声称所有路径的累计下载量降低；本阶段验收收益限于实际初始加载。没有按包名拆React/vendor或提高warningLimit。

[预算](phase-29-budget.json)由baseline及诊断实测冻结：初始transfer≤330000B、encoded body≤320000B、LCP中位≤4184ms、受控交互最大≤100ms。正式复测均满足。总传输20%是候选，实际结果如表，没有将接近值虚报为达到20%；旧2.5s LCP候选也不冒称已达到。受控输入Event Timing最大值是本次固定交互的实验室INP代理，不是线上INP或生产SLO；未出现16ms以上条目时记录null及阈值说明，不填0。增加的共享请求数、HTTP/gzip与真实HTTPS/缓存/CDN差异需要生产环境另测。

## Actual Verification

命令均由Node 24.18.0执行；完整原始日志路径、SHA、退出状态和运行索引见[runtime](phase-29-runtime.json)。没有继承Phase28或早期阶段的PASS。

| 实际命令 / 范围 | 结果 |
| --- | --- |
| node tests/phase29-build.mjs before / after（先后各production Vite构建） | PASS；模块图、最终磁盘gzip/SHA保存 |
| node frontend/node_modules/typescript/bin/tsc -b frontend/tsconfig.json | PASS |
| node frontend/node_modules/eslint/bin/eslint.js frontend/src frontend/vite.config.ts | 退出0 / 0 errors / 61 warnings，warning均在本阶段修改范围外；不是零warning |
| node tests/browser/run.mjs --browser chromium --report target/phase-29/full-chromium-r2.json | 11套件 / 209 PASS / 0 FAIL / 0 SKIP |
| Chrome / Edge既有10套件及修正后production专项 | 各200既有组PASS，加各9新组PASS；原整轮FAIL保留 |
| node tests/phase29-linux-validation.mjs target/phase-29/linux-contracts-r3 | 隔离Linux 391 PASS / 0 FAIL / 0 SKIP；自有容器退出并移除 |
| static-image-policy、nginx-render-policy、ci-runner、ci-workflow、browser results/wait | 55 PASS / 0 FAIL / 0 SKIP |
| node tests/phase29-nginx.mjs target/phase-29/nginx-r3 | 7真实请求PASS；旧hash保留，404纯文本，清理PASS |
| node tests/phase29-performance.mjs before；after performance-final.json；node tests/phase29-compare.mjs | 两侧各5冷启动、预算PASS |
| node tests/phase29-evidence.mjs | 来源、相同产物、原报告、124历史文件、容器清理逐项核验PASS |

生产专项9组逐项为：匿名深链及私有模块排除；面板按需/快捷键/关闭重开；render throw/局部重试/真实侧栏导航；lazy404/无自动刷新/手动刷新；pending lazy下壳DOM保持/可导航；拒绝管理路由时不加载页面/不请求业务API；HTML创建校验/快速接入payload/发布引导；编辑不可变entry/runtime元数据；未注册ESM在写入前拒绝。

既有200组包括micro-host47、通知10、设置11、地区12、实体/登录13、路由权限64、导航14、共享会话17、部门11、聊天1，均在本次实现上实际执行；实体内部25场景按13个PASS组计数，不混用计量单位。三个产品的本次production JS/CSS均与正式性能产物相同；隔离Linux输入与最终输入之间的差异只在本阶段审计工具/README，业务、授权和执行契约字节保持一致，详见runtime。

## Failed / Diagnostic Runs

保留失败，不追改原始结果：Windows首次contracts缺少工单Vite依赖，且文件symlink用例因EPERM跳过；安装锁依赖，并在隔离Linux补齐391项零跳过。Linux首轮因WSL命令字符串变量展开而失败，改为stdin脚本；次轮缺生成的兼容public镜像，按两个public源码重建后r3通过。新增浏览器夹具先缺显式权限，随后修正真实侧栏button选择器和将document请求与同页history事件分开计数；三产品首轮整体均为202 PASS/1 failed suite，失败仅在新增专项，不能称整体通过。专项最终各9 PASS，另完整重跑Chromium为209 PASS。CI旧总数200/10更新为209/11，原政策失败保留。

Nginx首轮发现扩展名MIME继承，修复types空表；次轮旧资源摘要校验揭示generateBundle尚非Vite最终字节，采集改为writeBundle/磁盘值，原初步清单另存build-generatebundle.json，第三轮真实旧资源字节通过。初始传输/LCP/Event Timing来自浏览器，不受这项build清单修正影响。Zod注释位置的Rollup warning保留，没有靠调阈值消除。最终审计脚本首轮错误要求lint日志为空；实际ESLint退出0但有61个范围外warning，修正分类并保留原失败日志，没有抹掉warning或顺手修改无关源码。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 登录入口不加载完整命令面板/导航/通知连接链；初始传输较同口径基线下降。 | PASS | 逐模块/逐资源证明，初始transfer下降19.93%；候选20%不冒称达标。 |
| 注入页面异常时壳导航/恢复入口仍可用，旧chunk404不变成HTML。 | PASS | 真实production render throw、lazy404、pending注入；局部重试/侧栏/显式刷新；Nginx404 text/plain且旧hash仍可取。 |
| 主流程/授权/表单回归无退化，报告给出原始测量而非仅总包大小。 | PASS | 完整Chromium209、三产品新增9及既有200、Linux391；原始资源/LCP/交互、预算与限制已归档。 |

## Compatibility / Rollback / Risks

无新增依赖、API/SDK/数据库迁移。部署回退采用完整宿主前端+静态Nginx的上一固定镜像版本，保持旧hash并遵循现有history流程；不覆盖删除版本资源。源码回退只撤本阶段加载/恢复与应用编辑用例变更，使用target中before四文件作差异参考，不能重置整个脏工作树。新增Nginx命名错误位置可随同一宿主版本整体回退。

边界捕获React渲染及lazy加载失败，无法终止同步死循环，也不取代可信微应用的超时/故障锁存。对已缓存失败的lazy模块不做无效循环重试，由用户刷新获取当前HTML；没有自动刷新。页面重试会重置本页未保存状态，导航/组织切换仍按既有规则退役草稿。Chrome/Edge旧整轮失败与修正专项分列，不能伪造三份全绿整轮。没有重跑Rust/真实PG、完整Linux fast/delivery、远端CI、真实生产HTTPS或线上INP；这些不作为本次通过证据。

## Suggested Task Ledger Update

Phase29：COMPLETED；输出为本报告、performance/budget/inputs/runtime机器证据及[终态建议](phase-29-ledger-update.md)。全部三条原验收PASS；本阶段无剩余工作/新增阻塞问题。局部恢复、零自动reload、按真实测量冻结本地预算为本次决策；生产性能与产品版本支持矩阵仍属既有Q09/Q10边界。下一建议Phase30“优化授权查询链并收敛Rust能力模块”保持NOT_STARTED，需新的明确请求；不自动执行。
