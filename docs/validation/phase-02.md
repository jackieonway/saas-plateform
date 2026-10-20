# Phase 2 执行与验证证据

日期：2026-10-15（Asia/Shanghai）。阶段：将静态兼容示例和根前端入口迁为 Vite。结果：**COMPLETED**。仅执行 Phase 2，Phase 3 及后续阶段未启动。

## 1. 起点、工具与源码绑定

- 用户明确要求按台账执行 Phase 2；前置 Phase 1 为 COMPLETED，三项验收及证据已存在。
- Git 基线仍为 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`、分支 main。开始时保留了 Phase 1 全部未提交变更，以及更早的 README/优化方案/台账文件。本次未提交 Git，不能把基线 SHA 当成本次变更后的提交。
- 本阶段 62 个实际源码/配置/锁文件输入记录于 [phase-02-inputs.sha256](phase-02-inputs.sha256)。该清单自身 SHA-256 为 `71264d7183df29f73c03576cfb9f392fb1c734b603b354db7280e0ca72fba63c`，按 Windows 工作树字节计算；Git 换行转换可能改变字节摘要。其余未改应用源码由上述 Git SHA 绑定。
- 实测环境：Windows PowerShell；Node **24.18.0**、npm **11.16.0**、宿主 Vite **6.4.3**，三个迁移示例独立精确锁定 Vite **8.3.4**。WebSocket 工程 TypeScript **5.9.3**、React/ReactDOM **19.3.0**、Socket.IO 客户端 **4.8.4**；兼容 Vue **3.5.43**、jQuery **3.7.1**。
- 默认 PATH 中旧 Node 16 仍需注意；本次命令统一将已安装的 Node 24 工具目录置于 PATH 首位，没有依赖个人全局 Vite。Linux/POSIX 启停路径已有实现，**没有实际运行 Linux、GitHub Actions、Nginx 或容器**。
- 没有读取私有 `.env` / `.env.test` 内容，没有启动平台/工单/通知/OAuth/聊天真实后台、执行数据库迁移、连接生产库、推送镜像或发布服务。浏览器使用隔离 profile、API/传输 fixture；构建秘密检查使用固定的测试哨兵。

## 2. 最终实现

### 独立 Vite 示例

`mini-services/micro-vue`、`micro-jquery` 各自提供 package/lock、根 `index.html`、`vite.config.mjs`、dev/build/preview、README 和真实静态交付测试。删除原 Bun `index.ts`，将原 public/index 迁至根入口，public 只保存经典脚本及 vendor。每个工程仅导入自身安装的 Vite；初始红测曾真实因为本地 Vite 缺失而失败，没有回退借用宿主依赖。

构建采用 `base: './'`、`appType: 'mpa'`，生产目录为 `dist/`。classic `app.js` 与 vendor 字节保持不变，继续暴露 `__MICRO_APP__` 的 mount/update/unmount 和原有取消/迟到响应处理。开发入口去除 Vite 的 ESM/HMR 注入，并保留相对资源路径，避免宿主 HTML loader 执行开发模块或把资源解析到宿主根路径。构建各有两条 classic script 无法按 module 打包的提示；相关文件由 Vite public 复制，内容一致性、实际加载和生命周期均已验证。

这两个工程是现有 **TRUSTED 兼容示例**；默认 opaque iframe 应用仍需 SDK 握手。本阶段没有放宽 iframe 权限，也没有把经典生命周期等同于 iframe 协议已适配。

`examples/websocket` 新增独立 React/TypeScript/Vite 工程，消除宿主组件别名和根 node_modules 依赖。默认离线，只有经过校验的公开 `VITE_CHAT_ENDPOINT` 能进入浏览器构建，且用户仍需显式点击 Connect。拒绝含凭据、路径、查询、片段或非 HTTP(S) 的配置；不读取 `.env` 或批量注入 VITE 变量。保留聊天事件和 `/` path，修复断开后重新加入状态；原 `server.ts` 保持未改、未运行，后续由 Phase 22/23 处理，与平台通知 `/socket.io` 契约不兼容。

### 根调度、构建与宿主静态入口

- 根 dev/dev:all/dev:frontend 使用 Node 内置 API 的 `scripts/dev.mjs`；其他示例 dev 代理到各自 npm。旧 shell 脚本转交同一前台调度器，移除个人绝对路径、ss/后台逃逸、自动启动 PostgreSQL 和源码复制。
- 默认仅启动 host/vue/jquery；dev:all 另加 websocket。各前端直接执行自身锁定的 Vite，端口严格、可配置，宿主代理跟随实际端口。启动有总 readiness deadline，慢响应头也受绝对计时器约束；早退、超时、Ctrl+C、父进程消失均清理自己拥有的进程树，不按端口/进程名杀其他监听者。
- `--with-backend/notifications/oauth` 保留为显式的旧服务开发选项，帮助及启动信息标明真实服务/迁移副作用，本次没有运行；不是新的 Node 后台实现。
- 根新增 npm 锁并删除 `bun.lock`。根旧 Socket.IO 等依赖保留用于未迁移的参考后台，没有宣称全仓 Rust 化。四个新/改 package 与锁中的 dependencies/devDependencies/engines 已做一致性核对。
- `build:all` 顺序为 **build:examples → micro:sync:examples（从 dist）→ build 宿主**，防止宿主携带旧兼容副本。内置副本已同步到 `frontend/public/sub-apps/{vue,jquery}`，不再从 public 源目录直接发布。
- 宿主仅在版本请求到来时解析 `MICRO_APPS_DEV_DIR`，缺目录不影响 fresh checkout 的 dev 启动；版本和兼容静态路径缺资源均返回真实 404。保留 realpath 限界、路径段校验、GET/HEAD、静态 MIME/CORS 与版本缓存语义，未改管理 API CORS。
- 删除无调用方的 `tool-results/inspect18.ts` 旧 Bun.SQL 本机探针，未执行其查询。Bun 清除扫描覆盖本阶段有效 package/脚本/HTML/前端源码；历史审查文字及拒绝 Bun 的旧测试 fixture 文字不当作运行入口。

## 3. 实际命令与结果

`PASS` 表示实际成功；`FAIL` 表示实际失败；`NOT_RUN` 表示未执行。以下各组存在重叠测试，不相加为独立测试总数。最终必需组均零 fail/skip/cancel。

| 命令／检查 | 分类 / Exit | 实际结果 |
| --- | --- | --- |
| 根：`npm ci --ignore-scripts` | PASS / 0 | 从新根 npm 锁干净安装，36 packages added，audit 0 vulnerabilities；旧根依赖不参与微应用正式依赖解析 |
| `npm --prefix mini-services/micro-vue ci`、`run build` | PASS / 0 | 独立干净安装和 Vite 静态构建成功，audit 0 |
| `npm --prefix mini-services/micro-jquery ci`、`run build` | PASS / 0 | 独立干净安装和 Vite 静态构建成功，audit 0 |
| `npm --prefix examples/websocket ci` | PASS / 0 | 独立干净安装 33 packages，audit 0 |
| `npm --prefix mini-services/micro-vue test` | PASS / 0 | **7/7**：既有 6 项生命周期/迟到响应，加真实本工程 Vite dev/build/版本前缀 preview；零跳过 |
| `npm --prefix mini-services/micro-jquery test` | PASS / 0 | **7/7**：同口径，零跳过 |
| `npm --prefix examples/websocket test` | PASS / 0 | 先 TypeScript/build，再 **5/5**：公开 endpoint 校验、HTTP/版本资源、404、配置哨兵隔离；零跳过 |
| 根：`npm run test:dev` | PASS / 0 | **13/13**：12 个受控 fixture/CLI/清理回归，另 1 项实际四 Vite 工程集成；自定义代理端口、真实资源、缺资源 404、停止后端口重绑均成功 |
| 根：`npm run test:vite` | PASS / 0 | **5/5**：真实执行根 `npm run build:all`，四工程产物扫描；打包两个 dist 到自有临时版本目录并加载；宿主缺目录启动、资源及真实 404；零跳过 |
| 根：`npm run lint` | PASS / 0 | 0 errors、**64 条既有 warnings**；数量与 Phase 1 相同，未隐藏 |
| 根：`npm run test:contracts` | PASS / 0 | **70/70**，0 fail、0 skip、0 cancelled；包含原两个示例生命周期与取消测试 |
| `node tests/micro-host-browser.mjs vendors` | PASS / 0 | **1 个场景覆盖 Vue/jQuery 两应用**：真实宿主/Vue/jQuery、授权展示、续期 update、API token、退出清理 |
| `node tests/micro-host-browser.mjs releases` | PASS / 0 | **3 场景**：TRUSTED、IFRAME 版本替换及真实 Vue Vite build→打包→1.0.0/1.0.1 切换→回滚，旧 signal/callback 取消 |
| WebSocket 默认离线浏览器烟测 | PASS | Edge **142.0.3595.94**，正常渲染、0 JS error、0 WebSocket；截图在本地日志目录 |
| WebSocket：`npm run test:browser` | PASS / 0 | 同一 Edge；隔离拦截所有聊天传输，显式 Connect/Join/message/断开/重连加入及精确事件 payload 通过，未连真实聊天后台 |
| 新根、两个 micro、websocket 的 npm audit | PASS / 0 | 各工程均 0 vulnerabilities；未执行批量 audit fix |
| 活动入口/锁/源码一致性扫描 | PASS | 无 Bun API/命令、import.meta.dir、个人旧绝对路径；旧文件确实删除、4 组 package/lock 匹配；app.js/vendor/旧聊天后台/平台源码和迁移未改 |
| Git 空白检查及输入摘要复核 | PASS / 0 | `git -c safe.directory=<当前仓库> diff --check`；62 个输入摘要与最终工作树一致 |
| Linux/POSIX、GitHub runner、Docker/Nginx、真实 DB/后台/现网/恢复 | NOT_RUN | 本阶段为 Windows 上的受控前端交付验收，没有借用历史结果宣称通过 |
| Rust fmt/check/Clippy/test/doc | NOT_RUN | 平台 Rust/迁移未改，本阶段未重跑；Phase 1 的既有 fmt/Clippy 债务与 ignored 仍有效，不算本阶段新通过 |

浏览器宿主组实测工具：Node 24.18.0、Playwright **1.62.1**、已有 Chromium **151.0.7922.34**。使用工具返回的已有运行时，显式依赖路径只用于测试驱动；正式工程安装/构建不借它或宿主 node_modules。项目受管浏览器环境仍归 Phase 4。

### 旧全量浏览器脚本的真实失败

| 运行 | 实际结果 | 处理 |
| --- | --- | --- |
| 首次完整 micro-host-browser | 17 PASS、1 FAIL、3 NOT_RUN，exit 1 | 旧独立 Vue fixture 只复制 public，缺迁移后的根 HTML；已改为本工程 Vite 在自有临时目录真实构建，再制作两版本 |
| 修复后的完整 micro-host-browser | **18 PASS、1 FAIL、2 NOT_RUN，exit 1** | 独立 Vue 双版本链已通过，后续 management 首场因陈旧 fixture 失败 |
| 单独 management 组 | 首场 FAIL，exit 1 | 同一旧 fixture 问题复现，没有把其余未执行权限场景记为 skip/pass |

management fixture 把应用设为 `basePath: '/system'`，现 UI 仅为 `/micro/<code>` 显示发布控件；旧脚本后续还使用“登记版本”表单，而当前实现已为 ZIP/tar.gz 上传。相关管理 UI 业务源码本阶段未改，Phase 2 不扩展成发布 UI 重写。该旧全量套件 **仍失败**，应在 Phase 4 建立受管分层门禁时修订 fixture，再按发布相关阶段验收。阶段选定 vendors/releases 两组则均实际 exit 0，结果未混用。

### 先红后绿与调试记录

- 两微应用测试先因无本工程 Vite 失败；随后捕获 Vite dev 将脚本重写为根绝对 URL，修正为版本相对路径后通过。
- 宿主 fresh checkout 红测重现 `ENOENT`（不存在的版本目录），另一个红测重现缺 JS 返回 200 HTML；懒解析和静态路由修复后 3/3 通过。首轮配置异常留下的本次测试子进程已按确认的拥有 PID 停止，最终成功运行无残留。
- 审阅以持续未完成响应头复现 socket idle timeout 可绕过总启动预算；改为独立绝对计时器，1600 ms 启动预算的慢头回归在含清理预算内失败并释放端口。
- 四真实 Vite 集成首次发现测试未消费大资源响应体，停止服务器时触发 Node Undici 的 paused parser 断言；测试改为先消费响应体，最终 13/13 通过，WebSocket HTTP fixture 同步处理。

## 4. 静态产物与配置边界

| 工程 | 标准产物 | 兼容与交付验证 |
| --- | --- | --- |
| 宿主 | `frontend/dist/` | 根 build 真实执行 tsc + Vite；当前主要 index JS 为 **865740 bytes**，仅作后续同口径基线，不是性能达标声明 |
| Vue | `dist/index.html`、`app.js`、`vendor/vue.global.prod.js` | 两classic资源相对入口解析，源码/vendor字节未变；dev相对资源及版本前缀preview/宿主读取成功 |
| jQuery | `dist/index.html`、`app.js`、`vendor/jquery.min.js` | 同上；missing JS/vendor/CSS/图片/HTML/无扩展名均 404 |
| WebSocket 前端 | `dist/index.html`、`assets/*` | 相对 JS/CSS、版本前缀加载；开发服务拒绝 `.env` 和旧 server.ts，生产只交付 dist |

根集成构建注入固定测试哨兵：DATABASE_URL、JWT_SECRET、PLATFORM_SESSION、VITE_CLIENT_SECRET、VITE_HOST_SESSION；随后遍历四个 dist 的全部文件，确认哨兵、PostgreSQL URL、`.env`、`.npmrc` 和链接文件不在产物。WebSocket 独立测试另外确认只有经校验的公开 endpoint 被编译，其他 VITE/DB/session 哨兵不被注入。没有使用真实私密环境进行构建；运行时应用 token 仍通过宿主 props/上下文消费。

Vite dev/preview 是前端工具；生产只发布静态文件。版本 URL 使用已有打包器写入本次自有临时目录，不注册真实平台、不激活现网、不覆盖已有版本。根 deploy 仍缺失，Nginx/镜像交付留给 Phase 3。

## 5. 依赖、已知限制与回退

Vite 是每个示例开发/构建/预览所需的精确开发依赖；TypeScript/types 仅用于聊天前端检查，React/ReactDOM 和 socket.io-client 为保留现有页面及聊天协议所需，没有引入新 UI 框架或后台依赖。已有 Vue/jQuery runtime 及许可证头保留。直接依赖主要为MIT，TypeScript 5.9.3为Apache-2.0；传递构建依赖还含Apache/BSD/ISC，lightningcss及其平台包为MPL-2.0，只用于构建，未作为代码分发进dist。WebSocket的xmlhttprequest-ssl实际LICENSE/licenses[]为MIT。

宿主 64 条 lint warnings、原前端锁中的 source-map-js 旧审计问题与 Rust fmt/Clippy 债务沿用 Phase 1，本阶段未改宿主依赖或后台源码，也未伪称这些已解决。新根/示例审计结果为零不替代全仓审计。

本地详细日志在 Git 忽略的 `target/phase-02/`：根 ci/lint/contracts/dev-runner/vite-delivery；两个 micro 的 ci/build/test/relative-red/audit/licenses；websocket ci/test/browser/protocol-browser/audit；宿主 browser 的 before-fixture-fix、最终全量、management、vendors、releases；输入扫描及静态 red/green 日志。原始日志不随Git自动交付，本文件及输入清单固化了版本、命令、分类和关键结果。

回退仅涉及本阶段前端入口/锁/配置、开发脚本、静态副本、测试和文档；恢复相应文件后按各自锁重新 npm ci。原Bun入口可从Git历史找回，不代表目标栈允许重新投入生产。没有迁移或用户数据回退；不能整仓 reset 覆盖已有 Phase 1/方案修改。只清理本次拥有的临时目录/进程，未删除旧版本资源。

## 6. Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 两个静态微应用均能独立 npm ci/build；产物在版本前缀下加载且缺资源真实 404 | PASS | 各自安装/build/7项测试；实际 Vite preview、已有打包器+宿主版本目录加载；两应用真实浏览器兼容组及 Vue 双版本链通过 |
| 宿主与示例的有效前端启动命令不调用 Bun；未破坏现有生命周期/迟到响应测试 | PASS | 活动入口扫描；70项既有契约；13项开发测试含实际四Vite/代理/清理；app.js/vendor字节不变；vendors/releases浏览器组exit0 |
| 构建资源中不含 secret、数据库地址或宿主 session | PASS | 四dist的固定哨兵、数据库URL/敏感文件扫描；单公开endpoint校验与注入测试，运行时token机制未改 |

Phase 2 三项必需验收全部通过。以上 PASS 不包含明确失败的旧管理 UI fixture，也不代表后台 Rust 化、真实平台/数据库、Linux容器或生产发布完成。

## 7. Suggested Task Ledger Update（已按台账同步）

- Phase Status Update：Phase 2 `COMPLETED`；Phase 1 保持 COMPLETED，其余阶段 NOT_STARTED。
- Output Summary：两个独立 classic/Vite 微应用、独立聊天 Vite 前端、npm 根入口/锁、受控跨平台开发脚本、相对产物与真实404、源码摘要与本证据。
- Completed Items：上述三项验收全部 PASS；README/现状架构/工程基线同步。
- Remaining Items：Phase 2 无剩余必需项；旧完整浏览器 management fixture、Linux受管环境、生产Nginx及业务迁移仍是后续输入。
- New Decisions：兼容示例保留classic/TRUSTED生命周期；独立Vite8不强升宿主Vite6；根默认仅前端；统一构建先示例/同步再宿主；总deadline为绝对预算。
- New Open Questions：没有新增阻塞Phase 2的问题；原台账真实Linux/域名/TLS/镜像环境问题保持有效。
- Next Recommended Phase：Phase 3「恢复 Nginx 部署基线并交付宿主静态镜像」，前置1/2已完成，只更新指针，本次不执行。
