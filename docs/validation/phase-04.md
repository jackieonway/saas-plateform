# Phase 4 执行与验证证据

日期：2026-10-15（Asia/Shanghai）。阶段：建立项目受管的分层 CI 与隔离测试环境。结果：**COMPLETED**。本文按实际执行的 manifest 记录结果；[台账](../architecture-optimization-ledger.md)已同步完成状态并将指针移至 Phase 5 NOT_STARTED。本次只执行 Phase 4。

## 1. 边界与源码绑定

用户明确要求执行 Phase 4，前置 Phase 1–3 已完成，复用已授权的 WSL 隔离环境。Git HEAD 保持 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，main 分支；保留全部已有未提交内容，没有创建提交、推送远端或改动生产数据。HEAD 只标识原提交；实际输入必须结合每次 `source-manifest.json` 的逐文件 SHA-256 和 `inputsSha256` 读取。

Linux 实测使用 Ubuntu WSL 的本地 Unix Docker socket，Docker 29.1.3、Compose 2.39.4、linux/amd64；Windows 驱动使用 Node 24.18.0/npm 11.16.0。工具镜像固定 Rust 1.97.0、Node 24.18.0/npm 11.16.0，浏览器层固定 Playwright 1.64.1；每次实际品牌浏览器版本单独记录。下载使用正常 TLS 验证；需要时仅为本次构建传入既有 loopback 代理，没有修改 daemon/global 配置或关闭证书验证。

每次干净测试都复制白名单源码，排除私有 env/npm 配置、密钥、`.git`、node_modules、dist、target 和个人缓存；容器无工作区、home 或 Docker socket 挂载。下载浏览器仅来自项目 `.browsers` 或固定工具镜像层。测试网络和容器使用唯一 owner label，删除前验证归属。PG 为独占私网、随机临时凭据与 tmpfs，不发布宿主端口。

各门禁在不同源码快照上运行，不能把它们的摘要写成同一个最终版本：

| 实测 | 实际输入摘要 | 说明 |
| --- | --- | --- |
| Linux fast r2 | `325c2e118a4814db55bc15f8f620babe0c5a0af90a934f95dc20100f6073a305` | 已含构建先于契约、受控 npm、串行 Vite 和 POSIX 进程组清理修复；后续 launcher 取消回归与说明文档另测 |
| Linux PostgreSQL r1 | `5ce0a2dc2e05a0e48d3442bd4822c1dac233de255c221ab98126dad0dc71d7bc` | 现有平台 Rust 与四个 DB 模块实际运行；后续 helper 报告/取消边界加固不冒充本次原始快照 |
| Linux Chrome r1 | `5a225151c0e48b8df849627d1900a24dbd5b088eb2bfb8983ba7194aac374df0` | 完整 143 组通过；部门等待助手最终变更后另补跑受影响的 11 组 |
| Linux Chromium / Edge r1 | `5cbb732d7acb5edfdb1c05e26bc667a1742c050db840ce36d19821b47714e20a` | 两个独立环境各完整执行 143 组；部门最终变更另补跑，不冒充同一快照 |
| delivery r3 镜像输入 | `e77009d4a4e461da0176d2f96a4c17486071dca555c5be6048600ca9352a4ee4` | 独立生产输入 manifest；CI 调度快照另列，详见交付 JSON |

最终工作树白名单 [phase-04-inputs.json](phase-04-inputs.json) 包含 395 项输入，`inputsSha256=00b536afd8b8baaa2406d074af952a7d560758c2ba8bbeabf366a1bcaa6fca27`。与已运行快照的逐文件差异保存于 [phase-04-ci.json](phase-04-ci.json)：fast 后仅改 launcher/其回归及三份使用说明；三浏览器后涉及部门等待助手的变更单独补跑；PG 后没有平台 Rust 变更；最终树中原交付生产 manifest 的 289 项文件全部摘要一致。CI 调度源码和文档变更不能冒充已重新构建的镜像。

## 2. 实现与必测清单

入口为 [scripts/ci/run.mjs](../../scripts/ci/run.mjs)，使用说明见 [CI README](../../scripts/ci/README.md)。它安装精确 lockfile、过滤命令环境、单独配置 npm 缓存与配置、记录工具实际版本及逐步结果，并对 TAP/Cargo/浏览器/PG/交付报告分别判定。必需失败、超时、skip/todo/cancelled、零测试、缺报告和清理错误均失败；依赖失败后的步骤是 NOT_RUN。

| 层 | 必须执行 | 验收边界 |
| --- | --- | --- |
| fast | 四个前端与根/CI 包干净安装；构建；lint/typecheck；70 项无 DB 契约；34 项部署单元；CI 策略；19 项 dev/Vite；Rust all-targets check、config 单元及独立 transport fixture | 平台为单 bin crate，显式指定 `backend/Cargo.toml` / `--bin saas-platform-backend`；没有空根 workspace 测试 |
| postgres | production_security 8、platform_settings 4、authorization_isolation 3、entity_creation 6 | 21 项真实隔离 PostgreSQL 测试；显式选取 ignored 模块并要求实际数量，不把未选项目算通过 |
| browser | micro-host 21、notifications 10、platform-settings 11、regions 12、entities 13、route-permissions 64、departments 11、chat 1 | 每产品 8 套件/143 个必测检查组，真实 Vite/React UI + 隔离 API/Socket.IO fixture；不是完整真实业务授权 |
| delivery | Compose、三镜像构建、完整 OCI archive 图验证、平台 ELF/UID/ldd、真实 Linux Docker/Nginx/HTTPS 七组 | 构建、smoke、registry 发布与目标部署分别报告；没有秘密、外部推送或生产部署 |

[GitHub workflow](../../.github/workflows/production-build.yml) 在 Ubuntu 24.04 上运行 fast → PostgreSQL 与 Chromium/Chrome/Edge 三产品矩阵；main push/手工候选再运行 delivery。最终聚合 gate 始终检查所需 job，failure/cancelled/skipped 均拒绝。权限为 contents:read，checkout 不保留凭据，无 pull_request_target、生产 secret、registry 登录、推送或目标主机步骤。只上传列出的脱敏报告和镜像 archive，不上传含临时 TLS 私钥的 smoke 工作目录。

Firefox/WebKit 仅显式扩展，不替换三项必需产品，WebKit 不等于 Safari 产品支持。Chrome/Edge channel 每次报告真实版本，当前结果不证明“最近两稳定版”矩阵已全部覆盖。

## 3. 实测结果

各组存在重叠，不能相加为独立总数。命令步骤（安装、构建、静态检查）不伪造测试计数。

| 实际检查 | 结果 | 原始证据（仓库相对路径） |
| --- | --- | --- |
| 干净 Linux fast r2 | PASS；19/19 步骤，0 fail/skip；前端 build/lint/typecheck、契约 70/70、部署 34/34、dev/Vite 19/19、Rust all-targets check、config 3/3（86 filtered）、transport 8/8；外层 exit 0、清理无错 | `target/phase-04/linux-fast-r2/reports/fast/summary.json`；持久摘要 [phase-04-ci.json](phase-04-ci.json) |
| 干净 Linux PostgreSQL r1 | PASS；21/21，0 fail/skip，21 个自有库清理断言及 pool 关闭均确认；容器/网络清理无错 | `target/phase-04/linux-postgres-r1/`；持久摘要 [phase-04-postgres.json](phase-04-postgres.json) |
| Linux managed Chromium | PASS；156.0.8078.4，8 套件/143 组，0 fail/skip；外层 exit 0、cleanupErrors=[] | `target/phase-04/linux-browser-chromium-r1/` |
| Linux Google Chrome | PASS；155.0.8059.79，8 套件/143 组，0 fail/skip；外层 exit 0、cleanupErrors=[] | `target/phase-04/linux-browser-chrome-r1/` |
| Linux Microsoft Edge | PASS；155.0.4283.45，8 套件/143 组，0 fail/skip；外层 exit 0、cleanupErrors=[] | `target/phase-04/linux-browser-msedge-r1/` |
| 最终源码 Linux 部门套件补跑 | Chromium/Chrome/Edge 各 11/11 PASS，wait 回归各 3/3，0 fail/skip；使用最终 395 项输入摘要，均 exit 0、清理无错 | `target/phase-04/linux-departments-final-r1/matrix-result.json`；持久摘要 [phase-04-browser.json](phase-04-browser.json) |
| Windows 浏览器补充 | Chromium 156.0.8078.4 / Chrome 154.0.8037.99 / Edge 142.0.3595.94 各 143/143，0 fail/skip | `target/phase-04-browser-windows-matrix.json`；持久摘要 [phase-04-browser.json](phase-04-browser.json) |
| 实际必需浏览器缺失负例 | 正确 FAIL，exit 1，pass 0/fail 1/skip 0，无个人缓存回退 | `target/phase-04-browser-missing-environment/result.json` |
| 最终 CI 工具回归 | Linux 干净容器及 Windows 各 41/41 PASS、0 fail/skip；含创建中、attach 中及报告复制时取消回归；Linux Mounts=[]、无端口绑定、清理无错 | `target/phase-04/ci-final-linux-r4/{result.json,tests.log}`、`ci-final-windows-r4.log`；持久摘要 [phase-04-ci.json](phase-04-ci.json) |
| 真实 delivery r3 | PASS；5/5 步骤、Compose 4/4、HTTPS smoke 8/8（父测试+七组），0 fail/skip | `target/phase-04/delivery-final-r3/`；持久摘要 [phase-04-delivery.json](phase-04-delivery.json) |
| POSIX 进程组关闭回归 | 旧实现被 12 后代真实监听进程用例复现为 FAIL；修复后 Linux dev-runner 13/13、0 skip，实际四项目 Vite 独立三轮全部 PASS | `target/phase-04/dev-cleanup-regression-r1/{before,after}.log`、`dev-vite-linux-r1/result.{log,json}` |
| Windows dev 补充 | 13 PASS、0 FAIL、1 个预期 POSIX 专用 skip；真实四 Vite 及退出立即重绑端口通过；不充当必需 Linux 零 skip 门禁 | 本会话实际输出；未单独归档 Windows 日志，必需 Linux 证据另列 |
| Workflow 政策与 actionlint | 政策 3/3、0 skip，含 17 种错误修改拒绝；actionlint 1.7.7 exit 0 | `target/phase-04-workflow/{workflow-tests-final,actionlint-final}.log` |
| 最终源码/台账/清理复核 | 395 项摘要一致；37 阶段中仅 1–4 COMPLETED，指针 5 NOT_STARTED；最终查询所有 `saas.ci.owner` 容器/网络均为空；git diff --check exit 0 | `target/phase-04/final-resource-audit.json` 及 [phase-04-ci.json](phase-04-ci.json)；diff 只有既有换行提示 |

Windows Chrome/Edge 的完整运行发生在部门等待助手最终修正前，最终助手在两产品各补跑部门 11/11；Windows Chromium 最终完整运行使用修正后源码。Linux 三产品完整运行也发生在最终部门助手修正前，最终受影响套件在三产品各补跑 11/11、等待回归各 3/3，使用最终输入摘要 `00b536afd8b8baaa2406d074af952a7d560758c2ba8bbeabf366a1bcaa6fca27`，运行前后源码摘要一致。补跑是原 143 组中的重复子集，不算 154 个唯一组。源码关系、最终清单比对及报告 SHA 保存在浏览器与 CI JSON，没有将多次运行描述为同一快照。

工具镜像实际 ID：基础 `sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7`；浏览器层 `sha256:66ac2ee1834bf8ccec2cfce67f5bd9f72a3382488ae170134fafc4cb493f3048`。PG 首轮实际使用其报告中的早期工具镜像 ID，未篡改成后续镜像。浏览器依赖 `npm audit` 0 vulnerabilities；现有平台/前端的历史依赖审计结果不因此消失。

## 4. 镜像与真实恢复验收

最终 buildId 为 `32956c4532bb-e77009d4a4e461da`，架构 linux/amd64。三份 archive 的 manifest/config/layer 字节均已校验；每个镜像 commit、buildId 与输入清单一致。

| 镜像 | 实际 OCI manifest digest |
| --- | --- |
| frontend | `sha256:d7bc94a08230852284959d5fe3fe68eeb5a63f802376c2f3d29e5407948c6f30` |
| edge | `sha256:b2a670b5364b2c507542f2f47b222eb6e8090255f536a18c1b1b23a2cb1558e5` |
| platform | `sha256:25dc278ea4ebdb9e4e7e8e4a4c9aa6d1f22c967aa4c337b6d7085ad83111d346` |

平台镜像实际只读、network none 探针确认 UID 10001、ELF machine 62（x86-64）、所有动态库解析成功，运行环境无 Node/npm/Bun/Caddy。二进制 SHA-256 为 `529233ac3c558fba6b5f5128021794b01b18473f7370355cbdebd06a9514f78b`；探针不启动服务或连接数据库。

真实 smoke 使用独立入口/宿主 Nginx 与 test-only Rust transport，覆盖：非 root/只读及私网；正常 CA/SNI HTTPS 与代理原字节；polling/WSS Upgrade；静态/导航/版本/安全头；上游换 IP 不 reload 自动恢复；ZIP 静态只读、重建后持久性；宿主更新/回退保留旧 hash 入口字节。此处“恢复”为容器、路由与静态资源恢复，不是 Phase 31 的业务数据库灾难恢复，也不是未来 Rust Socket.IO 协议验收。

报告明确 `registryPublication=NOT_REQUESTED`、`targetDeployment=NOT_RUN`。OCI manifest digest 是已验证内容摘要，不能声称 registry 已存在同一产物或目标主机已部署。真实 registry、生产 CPU 架构、DNS/TLS、主机及发行凭据仍见台账 Q04；本次没有猜值或外部发行。

## 5. 失败、修复与限制

- fast r1 保留 FAIL：干净源码未含生成兼容副本，执行顺序错误；实际 Vite 还暴露 npm CLI 选择和 POSIX guard 已退出但子进程组尚未排空的竞态。修复为先完整构建、固定选中 npm、串行有监听者的集成文件、等待自有组 ESRCH 并有界 TERM/KILL。新增真实监听后代回归，旧实现明确失败，修复后成功。
- 陈旧浏览器 fixture 按当前真实路由、ZIP 表单、公共 branding、租户守卫更新；部门等待助手捕获已成功的一次结果，避免再次异步取样造成假失败，并等待实际 retry HTTP 响应。只改测试基础设施，未重写业务 UI 或扩大权限。
- delivery 早期 APT 网络超时与 Windows DrvFS 直接导出 archive 的 ENOMEM 均保留为 FAIL。最终使用正常构建级代理、专属 WSL ext4 导出目录和有界流式回传，完整校验后清理暂存；没有全局清缓存或重启 WSL。浏览器子门禁完成后外层报告复制曾延迟，最终全部正常退出并完成清理。
- launcher 使用共享 cleanup Promise；创建/复制中的取消等待当前操作结束后清理，attach 中取消先核验 owner 再强制移除本次 runner，使迟到启动也无法继续挂起，最后清理 PG/网络。共享清理、延迟网络/runner/PG 创建、报告复制、运行中及 created-but-not-running attach 都有可控回归。最终 Linux/Windows 41 项工具检查全部通过；取消不充当 PASS。
- 额外 CI 工具审计 harness 首轮错误地将 npm user/global 配置设为同一 `/dev/null`，npm 正确拒绝，未运行测试且记录 FAIL；改为两个独立配置文件后通过。它不属于产品 gate 失败，没有隐藏或沿用失败报告。
- 既有平台严格 fmt/Clippy 债务、历史默认组 52 ignored、宿主 lint warnings 仍按 Phase 1 单列；本次没有批量格式化或关闭规则，也没有把剩余 ignored 算 PASS。当前 gate 只承诺明确 manifest，未迁移 Node 业务后台、未纳入套件、完整业务联合验收、监控和灾难恢复保持各自阶段。
- GitHub workflow 本地政策与 actionlint 通过；本次没有推送代码、触发或读取远端 workflow 运行，也没有修改分支保护。远端实际运行状态为 NOT_RUN。

## 6. Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 干净 Linux 执行无 DB、临时 PG、受管浏览器，无个人缓存 | PASS | fast 19 步骤、PG 21、三浏览器各 143 实测 PASS；最终部门助手另补受影响套件，源码关系分列 |
| 必跑失败/skip 阻止 gate，分列计数与源码/工具版本 | PASS | 策略负例、实际缺浏览器负例；TAP/Cargo/结构化报告 fail closed |
| 镜像目标架构、commit、buildId、digest 清单及 PR 秘密隔离 | PASS | 三份 linux/amd64 OCI archive 完整校验；只读 workflow 无生产秘密 |
| 单 crate 显式 Cargo 目标，预发布与外部发行限制清楚 | PASS | bin-only 显式清单；本地 HTTPS 实证、远端 workflow/registry/生产部署分别 NOT_RUN/NOT_REQUESTED |

全部必需实测与自有资源清理已完成。按台账的阶段同步要求，已更新摘要、详情、决策、变更日志与指针。

## 7. Suggested Task Ledger Update（已同步）

### Phase Status Update

Phase 4: COMPLETED。Phase 5: NOT_STARTED。

### Output Summary / Completed Items

- 四层 CI、精确安装入口、必测 manifest、脱敏结果与工作树源码摘要落地。
- 干净 Linux fast、选定 PG、三浏览器与最终受影响套件、真实三镜像/HTTPS smoke、最终工具回归及自有资源清理全部通过。
- 持久输入与 CI/PG/browser/delivery JSON、根 README、架构、工程基线和台账同步完成。

### Remaining Items

本阶段无待完成项。既有 Rust 债务、未迁移业务及完整灾备继续按所属阶段处理，不计本阶段 PASS。

### New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 当前 manifest 分层且必需 skip/零测试失败 | 不以未迁移服务或 ignored 隐藏缺口 | 平台 bin-only 和选定 DB/浏览器范围明确 |
| 各源码快照与补跑子集分别绑定 | 后续测试修正不能冒充原始运行输入 | 三产品完整 143 与最终部门 11 分列，无重复总数 |
| 校验 OCI 候选，外部发行另记状态 | 本次未授权 registry/生产操作 | 无推送/生产部署；后续使用真实环境和同一已验收产物 |

### New Open Questions

无阻断本阶段的新问题。已有 Q04/Q05/Q10 的生产环境、完整浏览器产品版本及目标恢复空间仍由运维/产品在相应后续验收前提供。

### Next Recommended Phase

Phase 5: 完善 ZIP 兼容静态包的原子安装与恢复对账。需要新的明确执行指令，未自动开始。
