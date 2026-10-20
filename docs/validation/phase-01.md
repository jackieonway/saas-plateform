# Phase 1 执行与验证证据

日期：2026-10-15（Asia/Shanghai）。阶段：冻结实施基线、工具版本与验证口径。结果：**COMPLETED**。只完成 Phase 1，后续阶段未执行。

## 1. 执行边界与源码绑定

- 用户明确要求按台账执行 Phase 1；该指令覆盖台账创建时“未授权实施”的历史限制。阶段没有前置实施依赖。
- 源码基线：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`，分支 `main`。本次变更尚未提交，因此不能把该 SHA 描述成本次变更后的提交。
- 开始时已有 `README.md` 修改，以及未跟踪的 `docs/architecture-optimization-plan.md`、`docs/architecture-optimization-ledger.md`；均予以保留，在其基础上同步本阶段状态与文档。
- 执行环境为 Windows PowerShell、`x86_64-pc-windows-msvc`；本次没有执行 Linux runner 或 Linux 容器。Windows/Linux 通用命令已记录于[工程基线](../engineering-baseline.md)。
- 没有修改 `backend/src`、`frontend/src`、既有测试或数据库迁移，没有读取 `.env` / `.env.test` 内容，没有启动平台/业务服务、执行真实数据库测试、生产构建、镜像构建/推送或部署。
- Git 首次因目录属主与当前账号不同而拒绝访问；后续仅通过每条命令的 `git -c safe.directory=<当前仓库>` 读取状态，没有修改全局 Git 信任配置。

下列为本次验证所用配置与锁文件的 **SHA-256（Windows 工作树原始字节）**。执行源码和八个契约文件沿用上述 Git 基线；新增的质量入口由这些摘要绑定。跨平台换行转换可能改变字节摘要，不应把它当作产物摘要。

| 文件 | SHA-256 |
| --- | --- |
| `.nvmrc`、`.node-version`（内容相同） | `55075b5ec4e8b31936cbbc282b8829116d1fd48f2f2f1856dee592a6650700ce` |
| `rust-toolchain.toml` | `174a086284d57e79557b62bc627e96257893375dfd37239e3c3e830ce2629b95` |
| `package.json` | `0d4517f7e6038a54eee270d8d7e3e4915a85f3549b5edfd16ae25b59524d0062` |
| `frontend/package.json` | `169cdc1f45563214650e46e4e7412a3ad9f1b3c474f04ee15432d2392e0ad41b` |
| `frontend/package-lock.json` | `751dfe934c5439f331910a945070fc1735147a61fd40bdba949a3f90c8dbf8b8` |
| `frontend/eslint.config.js` | `aa3be290afdc3bfcc82fb7c81b36b6708e49dadf539168a1f0137152d5f549b4` |
| `frontend/.npmrc` | `7151cf397def0c2cb0ab65643701d27d335a72c90f775675b5f826bc7005818a` |
| `backend/Cargo.toml`（未改动） | `952f938f18b05d1e5eab055dbc6a5c45c408daa10a964e4f8773245db9b62abd` |
| `backend/Cargo.lock`（未改动） | `18e8b51f5dcfd03f35b2e6a2be4906a94bfd91d9ae1f9b0a76c3132cbce24b71` |
| `.github/workflows/production-build.yml` | `d38f48dffa857bacbc2e1194e8f177c963724dc0be072cf99225ce1a65328e1c` |
| `frontend/Dockerfile` | `09533b90a360e2de05254cbe55241bdc90adee306c8244e9e1341b92193a8895` |
| `backend/Dockerfile` | `97b07d3b9bc8735b5f24c94c8cd209841311e1483aafffdeaea0432394e8e47a` |

## 2. 工具版本及工程入口

| 工具 | 实际执行版本 | 固定方式与结论 |
| --- | --- | --- |
| Node | `v24.18.0` | `.nvmrc`、`.node-version`；根与 frontend 的 engines 限定 `>=24.18.0 <25`。默认 PATH 原为微信开发工具的 `v16.13.1`，验证时显式将 Node 24 安装目录置于 PATH 首位 |
| npm | `11.16.0` | 根与 frontend 的 `packageManager: npm@11.16.0`、engines `>=11.16.0 <12`；frontend `.npmrc` 启用 `engine-strict`；CI 与宿主 Docker 构建阶段显式安装此版本 |
| TypeScript | `5.9.3` | 来自 frontend 的 npm 锁文件；`tsc --noEmit -p tsconfig.json` |
| ESLint | `10.12.0` | 精确开发依赖及 flat config；检查 `src` 的全部 TS/TSX 与 `vite.config.ts` |
| Rust | `rustc 1.97.0 (2d8144b78 2026-07-07)` | `rust-toolchain.toml` 固定 `1.97.0`、minimal profile、rustfmt/clippy；本次用 rustup 实际安装并核验固定版本 |
| Cargo | `1.97.0 (c980f4866 2026-06-30)` | 同一工具链；唯一 crate 是 `backend/Cargo.toml`，锁文件为 `backend/Cargo.lock` |
| rustfmt / Clippy | `1.9.0-stable` / `0.1.97` | 实际运行结果见下表 |
| Git | `2.55.0.windows.5` | 用于源码、工作树及未改业务源码核验 |
| Docker | `UNAVAILABLE` | 当前 PATH 没有 Docker CLI；本阶段也未授权实际容器/部署验收 |

版本选择以本次可实际安装和编译的 Rust 1.97.0 为基线，将原 CI 的 1.99.0 和平台 Docker 构建镜像一并对齐；这不是声明新的最低 Rust 支持版本。前端 Docker 的 Node 从 24.3.0 对齐为 24.18.0，并在安装依赖前复制 `.npmrc`。这些仅为构建工具声明修改，**未验证镜像标签拉取、Linux 编译或容器运行**。生产 Caddy 运行阶段及缺失配置仍留待 Phase 3。

新增开发依赖分别是 `eslint@10.12.0`（执行器）、`@eslint/js@10.0.1`（官方 JS 规则）、`typescript-eslint@8.71.2`（TS 解析和规则）、`eslint-plugin-react-hooks@7.1.1`（hook 使用及依赖规则）。现有 TypeScript 编译器不提供这些 lint 规则，因此需要补充；包采用 MIT 许可、使用支持 ESLint 10 的组合，精确版本进入锁文件，未增加生产后台依赖。

根目录新增/修复 `lint`、`typecheck`、`test:contracts` 代理，以及 `check:rust`、`test:rust`、`fmt:rust`、`clippy:rust`、`doc:rust`。所有 Rust 命令显式指定 `backend/Cargo.toml`，没有引入 Cargo workspace。根目录目前没有 npm 锁，干净安装入口为 `npm --prefix frontend ci`；根 Bun 开发/构建入口保留为 Phase 2 待迁移项。

## 3. 实际执行结果

分类：`PASS` 为实际成功，`FAIL` 为实际失败，`SKIP/IGNORED` 为测试自身跳过，`NOT_RUN` 为未执行，`UNAVAILABLE` 为当前工具不可用。退出零与零告警/零跳过分别记录。

| 命令与目录 | 分类 / Exit | 结果 |
| --- | --- | --- |
| 修改前，frontend：`npm run lint` | FAIL / 1 | 在 Node 24 下复现 `eslint is not recognized`，依赖及配置缺失 |
| frontend：`npm ci` | PASS / 0 | npm 从锁文件干净重建依赖（328 packages added）；没有沿用个人 node_modules 作为验收依据 |
| frontend：`npm run lint`；根：`npm run lint` | PASS / 0 | 两个入口均实跑，0 errors、64 warnings，未隐藏 warning |
| frontend：`npm run typecheck`；根：`npm run typecheck` | PASS / 0 | 使用现有 strict tsconfig，独立运行 TypeScript；不生成 dist |
| frontend：`npm run test:contracts`；根：`npm run test:contracts` | PASS / 0 | 两个入口均实跑，70 tests、70 pass、0 fail、0 cancelled、0 skip、0 todo |
| 根：`cargo check --locked --manifest-path backend/Cargo.toml --all-targets`；`npm run check:rust` | PASS / 0 | 原生 Cargo 与根代理均实跑；1 个既有 dead_code warning |
| 根：`cargo test --locked --manifest-path backend/Cargo.toml --bin saas-platform-backend` | PASS + IGNORED / 0 | 89 total：37 passed、0 failed、52 ignored。没有把 ignored 计入通过，也没有执行 `--ignored` |
| 根：`cargo fmt --manifest-path backend/Cargo.toml --all -- --check` | FAIL / 1 | 38 文件、470 处格式 Diff hunk；业务源码未改动，记录为既有格式债务 |
| 根：`cargo clippy --locked --manifest-path backend/Cargo.toml --all-targets -- -D warnings` | FAIL / 101 | 普通 bin 输出 15 previous errors，test bin 输出 38 previous errors；target 诊断可重叠，不相加为独立缺陷数 |
| 根：`cargo doc --locked --manifest-path backend/Cargo.toml --no-deps` | PASS / 0 | 有 2 个既有 rustdoc warning，见下文 |
| 根：`node --test tests/production-native-config.mjs` | FAIL / 1 | 当前重现 1 test、0 pass、1 fail：顶层导入缺失 `deploy/native/render.mjs`，错误码 `ERR_MODULE_NOT_FOUND` |
| frontend：`npm audit --json` | FAIL / 1 | 1 项既有 high：`source-map-js@1.2.1`，原 SHA 的锁文件也是该版本；本次未引入且未执行 audit fix |
| `git -c safe.directory=<当前仓库> diff --check` | PASS / 0 | 已检查最终变更的空白错误 |
| Linux GitHub Actions / Docker build、Compose config/up | NOT_RUN | CI 文件仅修改版本声明；没有运行 runner、构建生产产物或操作容器 |
| 真实 PG、完整浏览器、TLS 网关、现网、恢复 | NOT_RUN | 不属于 Phase 1 的必需验收；未加载现有私有环境或将历史结果算入当前通过 |
| 原 `test:micro` 扩展组、网关/浏览器可选分支 | NOT_RUN | 必需组独立执行，未把可选分支的潜在 skip 混入 70 项结果 |

选定八个既有无数据库契约文件如下。它们使用内存、VM、受控时间或注入的请求实现，不读取 `.env`、连接真实数据库或调用真实外网。范围覆盖微应用生命周期/取消、通知上下文、会话刷新、地区解析、两个静态示例和后台授权参考。

```text
tests/micro-context-runtime.mjs
tests/micro-sandbox-lifecycle.mjs
tests/notification-context.mjs
tests/session-refresh.mjs
tests/map-location-regions.mjs
mini-services/micro-vue/smoke-test.cjs
mini-services/micro-jquery/smoke-test.cjs
examples/app-backend/authorize.test.mjs
```

原有 `test:micro` 和其它检查脚本未删除；其中 iframe 的真实浏览器分支与 Windows symlink 条件需要独立环境。这次 70 项是明确选择的契约集，**不是**旧 88 项历史集的重跑，也不是完整浏览器验收。

## 4. 已知失败、告警与接续边界

### 前端 lint 策略

64 warnings：37 个 unused-vars、18 个 exhaustive-deps、4 个 explicit-any，prefer-const / no-useless-assignment / preserve-caught-error 各 1，以及 2 个既有无效 eslint-disable。清理/类型/依赖告警在 Phase 1 保持可见，未使用 `--quiet`、全局忽略或批量改写组件来使检查归零；其余推荐错误规则和 `rules-of-hooks` 仍为 error。现有 tsconfig 也未开启 noUnusedLocals/noUnusedParameters。

只对 `micro/iframe.ts`、`notification-protocol.ts`、`PlatformSettingsPage.tsx` 和 `vite.config.ts` 中已审查的安全输入校验关闭 `no-control-regex`，因为这些规则明确需要匹配并拒绝控制字符。没有开启需要配套编译器迁移与行为验证的 React Compiler 规则。Phase 4 建立 CI 门禁及 Phase 27/28 局部重构时应以当前清单处理相应债务，不能把本阶段 exit 0 描述成零告警。

### Rust 与依赖债务

- `backend/src/config.rs:168`：`Config::from_env` 未使用；严格 Clippy 还命中既有 needless borrows、type complexity、参数数量等问题。Phase 1 交付检查入口与真实基线，没有批量格式化或修改授权/业务逻辑。
- rustdoc 两条 warning 位于 `backend/src/routes/notifications.rs:482`（`[01]` 被解析为链接）与 `backend/src/extractors.rs:12`（`<session JWT>` 被解析为未闭合 HTML）。
- 52 个 ignored 除隔离数据库测试外，还含单独的 IPv6 HTTP fixture；没有声称全是 DB，更没有为了清零执行会迁移数据库的 ignored 测试。
- `source-map-js` 旧传递依赖的 [GHSA-68fv-2mgg-jv7q](https://github.com/advisories/GHSA-68fv-2mgg-jv7q) 保留为待处理项；本阶段未进行无关依赖升级。npm 11.16 还提示 `esbuild@0.25.12` postinstall 未加入 allowScripts，本次未盲目批准安装脚本；lint/类型/契约验证均成功，生产构建能力不在本阶段宣称范围。

### 部署与运行入口

[工程基线](../engineering-baseline.md)已覆盖工单前后端、通知、OAuth、websocket 和 app-backend，以及所有 package/Cargo 与锁文件。生产 Node 的工单/通知入口、Node OAuth 演练、Bun 静态示例、Caddy、CLI 和历史测试入口均给出退出阶段，没有新增生产 Node/Bun 入口。

根 `deploy/` 缺失（嵌套工单 deploy 存在）；真实配置测试会在导入 render 时失败。Vite dev 插件还会对默认缺失的版本目录执行 realpathSync，需显式提供自有 `MICRO_APPS_DEV_DIR` 才能进行后续受控 dev/代理验证。部署恢复归 Phase 3，CI 分层及受管环境归 Phase 4，监控/恢复归 Phase 30/31。不存在的 `.zscripts` 与历史 Python/Next/Bun 测试也已单独列明。

## 5. Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 干净安装可执行前端 lint、类型检查和选定无数据库契约测试；结果绑定源码 SHA 与工具版本 | PASS | 干净 npm ci 后三项均 exit 0；70/70、零失败/跳过；本文件记录完整 SHA、工具版本及工作树配置摘要 |
| Rust 检查入口与当前单 crate 形态一致，不引用不存在的 workspace | PASS | 根代理/原生命令均显式 manifest；check 实跑成功；test/doc/fmt/clippy 实跑结果及既有失败分别记录 |
| 清单覆盖工单、通知、OAuth、websocket 和 app-backend 示例；没有遗漏生产 Node 入口 | PASS | 工程基线逐组件列出入口/锁/协议/遗留运行时及退出阶段，并做只读交叉审阅 |

Phase 1 的目标是可执行入口与真实基线，上述三项必需验收全部满足。完整 Rust 格式/严格 Clippy、依赖审计、部署配置和环境验收仍有明确失败或未运行项；没有把它们记为通过，也没有宣称全仓质量门禁或架构迁移完成。

## 6. 可复核日志与回退

详细原始输出保存在 Git 忽略的 `target/phase-01/`：`frontend-lint-reproduction.log`、`frontend-npm-ci.log`、`frontend-lint.log`、`frontend-typecheck.log`、`frontend-test-contracts.log`、`frontend-tooling.log`、`frontend-npm-audit.json`、`root-{lint,typecheck,test-contracts}.log`、`rust-versions.log`、`cargo-{fmt,check,clippy,test,doc}.log`、对应 result.json、`cargo-fmt-files.log` 和 `rust-summary.json`。这些本地原始日志不会自动随 Git 交付；本文件已固化命令、版本、摘要、分类和关键脱敏结果，不依赖这些日志才能理解结论。

兼容与回退：未变更 API、SDK、业务 SQL、迁移、生产运行阶段或用户数据。回退时仅撤销本次工具声明、质量脚本、ESLint 开发依赖/锁、CI/Docker 构建版本和文档变更，并在 frontend 重新 npm ci；不要使用整仓 reset 覆盖执行前已有的 README/方案/台账。构建缓存和本地日志不属于部署产物。

## 7. Suggested Task Ledger Update（已按台账同步）

- Phase Status Update：Phase 1 `COMPLETED`；其余阶段保持 `NOT_STARTED`。
- Output Summary：工具版本声明、前端 lint/typecheck/70 项契约入口、单 crate Rust 检查入口、工程/运行/协议基线与本证据。
- Completed Items：本文件 Self-Check 的三项全部 PASS。
- Remaining Items：Phase 1 无剩余必需项；已知 Rust 格式/Clippy、前端 warning、旧依赖审计与部署缺失作为后续输入，不在本阶段伪标通过。
- New Decisions：固定本次可复现的 Node 24.18.0/npm 11.16.0/Rust 1.97.0；用可见 warning 冻结既有前端债务；必需契约与有环境条件的扩展测试分开记录。
- New Open Questions：没有阻塞 Phase 1 的新增业务/环境问题；原台账外部环境问题仍有效。
- Next Recommended Phase：Phase 2「将静态兼容示例和根前端入口迁为 Vite」，只更新指针，本次不执行。
