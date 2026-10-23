# qiankun 迁移实施与验证

更新日期：2026-10-23。用户确认本轮目标为本机开发与验收。审查后已实施源码、默认配置、发布工具和持续检查修复，使用自有临时数据库、HTTP 服务、浏览器与容器验收；没有切换现有业务库或生产环境。

**当前活动源码和本机交付链已统一采用 qiankun 2.10.16、HTML/classic 或 UMD、SDK 2.0.0。** 七个真实子应用可挂载、更新、卸载，最终 HTTPS 网关的 CSP 也已通过真实浏览器验证。本机候选与已有环境的应用登记、活动版本是不同状态，候选不会自动改变已有平台配置。

## 范围和运行边界

独立微应用包括工单、门店、Vue、jQuery、报表、聊天和 OAuth。系统管理与 Hermes 等内置页面继续由宿主 React Router 承载；`/embed/*` 是外链嵌入功能。通知仓库是独立 Rust Socket.IO 后端。它们不构成旧微应用 loader 的回退路径。

SDK 包版本为 2.0.0，MicroProps 为 2.0，应用上下文与通知协议仍为 1.0。后端 combined/legacy 是 HTTP 授权策略，和前端 loader 无关。SDK 的 1.0.0 发布目录、旧迁移及历史业务包保留原字节；它们不参与当前微应用接入和默认验收。

## 已落实的修复

| 问题 | 当前行为与实现位置 |
| --- | --- |
| 网关默认 CSP 阻断 qiankun | `backend/nginx/config.example.json` 和宿主策略默认启用可信 HTML 求值；micro-static 传输文档保持独立策略。宿主 Nginx 示例替换上游旧 CSP，保留准确资源来源限制 |
| 工单/门店 Origin:null | 两服务的 Rust、Node 兼容配置、Compose 和模板均显式要求准确宿主 Origin，拒绝缺失、空值、null、通配符；生产要求 HTTPS，开发仅额外允许准确回环 HTTP |
| 旧宿主示例副本 | 删除 `frontend/public/sub-apps/vue` 和 `jquery` 八个旧文本产物，Vite 仅提供显式版本资源，非版本旧入口 404 |
| 旧 null 来源例外 | 移除 Vite 旧预检插件和后端 `bootstrap.rs` 两个 external view 的 null 特例，平台 CORS 统一消费配置白名单 |
| 门店默认旧版本检查 | 启动/验证要求显式版本，校验完整 qiankun 产物和完成状态；不再默认检查历史 1.0.1 |
| OAuth 失效开发入口 | 仓内调度器启动自身 API 与前端，不调用缺失的工作区根脚本 |
| OAuth 网关路径不一致 | 提供 `/oauth-playground-api/*`、`/sub/oauth-playground/*`、callback 和 assets，保留真实 Origin，剥离平台 Authorization/其他 Cookie；稳定文件名不缓存旧版本 |
| query/hash 更新遗漏 | `MicroAppContainer.tsx` 交付完整 pathname/search/hash；权限与异步授权边界使用独立 pathname，查询与 fragment 仅影响应用状态 |
| 成员服务等待占用业务连接 | 工单预检查显式等待连接归还连接池，再发起成员 HTTP 请求；真实数据库契约核对等待期间无业务连接或工单行锁占用 |
| 过期工单测试入口 | backend npm test 执行真实 tests；迁移回归核对当前三份 SQL 的文件名、摘要与重跑前后记录 |
| 平台超限资源测试的空响应体 | fixture 提供真实边界字节，并核对线路上的声明长度、chunked 超限与恰好上限；保留生产 16 MiB 资源限制 |
| 发布工具覆盖风险 | 版本目录排他预留，资源排他复制，拒已有/未完成版本与符号链接祖先；门店完成标记最后写入，启动/验证必须检查它 |

可信 Origin 只决定浏览器来源准入，不替代应用 Bearer、功能 permission、数据授权、租户及组织边界。无 Origin 的服务端调用仍执行原有身份核验。

## 产物门禁和回归

宿主构建及 CI 检查活动 src/public/SDK 和当前 dist，拒绝旧 bridge、旧全局生命周期、自带子应用副本。宿主本身仍使用 Vite ESM；子应用 HTML 使用 classic/UMD。历史 hash 静态资源保留流程与活动入口检查分别管理。

宿主和门店使用锁定的 parse5 开发依赖解析 HTML，正确处理脚本 raw text、注释、实体和无引号属性。针对“脚本字符串中的注释起始符隐藏后续 module 标签”的探针，已使用真实 Edge 对照验证。示例使用已有的 inert DOM 解析环境并禁用解析时脚本/外部加载。解析器用于构建/测试，不进入宿主运行包。

门店门禁检查 UMD 的四个生命周期以及绑定实际 bundled SDK 的只读版本协商契约；无 SDK 空壳、旧 SDK 和伪称版本但接受旧协议均被拒绝。工单门禁核对固定 HTML、公开配置、实际 UMD、源码摘要和文件清单。五个示例实际执行当前构建的生命周期，检查 SDK 1 拒绝、SDK 2 挂载、更新、卸载及宿主保留。

平台目录检查基于 Git 跟踪文件与非忽略新增文件。忽略的本机辅助目录不参与交付；意外跟踪或非忽略新增目录仍失败。本机 `local-application-rebuild` 未被删除。

## 本地候选

| 应用 | 本次生成的位置 | 使用约束 |
| --- | --- | --- |
| 工单 | `saas-work-orders/frontend/deploy/micro-apps/work-orders/versions/2.0.1-qiankun/` | html/public-config/candidate，面向 http://127.0.0.1:3000，API /work-orders-api |
| 门店 | `saas-stores/artifacts/micro-apps/stores/versions/2.0.3-qiankun/` | 使用最终门禁与完成标记，面向 http://127.0.0.1:3000，API /stores-api |
| 五个示例 | `saas-examples/artifacts/candidates/qiankun-20261023-current-v3/` | 五份完整前端、实际工具链与源码/包锁/验证器/文件摘要清单，由各自独立工程构建 |

这些是本地未激活候选，不能作为已上线凭据。门店本轮较早的 2.0.0/2.0.1 缺少最终 SDK/完成标记契约，2.0.2 与工单 2.0.0 则早于最后的 Node/构建输入更新；旧候选均保留原字节，不作为当前源码候选。最终七应用均用 Node 24.22.1 另建新版本，并核对当前源码摘要；五示例新增 build-provenance，旧 current/v2 不再作为当前源码候选。原 1.0.0/1.0.1 历史包没有改写，聊天旧 ZIP 与 retired-release 继续只作历史材料。

正式 HTTPS 宿主确定后，需要用准确来源另建新版本。生产 build-static 的 HTTPS 校验没有放宽。本机接入也须通过平台的应用登记、当前环境入口、角色菜单与数据授权；文件存在不会自动启用应用。操作步骤分别见各业务部署文档与[接入指南](micro-application-onboarding.md)。

## 验证结果

当前仓库工具基线为 Node 24.22.1、npm 11.16.0、Rust 1.97.0，见[发布日志](../../RELEASE.md)。最终 Windows 验证和候选生成实际使用 Node 24.22.1、npm 11.19.1；npm 版本满足 engines 的 11.x 范围，但与 packageManager 声明的 11.16.0 不同。隔离 Linux 容器内实际使用 Node 24.18.0、Rust 1.97.0。容器证据覆盖迁移与运行行为，不作为 Node 24.22.1 安全基线已在 Linux 全面验证的证明。本机 shell 曾优先匹配其他软件附带的 Node 16，验证进程显式选择 Node 24，未修改机器全局配置。

| 范围 | 本次结果 |
| --- | --- |
| 宿主前端 | 396 项测试通过；SDK、类型、构建与 389 份文本产物门禁通过；ESLint 无 error，47 条既有 warning |
| 七应用真实浏览器 | 最终七份候选在隔离 Linux Chromium 156.0.8078.4 + qiankun 2.10.16 薄宿主中 7/7 通过挂载、更新、卸载与宿主保留；另有 HTML parser 与真实 Edge 执行对照 3/3 |
| 最终网关 | Nginx 1.28.0 + Chromium 156.0.8078.4 + qiankun 2.10.16，15/15 通过、无跳过 |
| 工单前端 | 88/88，通过 17 项发布门禁及最终本地候选校验 |
| 门店前端 | 115/115；另有发布/安装专项 3/3；实际候选门禁通过，包含 HTML 属性、SDK 空壳、不可覆盖、并发和路径检查 |
| 示例前端 | 既有 JavaScript/网关 68 项通过；最终新增来源/HTML 回归 3 项通过。五份 Node 24.22.1 实际构建、SDK/生命周期及当前源码/包锁/验证器摘要门禁通过 |
| 平台普通 Rust | Linux 原生 all-targets 144 项通过、0 失败；131 项默认 ignored 不计通过。main 138、authorization_policy 4、retention 2；格式、Clippy -D warnings、rustdoc 通过 |
| 平台发布数据库专项 | 29/29：初始化/历史升级、qiankun 准入、URL 覆盖、预检、激活回滚、ZIP/镜像就绪、清理保护及冻结旧客户端 |
| 平台身份/授权数据库专项 | 20/20：组织隔离与撤权 3、会话刷新 4、接入授权 6、数据授权 7 |
| 工单 Rust | 显式测试 Node listener 下库测试 60/60，doc 1/1；check/fmt/Clippy 通过 |
| 工单真实数据库 | 正式 Rust 78/78、无跳过：迁移 27、读取 13、写入 14、成员 12、提醒 12；另有 Node 兼容契约 59/59。保留原 CI 归属门禁与原生 Rust HTTP 监听 |
| 门店 Rust 与数据库 | 显式 Node listener 下库测试 58/58、doc 1/1；真实 PostgreSQL contracts 2/2，含 8 组 SQL 断言；check/fmt/Clippy 通过 |
| OAuth Rust | 显式 Node listener 下 17/17，原 6 个上游契约失败全部通过 |

网关验证使用自有临时证书、回环上游和 `--network none` 容器，验证最终 CSP、真实 qiankun 生命周期、宿主保留、未批准外源拒绝，以及 OAuth 路径与 Cookie/Authorization/Origin 隔离。没有修改现有 Nginx 站点或容器。

最终候选浏览器验收读取本节列出的工单、门店和五份示例原始文件，在容器内的 `http://127.0.0.1:3000` 提供临时薄宿主，传入 SDK 2 的 platform/services 及合法隔离身份 UUID。空权限、空数据授权和 API fixture 403 保持拒绝边界；不执行真实业务写入、OAuth 或聊天。52 份候选文件在验收前后逐项摘要一致，无页面异常、CSP 违规或外源请求；门店实际接受有效身份，主题更新为 dark。该证据覆盖真实 qiankun 与最终候选的集成，不覆盖平台完整授权 adapter；授权、路由和上下文更新由宿主组件与数据库专项分别验证。

平台数据库使用本次独立初始化的 PostgreSQL 18.6 实例；用例自行创建和删除随机数据库。门店另用显式 owner marker 限定的 loopback 非默认端口模式，原 CI 专属主机保护继续保留。工单兼容测试同样只操作本次实例的随机自有库。全部数据库验收完成后，自有临时实例已停止并清理。任何测试成功均不代表真实用户环境已经迁移或授权。

## Windows HTTP fixture 与历史客户端

本轮较早验证中 Node 监听可达、Rust 测试进程自身 TCP/HTTP 监听超时，因此增加了显式测试监听选项。最终 Windows 重跑时，Node 自身回环 HTTP 与 Edge/Chrome 访问 Vite 也出现超时；完整宿主 `--all-apps` 未进入生命周期断言，平台普通测试亦有 HTTP fixture 超时，不能把此前成功结果作为最终源码已在 Windows 完整通过的证据。最终候选浏览器和平台普通测试改在隔离 Linux 环境核验，没有修改防火墙、代理或生产网络行为。

测试监听兼容方式只移动监听，正式 Rust HTTP 客户端、Axum 处理器、字节/头/期限/并发/鉴权断言保持。环境允许 Node 回环访问时，可显式设置以下变量运行对应测试；默认 Linux CI 仍走原生 Rust listener：

| 仓库 | 可选环境变量 |
| --- | --- |
| 平台 | `MICRO_PROXY_TEST_NODE` |
| 工单 | `WORK_ORDERS_TEST_NODE` |
| 门店 | `STORES_TEST_NODE` |
| OAuth 示例 | `OAUTH_TEST_NODE` |

变量值是已有 Node 24 可执行文件的绝对路径。测试结束或取消时回收自有子进程，不依赖正在运行的用户服务。CI 增加显式 Node 模式，以便持续检查该分支。

平台资源上限回归已分别在原生监听和显式 Node 监听下通过，先完整读取上游响应并核对线路长度，再验证恰好 16 MiB、声明长度超限和合法 chunked 超限。Linux 补验的 Node 路径为 `/usr/local/bin/node`、实际版本 24.18.0；没有将损坏的 HTTP 传输产生的 502 当成大小限制生效。

发布专项还恢复了拆仓时遗漏的冻结旧客户端四个文件，放在 `src/release_tests/fixtures/phase31/`。它们与历史提交 ac345cd 的 Git 对象逐字一致，adapter SHA-256 为 `5e729854cb91568f3b13dbc6db22f2a93b814958735a42291e5c62a5fdfe9cc8`。旧客户端仍通过真实 HTTP 调用当前 Router，没有用新写的替代断言冒充兼容验收。

## 当前限制和后续维护

本轮未进行生产部署、修改现有业务数据库、替用户登记真实 OAuth 客户端或启用真实资金/消息操作。新 CI jobs 已在本地验证其相关命令，尚未在远程 GitHub runner 运行。原生 IPv6 listener 诊断专项未在受限 Windows 环境运行；其余尚未选择的 ignored 专项不能计作已通过。

SDK 源仓库仍登记为 REPOSITORY_CANDIDATE，消费者来源包含 workingTree 状态，不能把当前摘要一致性表述为已发布正式 npm 版本。可选镜像 worker 仍需显式路由目录挂载/include；恢复工具仍是固定三库与指定业务版本的 pilot。这些能力边界见[部署](deployment.md)与[恢复](../ops/recovery/README.md)，不改变当前 qiankun 接入方式。

项目文档已同步当前实现。最终检查 69 份活动第一方 Markdown，276 个本地文件链接全部存在，152 个代码块围栏闭合，18 段 JSON 示例全部解析通过；10 个外部链接未做网络可达性验证。旧迁移分析、重复项目总览和失效状态报告已在文档重建时清理；有效身份、API、运维和支付业务约束保留。支付 45 阶段仍为未实施规划。完整入口为[文档导航](README.md)。
