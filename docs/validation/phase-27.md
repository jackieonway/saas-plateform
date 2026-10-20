# Phase 27 交付门店 Vite 微应用与双消费者契约实证

日期：2026-10-18 12:36:34（Asia/Shanghai）。阶段验收结论：**COMPLETED**。独立门店 React／TypeScript／Vite 微应用、精确 stores 登记与管理 CLI 接入、真实普通用户业务闭环及工单＋门店共享契约均已完成。严格收集器对本次唯一 r2 冻结输入的完整 fast 与 Chromium／Chrome／Edge 三个定向真实浏览器门禁重新核对，结果为 `PHASE27_GATES_PASS`。本报告的通过数仅来自这四个 r2 门禁。

## 输入、范围与证据边界

- source revision：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`。
- 输入 SHA256：`02304b07f8c9b25a34b124898c7ce06ef283f092eb95910dde73fa828b559b9d`；文件数 809；冻结时间 2026-10-18T04:09:40.265Z；候选 [target/phase-27/inputs-candidate-r2.json](../../target/phase-27/inputs-candidate-r2.json)。
- 物理快照：`D:\WorkSpace\rust\saas\rust-saas\target\phase-27\source-candidate-r2`。每次执行均重新核对宿主选择器、冻结快照及各自实际执行副本的完整文件集合与字节；运行前后源码保持一致。
- [最终边界](../../target/phase-27/boundaries-final-r2.json)绑定 [root 精确路径审批](../../target/phase-27/approved-source-paths-r2.json)；相对阶段开始共 88 个已审批路径。r2 相对 r1 仅修复三个验证生产者／策略测试文件，原 r1 输入、审批、边界与失败结果全部保留。
- 114 份历史正式证据逐字节保持，其他 36 阶段保持。没有修改 Phase26 正式证据，也没有继承 Phase21／26 的通过数量。
- 严格审计 [target/phase-27/managed-audit-r2.json](../../target/phase-27/managed-audit-r2.json)，文件 SHA256 `2efa45b8bc87dac391c5c3cc963d9202e6faff7ca88892b6c5f419eb0696a1b2`；核对 625 份原始材料，材料列表 SHA256 `5d73771dccdf34ebdccc143b115b17eb035cef4fd412414390d437abfb380fc3`。正式 runtime 另绑定最终工具与本报告等正式输出，不包含台账或自身的递归指纹。

本阶段采用独立 React 19.3.0、TypeScript 5.9.3、Vite 8.3.4、登记 SDK 1.0.0／契约 2.0。薄入口同步连接 bridge，再加载 React 动态 chunk；生产相对 base 与公开 config、asset-manifest、build-info 一起构成可复核产物。stores 保持独立稳定 appCode，11 项菜单、9 个 resource/action capability、3 个 memberLookups；工单保留独立的 7 项菜单与用途。没有新增公共 UI 框架或公共业务 crate。

门店资料、负责人目录、员工关联／解除、每日营业记录和服务端汇总都调用真实 Rust API。金额保持十进制字符串，营业日明确采用用户确认的上海日期；409 保留输入并显式重读、采纳版本；写入响应丢失保留原组织、目标及原日期，授权读回后核对，不自动重发。client 的 15 秒绝对期限覆盖 fetch 与正文；403 从响应头到达即冻结对应操作，迟到成功不能恢复；跨路由／续期的未知写恢复保留原操作锁。

## 实际门禁与计数

| 门禁 | 状态 | 命令步数 | 原始解析 counts | 浏览器版本 | 原始结果 | UTC 执行时间 |
|---|---|---|---|---|---|---|
| fast | PASS | 63 | 1033/1033; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 1931 | — | [outer result](../../target/phase-27/final-fast-r2/result.json) | 2026-10-18T04:10:39.782Z → 2026-10-18T04:34:59.579Z |
| browser-chromium | PASS | 14 | 21/21; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 | 156.0.8078.4 | [outer result](../../target/phase-27/final-browser-chromium-r2/result.json) | 2026-10-18T04:10:39.780Z → 2026-10-18T04:20:26.146Z |
| browser-chrome | PASS | 14 | 21/21; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 | 155.0.8059.84 | [outer result](../../target/phase-27/final-browser-chrome-r2/result.json) | 2026-10-18T04:10:39.807Z → 2026-10-18T04:20:31.877Z |
| browser-msedge | PASS | 14 | 21/21; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 | 155.0.4283.62 | [outer result](../../target/phase-27/final-browser-msedge-r2/result.json) | 2026-10-18T04:10:39.797Z → 2026-10-18T04:20:29.766Z |

完整 fast 的 63 个冻结计划步骤全部通过，mandatory failure／skip 均为 0；TAP／Cargo 实际合计 1033/1033，filtered 1931 单列，未当作执行通过。浏览器每产品 21 个 typed 组分别报告，不能加入 TAP／Cargo 总数。fast CLI TLS 的实际 TAP 父测试为 1/1，其内部具名检查 9 项；各定向浏览器中的 CLI TLS 直接执行脚本，各有 9 个具名检查，没有 TAP 父测试。命令、父测试、具名检查和浏览器组不相加成一个总数。

| 关键 fast 项 | 状态 | 实际 counts |
|---|---|---|
| `stores-frontend-contracts` | PASS | 114/114; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 |
| `stores-units` | PASS | 54/54; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 |
| `platform-admin-tests` | PASS | 31/31; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 |
| `platform-admin-tls` | PASS | 1/1; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 |
| `ci-policy` | PASS | 139/139; fail 0; skip 0; cancelled 0; 未实现测试 0; filtered 0 |

四个实际 launcher 命令如下；每个命令均绑定同一 candidate 文件、revision、SHA 和独立输出目录。完整 argv、退出码、超时、日志及工具副本见对应 outer／container report。

| 执行 | 实际 launcher argv |
|---|---|
| fast | `"D:\\Software\\Scoop\\apps\\nodejs\\24.18.0\\node.exe" "D:\\WorkSpace\\rust\\saas\\rust-saas\\target\\phase-27\\validation-runner\\run.mjs" --source-root D:/WorkSpace/rust/saas/rust-saas/target/phase-27/source-candidate-r2 --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --inputs-sha256 02304b07f8c9b25a34b124898c7ce06ef283f092eb95910dde73fa828b559b9d --candidate-file target/phase-27/inputs-candidate-r2.json --out-dir D:/WorkSpace/rust/saas/rust-saas/target/phase-27/final-fast-r2 --gate fast` |
| browser-chromium | `"D:\\Software\\Scoop\\apps\\nodejs\\24.18.0\\node.exe" "D:\\WorkSpace\\rust\\saas\\rust-saas\\target\\phase-27\\validation-runner\\run.mjs" --source-root D:/WorkSpace/rust/saas/rust-saas/target/phase-27/source-candidate-r2 --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --inputs-sha256 02304b07f8c9b25a34b124898c7ce06ef283f092eb95910dde73fa828b559b9d --candidate-file target/phase-27/inputs-candidate-r2.json --out-dir D:/WorkSpace/rust/saas/rust-saas/target/phase-27/final-browser-chromium-r2 --gate browser --browser chromium` |
| browser-chrome | `"D:\\Software\\Scoop\\apps\\nodejs\\24.18.0\\node.exe" "D:\\WorkSpace\\rust\\saas\\rust-saas\\target\\phase-27\\validation-runner\\run.mjs" --source-root D:/WorkSpace/rust/saas/rust-saas/target/phase-27/source-candidate-r2 --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --inputs-sha256 02304b07f8c9b25a34b124898c7ce06ef283f092eb95910dde73fa828b559b9d --candidate-file target/phase-27/inputs-candidate-r2.json --out-dir D:/WorkSpace/rust/saas/rust-saas/target/phase-27/final-browser-chrome-r2 --gate browser --browser chrome` |
| browser-msedge | `"D:\\Software\\Scoop\\apps\\nodejs\\24.18.0\\node.exe" "D:\\WorkSpace\\rust\\saas\\rust-saas\\target\\phase-27\\validation-runner\\run.mjs" --source-root D:/WorkSpace/rust/saas/rust-saas/target/phase-27/source-candidate-r2 --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --inputs-sha256 02304b07f8c9b25a34b124898c7ce06ef283f092eb95910dde73fa828b559b9d --candidate-file target/phase-27/inputs-candidate-r2.json --out-dir D:/WorkSpace/rust/saas/rust-saas/target/phase-27/final-browser-msedge-r2 --gate browser --browser msedge` |

工具由四个 container 原始版本日志核对：Node v24.18.0、npm 11.16.0、rustc 1.97.0 (2d8144b78 2026-07-07)。Windows 启动器固定为 `D:\Software\Scoop\apps\nodejs\24.18.0\node.exe`。Linux 工具镜像 `sha256:1912f7302185ed2c1af4df155d0d2784886c87984266ed981325575e415d71dc`；各浏览器的 PostgreSQL 镜像、工具镜像及实际安全参数由原始 outer report 记录。Playwright 1.64.1；三个真实产品版本列于门禁表，不以 Chromium 代替 Chrome 或 Edge。

## 独立安装、构建与产物

三个产品各自将 stores 工程复制到仓库外的独立目录，安装前没有 node_modules，并创建初始空的自有 npm cache。原始证据均确认 rootNodeModulesDependency=false、personalCacheDependency=false、outsideRepository=true、emptyCacheBeforeInstall=true、secretSentinelAbsent=true。各自产物与本次 frontend 源文件、锁文件、SDK 和输入摘要逐字节绑定；门店构建输入各 30 项。

| 产品 | 实际独立命令 | 状态 | 原始日志 |
|---|---|---|---|
| chromium | `npm ci --no-audit --no-fund` | PASS | [原始日志](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/artifacts/logs/standalone-npm-ci.log) |
| chromium | `node "<owned-work>/standalone-stores/scripts/build.mjs" --out-dir "<owned-work>/stores-app" --version phase27 --allowed-host-origin https://127.0.0.1:43347` | PASS | [原始日志](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/artifacts/logs/standalone-npm-build.log) |
| chrome | `npm ci --no-audit --no-fund` | PASS | [原始日志](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/artifacts/logs/standalone-npm-ci.log) |
| chrome | `node "<owned-work>/standalone-stores/scripts/build.mjs" --out-dir "<owned-work>/stores-app" --version phase27 --allowed-host-origin https://127.0.0.1:40505` | PASS | [原始日志](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/artifacts/logs/standalone-npm-build.log) |
| msedge | `npm ci --no-audit --no-fund` | PASS | [原始日志](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/artifacts/logs/standalone-npm-ci.log) |
| msedge | `node "<owned-work>/standalone-stores/scripts/build.mjs" --out-dir "<owned-work>/stores-app" --version phase27 --allowed-host-origin https://127.0.0.1:35077` | PASS | [原始日志](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/artifacts/logs/standalone-npm-build.log) |

| 产品 | 应用 | dist 文件数 | 构建清单 | 资源清单 |
|---|---|---|---|---|
| chromium | stores | 9 | [build-info](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/artifacts/stores-dist/build-info.json) | [asset-manifest](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/artifacts/stores-dist/asset-manifest.json) |
| chromium | work-orders | 9 | [build-info](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/artifacts/work-orders-dist/build-info.json) | [asset-manifest](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/artifacts/work-orders-dist/asset-manifest.json) |
| chrome | stores | 9 | [build-info](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/artifacts/stores-dist/build-info.json) | [asset-manifest](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/artifacts/stores-dist/asset-manifest.json) |
| chrome | work-orders | 9 | [build-info](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/artifacts/work-orders-dist/build-info.json) | [asset-manifest](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/artifacts/work-orders-dist/asset-manifest.json) |
| msedge | stores | 9 | [build-info](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/artifacts/stores-dist/build-info.json) | [asset-manifest](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/artifacts/stores-dist/asset-manifest.json) |
| msedge | work-orders | 9 | [build-info](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/artifacts/work-orders-dist/build-info.json) | [asset-manifest](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/artifacts/work-orders-dist/asset-manifest.json) |

每个 dist 文件的完整路径、长度、SHA256 在严格审计的 buildArtifacts／workOrdersBuildArtifacts 及 runtime 中；两业务均核对物理文件集合与实际字节，stores 另核对 relative module 入口、动态 chunk、build-info 输入和 package-lock，并验证 secret sentinel 不进入最终字节。宿主生产构建由各浏览器独立执行，实际命令及日志已归档。开发者工作区补充构建单独收录，不替代仓库外空缓存实证。

四个原始 Linux ELF（平台、门店、工单、管理 CLI）均从各实际构建路径复制到只含公开证据的归档。收集器复核 ELF 头、长度和 SHA，并将实际运行的二进制报告与这些原文件绑定。各产品原始 [chromium archive-proof](../../target/phase-27/final-browser-chromium-r2/public/archive-proof.json)、[chrome archive-proof](../../target/phase-27/final-browser-chrome-r2/public/archive-proof.json)、[msedge archive-proof](../../target/phase-27/final-browser-msedge-r2/public/archive-proof.json)记录完整物理归档清单；严格审计／runtime 索引保留所有 ELF、dist 字节与 SHA256，供 Phase28 镜像化。

## 真实业务、opaque iframe 与双页实证

三个产品均运行生产平台、门店 Rust、工单 Rust、三个独立 PostgreSQL 库和非超级业务账号，使用生产宿主、Nginx 与正常验证的 HTTPS。初始化应用／菜单／grant 通过正式 API 和 Rust CLI；普通业务账号执行业务，不用 mock 业务响应，不以 SQL 直写角色或 grant 代替授权。SQL 只用于受控身份准备和读取断言。

实际相对 ESM 入口和动态 chunk 在 opaque iframe 中加载；两个 Rust 业务的 Origin:null OPTIONS 都为 204，allow-origin 为 null，不允许 credentials，native fetch 为 omit 且无 Cookie。主题变更保持 formId 和合法金额，额外业务请求为 0；路由离开取消旧读，迟到汇总不回填，未知路由拒绝。真实应用 token 续期等待服务实际签发新的 201，而非缩短 TTL 或替换时钟，保留 11.22 输入并替换 formId：

| 产品 | 实际 issue 次数 | 续期状态 | 保留金额 | 实测 elapsedMs |
|---|---|---|---|---|
| chromium | 1 → 2 | 201 | 11.22 | 268776 |
| chrome | 1 → 2 | 201 | 11.22 | 268744 |
| msedge | 1 → 2 | 201 | 11.22 | 268806 |

两真实应用共用宿主会话与当前组织。实际四个页面先具有 A 组织的门店／工单／汇总／目录快照，再切换 B；旧内容清除，旧提交入口探测均产生 0 次跨组织写，原始 oldHandlerOutcome 分别为 chromium=`iframe-replaced`、chrome=`iframe-replaced`、msedge=`iframe-replaced`。两页刷新后仍在 B，退出传播至四页。实际 401 清除旧内容，403 清除数据并冻结操作，纯主题不能解冻。故障只丢失或延迟真实响应：日报实际提交后丢响应，再显式读回；只写无读场景实际提交后保持未知，提示无法读取核对；两类自动重试均为 0。

每产品全部 21 个原始 typed 组及逐请求记录见 [chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json)；收集器从各自 `logs/15-real-browser.log` 重解析实际结果与 cleanup。普通业务、两类未知写、版本冲突及三种跨应用／用途拒绝都有对应组，五项验收的精确组映射见 Self-Check。

实际截图：[chromium / phase27-real-create-initial.png](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/screenshots/phase27-real-create-initial.png)；[chromium / phase27-real-ledger-light.png](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/screenshots/phase27-real-ledger-light.png)；[chromium / phase27-real-form-dark.png](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/screenshots/phase27-real-form-dark.png)；[chrome / phase27-real-create-initial.png](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/screenshots/phase27-real-create-initial.png)；[chrome / phase27-real-ledger-light.png](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/screenshots/phase27-real-ledger-light.png)；[chrome / phase27-real-form-dark.png](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/screenshots/phase27-real-form-dark.png)；[msedge / phase27-real-create-initial.png](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/screenshots/phase27-real-create-initial.png)；[msedge / phase27-real-ledger-light.png](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/screenshots/phase27-real-ledger-light.png)；[msedge / phase27-real-form-dark.png](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/screenshots/phase27-real-form-dark.png)。截图与路径、PNG 签名及 SHA256 一起校验；金额汇总采用不换行显示，宽表保留局部滚动。

## 双消费者成员契约及差异

两个业务均真实调用平台 `/api/apps/{app}/members` 受控 HTTP，应用 Basic 内省、普通用户 Bearer、当前 permission／grant／组织硬上限按各自用途校验。成员候选不授予权限；新负责人／员工或工单处理人仍由后端复验，不读平台身份表。

| 产品 | stores 实际成员请求 | 工单实际成员请求 | 业务 DTO id 字段 | 跨业务 Bearer | 平台跨应用 | 未登记 purpose |
|---|---|---|---|---|---|---|
| chromium | 6 | 4 | id / userId | 401 | 403 | 400 |
| chrome | 6 | 4 | id / userId | 401 | 403 | 400 |
| msedge | 6 | 3 | id / userId | 401 | 403 | 400 |

业务跨应用 Bearer 实际拒绝为 401；平台跨应用成员调用实际拒绝为 403；未登记 purpose 实际拒绝为 400。三种状态分别记录，不能将旧开发文字中的 purpose 403 当作本次结果。权限／grant 缺失同样实际拒绝。

| 差异 | work-orders | stores |
|---|---|---|
| 成员用途 | ticket/create、ticket/update | store/create、store/update、store-staff/assign；日报及解除关联不新增成员用途 |
| 目录与提交 | 平台 id → userId → assigneeUserId | 平台 id 保持 id → managerUserId 或 userId |
| SELF owner | 工单创建人，处理人不扩大 SELF | 门店创建人／被关联员工／日报录入人，各资源分别校验 |
| 创建恢复 | Idempotency-Key 与 receipt 对账 | 没有 receipt；保留原业务域显式读回，不自动重发 |

完整差异和范围边界见 [双消费者成员契约实证](../../docs/stores-member-contract.md)，该文档已由本次审计指纹绑定。SELF、ORG、ORG_SUBTREE、TENANT 保持现有语义，不能按序号比较或借目录扩大组织硬上限。平台成员见证与业务提交之间仍无跨库共同事务；从请求前起最多 3 秒的绝对见证窗口不因锁等待续期，最后校验到 COMMIT 之间的撤权竞态保留。Phase30 有两个真实消费者、HTTP 请求与字段差异可用，不提前抽象公共能力。

## 原始失败、跳过与修复审查

[开发及被拒绝 r1 证据索引](../../target/phase-27/development-index-r2.json)包含 32 项记录。原始 status、exit、日志和审查文件均按实际字节保留；[首份索引](../../target/phase-27/development-index.json)也保持原样。开发观察不计入上面的四门禁统计。

最初浏览器开发的源码复制权限、局部变量初始化、Docker inspect CAP_ 命名与选择器／响应读取问题均保留 FAIL；r4 EOF 取消后内层公开报告未导出，只能将曾观察的 16 组列为 OBSERVED，不能重建原始结果。集成开发的三项原始 TAP 失败、Windows POSIX-only skip、错误的环境 Node 及修正后的运行分别保留，不能把 skip 计作 PASS。

r1 四个 outer 结果均为 FAIL：fast 63 步中 62 步通过，ci-policy 的 1 项旧断言失败；三浏览器 raw 虽各记录 21 PASS，但 typed 及 outer 因 workOrders／work-orders 元数据字段不一致而失败。因此 r1 不采用。r2 修复生产者键名及其回归测试、限定旧阶段策略断言，并重新冻结、运行全部四门禁；未修改严格 parser 接受条件，也未回写旧 FAIL。收集器另对真实 Docker CAP_ 形式及显式审批路径做窄兼容，独立审查保留正向与负向拒绝结果。[恢复工具审查](../../target/phase-27/review-recovery-finalization-r2.json)和[审批路径审查](../../target/phase-27/review-recovery-approval-r2.json)只覆盖工具／修复范围，不替代门禁或台账终态审批。

## 资源、前台 lease 与清理

每次验证通过前台 WSL／Docker stdin lease 运行，完成前 stdin 持续打开；container 未在工作时遇到 EOF。所有资源绑定各自 owner，容器、网络的创建前检查与结束后精确 label 查询均归档。浏览器请求仅保留 CHOWN／SETUID／SETGID，capDrop ALL、no-new-privileges；fast 不加 capability；实际非 privileged、无 Docker socket、无宿主端口。

| 执行 | owner | 原始 lease | 结束资源审计 |
|---|---|---|---|
| fast | ci-phase27-1792296639781-b8241534 | launcher 27984; attach 47368; stdinKeptOpenUntilContainerExit=true; completedBeforeEOF=true; eofDuringWork=false | containers=""; networks=""; cleanupErrors=[] |
| browser-chromium | ci-phase27-1792296639778-691074ff | launcher 28272; attach 7008; stdinKeptOpenUntilContainerExit=true; completedBeforeEOF=true; eofDuringWork=false | containers=""; networks=""; cleanupErrors=[] |
| browser-chrome | ci-phase27-1792296639806-e1f3912c | launcher 35292; attach 29604; stdinKeptOpenUntilContainerExit=true; completedBeforeEOF=true; eofDuringWork=false | containers=""; networks=""; cleanupErrors=[] |
| browser-msedge | ci-phase27-1792296639797-da282f5c | launcher 27240; attach 33180; stdinKeptOpenUntilContainerExit=true; completedBeforeEOF=true; eofDuringWork=false | containers=""; networks=""; cleanupErrors=[] |

浏览器内层所有 cleanup 字段均 true：服务与 Nginx 停止、浏览器关闭、临时目录删除、三库及账号删除、host 字节保持；公开归档仅包含 allowlist 日志、报告、dist、截图和原 ELF，私有目录、连接串、私钥未导出。四次 outer 的 cleanupErrors 都为空，ownedContainersRemoved 与 ownedNetworkRemoved 都为 true，最终 owner 容器和网络查询输出均为空。

最终自有 Cargo cache 清理在四个采纳结果结束后执行，原始 [缓存清理证明](../../target/phase-27/final-cache-cleanup-r2/result.json)：owner `ci-phase27-cache-1792286916077-ebc2d9dd`，volume `ci-phase27-cache-1792286916077-ebc2d9dd-cargo`，ownerVerified=true，attachedContainers=`""`，volumeRemoved=true，remainingOwnedVolumes=`""`。清理脚本与实际四条命令也按 SHA 绑定；未清理其他阶段或全局缓存。

## Self-Check

| Acceptance Criterion | Result | Notes |
|---|---|---|
| 独立目录npm ci/build成功，无根node_modules/个人缓存依赖，dist无凭据。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`production-react-vite-independent-build`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |
| 普通用户在真实平台/门店Rust/业务库完成资料、员工关联、每日记录及汇总，不用mock或直写平台DB授权。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`ordinary-user-authorization-menu11`、`store-create-manager-directory`、`store-update-status`、`staff-association-unassignment`、`daily-create-update-duplicate-summary`、`staff-report-without-store-read`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |
| opaque iframe module/chunk、Origin:null/CORS、主题/路由/取消/续期/失权实际通过。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`opaque-origin-null-rust-no-cookie`、`theme-keeps-draft-no-request`、`route-cancellation-and-not-found`、`normal-renewal-keeps-legal-input`、`revocation403-freezes-clears`、`unauthorized401-clears`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |
| 双页工单+门店刷新/切组织/退出无跨组织旧数据或草稿提交；两应用token/权限/数据grant不串用。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`dual-host-shared-session-org-switch`、`dual-host-refresh-logout-fences`、`stale-form-handler-cannot-cross-org`、`cross-app-bearer-permission-grant-denials`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |
| 两业务同一成员协议实证和差异记录齐全，未登记动作/跨应用调用拒绝，Phase 30的输入无需虚构。 | PASS | Chromium、Chrome、Edge 均由原始日志重解析；`dual-actual-controlled-member-http`、`cross-app-bearer-permission-grant-denials`；[chromium typed report](../../target/phase-27/final-browser-chromium-r2/public/reports/stores-browser/result.json)、[chrome typed report](../../target/phase-27/final-browser-chrome-r2/public/reports/stores-browser/result.json)、[msedge typed report](../../target/phase-27/final-browser-msedge-r2/public/reports/stores-browser/result.json) |

## 限制与下阶段输入

本阶段完成门店前端、必要 CLI／SDK／CI 接入及本次普通用户真实浏览器验收。镜像发行、公开 v1／v2 URL、生产部署、流量切换和 Phase28 联合交付不在本次范围；本地正常 TLS 与生产构建不构成生产环境部署验收。没有提前发布公共框架；每产品21组覆盖本阶段场景，不等于每种scope组合、全部历史浏览器或PostgreSQL矩阵重跑。

草稿只保存在存活 iframe 内存，刷新／关闭可丢失；未知写没有自动补偿或自动重试；平台成员／grant 与业务事务间的最后校验至 COMMIT 竞态仍保留。日志中 Cargo filtered 只表示被筛选，开发中的 Windows skip 只表示环境限定，均未改写为实际执行通过。

下一推荐阶段为 **Phase 28：交付工单/门店镜像与统一外部 URL 接入**，保持 NOT_STARTED，须用户新请求后执行。输入包括两个真实 frontend dist 及其完整 SHA 清单、四种原 ELF、stores bootstrap／grant manifests、成员字段差异、明确的未知写与会话隔离语义，以及本次四门禁证据。本次 Phase27 验收无业务余项；台账由 root 对最终正式证据生成具体提案，经独立审阅后同步终态。

完整 63 个 fast command argv、原始 count／exit／timeout／日志逐项保存在 [fast 原始 summary](../../target/phase-27/final-fast-r2/public/reports/fast/summary.json)。三产品的 14 个外层步骤包含四个 locked Cargo 构建、宿主与既有工程 npm ci／host build、直接 CLI TLS、真实 stores browser；完整 argv 见 [chromium container result](../../target/phase-27/final-browser-chromium-r2/public/reports/container-result.json)、[chrome container result](../../target/phase-27/final-browser-chrome-r2/public/reports/container-result.json)、[msedge container result](../../target/phase-27/final-browser-msedge-r2/public/reports/container-result.json)。最后命令的 --browser 分别为 chromium、chrome、msedge；独立 stores 安装／构建在 real-browser 中以仓库外新目录运行，前文已逐产品列出。
