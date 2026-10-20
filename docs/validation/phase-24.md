# Phase Execution Result — Phase 24

状态：**COMPLETED**。2026-10-16（Asia/Shanghai）。最终真实平台运行是 `linux-r6`，最终独立镜像运行是 `delivery-r5`；前轮失败和中止分别保留。按照用户补充要求，只标记 Phase 24 完成，整个 Current Execution Pointer 区段保持本次台账写入前的内容。

## Executed Phase

Phase 24：拆分 OAuth 演练为 Vite 页面和 Rust 流程 API。依赖 Phase 2、4、6、11 已完成。本任务只实施 OAuth 演练迁移，保留工作区原有和其他并行任务的变更；没有执行其他阶段、镜像推送或生产部署。

## Execution Summary

旧 `mini-services/oauth-playground/index.ts` Node HTTP 服务及 `pages.ts` 内嵌页面已退出运行入口。新增独立 Vite MPA 页面、真实 callback HTML 和独立 Rust/Axum API，开发端口为 3010/3011。客户端 secret、PKCE verifier、设备码、access/refresh token 留在 Rust 的有界会话中，浏览器只接收公开状态与摘要。

服务端 state/verifier 绑定发起会话、按 TTL 校验并一次性消费；错误会话、错误 state、重放、过期、未知交换结果均不能恢复旧凭据。每流程通过带 operation owner 的占位串行处理签发、刷新和吊销，超时后释放本次占位并保持旧凭据失效。请求要求精确 Origin、严格 JSON 与 HttpOnly 会话；生产 Cookie 使用 Secure、SameSite=Lax、`__Host-` 前缀。HTTP 并发、请求体、上游响应体、会话/state 数量和期限均有限额。

固定上游 origin/path、正常 rustls 证书校验、禁止 HTTP 重定向和隐式代理；错误只返回固定类别。未配置流程明确拒绝，password 默认关闭，public 设备客户端不使用其他机密客户端的内省权限。普通 OAuth token 对应用专属菜单/角色返回真实 403，额外使用平台正式 app-bound token 验证该接口正路径，没有修改平台授权规则。

## Deliverables

- [Rust 工程与配置/API 说明](../../mini-services/oauth-playground/backend/README.md)、独立 Cargo.lock、薄入口、窄 `Config/app` 接口及 17 项配置/状态/HTTP 回归。
- [Vite 工程](../../mini-services/oauth-playground/frontend/README.md)：四类流程、设备轮询、公开摘要与资源检查、callback 清理 URL；8 项前端测试。公开配置使用单独白名单解析，保留 `flows.password.enabled`，资源凭据过滤仍生效。
- [独立部署说明](../../mini-services/oauth-playground/README.md)、两个 [Dockerfile](../../mini-services/oauth-playground/deploy/Dockerfile.api)、[私网 Compose](../../mini-services/oauth-playground/deploy/compose.yml)、[HTTPS renderer](../../mini-services/oauth-playground/deploy/render-edge.mjs) 和非 root 静态 Nginx；运行私有配置不进入前端构建。
- [真实平台 fixture](../../backend/src/oauth_playground_ui_tests.rs)、[浏览器闭环](../../tests/phase24-oauth-browser.mjs)、[受管流程入口](../../scripts/ci/oauth-playground.mjs)、[受管 Docker HTTPS 验证](../../scripts/ci/oauth-playground-delivery.mjs)。
- 根 `dev:oauth` / `--with-oauth` 启动 Rust API 与 Vite，支持独立端口、IPv6、readiness、既有受控退出与脱敏；CI fast 安装/构建 OAuth 前端，运行前端、部署和 Rust 单元测试，并纳入公开源码快照。
- 平台仅新增测试 dev-dependency 与 `cfg(test)` fixture。Cargo 仍会解析该 path dependency，因此同步窄修平台 Docker 构建上下文及原生源码快照；依赖在构建阶段解析，平台运行层仍只有平台 binary。原生回归使用真实离线 Cargo 验证缺失路径必然失败。
- [最终相关输入](phase-24-inputs.json)、[运行/镜像/数据库证据](phase-24-runtime.json)、[台账更新记录](phase-24-ledger-update.md)。根 README、现状架构、工程基线及组件 README 已同步。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| PKCE/设备码/受控服务凭据与历史兼容演练按声明矩阵通过真实平台；不支持项明确拒绝。 | PASS | Chromium `156.0.8078.4` + 真实 Vite dist → Rust 演练 API → 真实平台 Axum/隔离 PG，12 项检查全通过。数据库有 1 次 consent、1 次 device 审批、4 类客户端实际签发；授权码及 password 保留令牌对已吊销，原始 JWT 持久化数为 0。public 设备内省、CC refresh/userinfo、普通 OAuth external 越权均明确拒绝。 |
| 错误 state、重放、过期、错误发起会话均拒绝；dist/URL/日志无 secret 或 verifier 泄漏。 | PASS | 真实浏览器验证 state/跨会话/重放、同源约束、verifier 接口关闭、URL/存储/响应；可控时钟的状态测试验证 TTL 边界；真实 HTTP fake-upstream 验证一次交换、并发刷新/吊销、重定向拒绝、容量/响应体上限、超时和脱敏。HTTPS 日志扫描及最终产物检查通过。 |
| Vite 独立构建，Rust 独立运行，端口/HTTPS/callback 与文档一致。 | PASS | 独立 npm ci/build、Rust 严格检查、两个实际 Linux 镜像及 10 项 HTTPS 检查通过。固定 callback、S256、Secure Cookie、实际首页/callback/3 个 hash 资源、404/no-store、UID 10001/101、只读/私网与 Compose 端口均验证。 |

## Source and Tools

- Git HEAD：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`；存在前阶段及并行阶段未提交内容，HEAD 不代替实际输入摘要。
- 最终 Phase 24 相关输入：`03a5eea5e0628d0a7135fcb23f656fb64594e76341faaaaafd2530bbd01db9f7`。
- 最终 OAuth 组件输入：`2efc48b954ad0815b0f3aae4aec03ef1426778ddceff681f987d4b0b17718d85`，已逐文件核对当前源码、真实平台快照与镜像快照一致。
- `linux-r6` 全快照：`890ab23d313e21b71d38d63b0481638ea3b833225d06ef91032dc7e6007b1888`。其他并行阶段被保留，其能力没有计入本阶段验收。
- Node `24.18.0`、npm `11.16.0`、Rust/Cargo `1.97.0`、Vite `8.3.4`、Playwright `1.64.1`、Chromium `156.0.8078.4`、Docker `29.1.3`、隔离 PostgreSQL `17.6`。
- Linux 工具镜像：`sha256:7ad323a47d6d5202da34beb0f315d24dd10102d9bebd0726260c189768d0094c`。Windows 命令明确使用 Scoop 的 Node 24，未使用默认 PATH 的旧 Node 16。
- 平台 Cargo.lock 对比 Phase 15 快照仅新增 `oauth-playground-api` 包记录及 dev-dependency，原有依赖版本不变；旧三份平台迁移摘要不变。

## Actual Validation Results

| 实际命令/范围 | 结果与证据 |
| --- | --- |
| `node target/phase-24/run-linux.mjs <fresh-output> chromium`，最终 `linux-r6` | **PASS，9 个步骤全部成功**；独立 npm ci/build、JS、Rust fmt/Clippy/test/doc、live。源快照前后不变；[summary](../../target/phase-24/linux-r6/reports/browser/summary.json)、[runner](../../target/phase-24/linux-r6/linux-runner-result.json)。该外部阶段 driver 留存于 target，不是全仓 fast/browser 门禁。 |
| Linux `node --test` 前端/部署/dev | **27/27 PASS、0 skip**：前端 8、部署 5、开发入口 14；[日志](../../target/phase-24/linux-r6/reports/browser/oauth-js.log)。含 POSIX 进程组回收测试，补足 Windows 平台条件 skip。 |
| 独立 Rust `cargo fmt ... --check`、`clippy --locked --all-targets --all-features -- -D warnings`、`test --locked --all-targets --all-features`、`doc --locked --no-deps` | **PASS**；3 项配置/状态、7 项 HTTP 安全、7 项上游 HTTP 契约，合计 **17/17、0 ignored**；[测试日志](../../target/phase-24/linux-r6/reports/browser/oauth-unit.log)，同目录保存 fmt/Clippy/doc 日志。 |
| `node scripts/ci/oauth-playground.mjs --browser chromium --out-dir <fresh-output> --source-revision <full-sha>` | **PASS，12 项浏览器检查 + 1 项 Rust fixture，零 skip**；真实 PG、审批/发行/撤销记录和清理均验证。[结构结果](../../target/phase-24/linux-r6/reports/browser/live/result.json)、[脱敏日志](../../target/phase-24/linux-r6/reports/browser/live/cargo.log)、[末态截图](../../target/phase-24/linux-r6/reports/browser/live/oauth-playground.png)。 |
| `node scripts/ci/oauth-playground-delivery.mjs --out-dir <fresh-output> --source-revision <full-sha>`，最终 `delivery-r5` | **PASS，10/10**，实际构建两镜像并用受信测试 CA 正常校验 HTTPS/SAN；没有 TLS 绕过。[结果/精确 argv](../../target/phase-24/delivery-r5/result.json)、[组件摘要](../../target/phase-24/delivery-r5/component-inputs.json)。 |
| Windows 独立 Rust fmt/check/strict Clippy/lib tests/security/doc | **PASS**；3+7 项非网络测试全通过。[记录](../../target/phase-24/windows/result.json)。新增上游网络测试在 Windows 本机裸 TCP loopback 也遭遇 OS 10060，最终以 Linux 7/7 实跑验收。 |
| 最终 Windows JS/CI/快照与前端/部署回归 | **43/43 PASS、0 skip**；[日志](../../target/phase-24/final-js-r2.log)。与 Linux 27 项有重叠，不相加宣称独立总数。 |
| `node --test deploy/native/tests/build.test.mjs` | **13/13 PASS、0 skip**；真实 Cargo 路径解析及缺 manifest/src/目录负向回归。[日志](../../target/phase-24/native-path-tests-r1.log)、[修复前真实失败](../../target/phase-24/platform-path-dependency-before-r1/failure.log)。 |
| 新平台 fixture rustfmt、阶段 diff whitespace | **PASS**；[fixture 格式](../../target/phase-24/platform-fixture-fmt.log)、[空白检查](../../target/phase-24/phase-whitespace.log)。 |
| 现有平台全目标严格 Clippy | **FAIL，既有 bin 15 / test-bin 38 条错误**；本轮日志没有新增 OAuth fixture/服务诊断。[日志](../../target/phase-24/platform-strict-clippy-r1.log)。新独立服务严格 Clippy 为 PASS；不修复或掩盖无关旧债务。 |

最终候选镜像（本地 image ID，无 registry 发行声明）：

| 镜像 | 最终 ID |
| --- | --- |
| Vite 静态前端 | `sha256:2dca7dfdc9bdbf940ab139fcf15e0924945edfd3b738f433b886b9879808925d` |
| Rust API | `sha256:3d66d8f3347556bf855d6bd2e15ba14a2693bf6d74c0ae3810c48647c6de5241` |

## Failure History and Repairs

- `linux-r1` 的前端构建/23 项 JS 通过，新增上游测试临时诊断代码未格式化，fmt FAIL，后续未执行。诊断代码已删除；最终全部 17 项 Rust 通过。
- `linux-r2` 在测试源尚未冻结时中止；最终独立审计发现 2 个容器、1 个网络残留，核对本轮 owner 标签后回收。初始审计 FAIL 与最终回收证据均保留。
- `linux-r3` 的 Rust 17/17、JS 25/25 通过，真实浏览器因跳转后 CDP 无法读取 auth-start 正文而 FAIL。`linux-r4` 的导航暂停观察方案未能解决该问题，受控停止旧容器（exit 143），随后使用测试内 fetch 观察器读取真实公开 metadata，再把原始 Response 返回页面；无平台 stub/响应替换，机械复现 20/20 后才进入真实复跑。
- `linux-r5` 真实通过 PKCE、设备、CC 等前 7 项，发现公开配置的 `flows.password` 被通用凭据过滤误删。先新增两项回归重现 6 PASS/2 FAIL，再按配置白名单修复；最终前端 8/8、`linux-r6` 四类真实流程全部通过。
- `delivery-r1` 两镜像构建通过，证书 fixture 在 Windows DrvFS 的 chmod 失败；修正测试权限处理。`delivery-r2` 实际发现 edge/upstream 重复 Cache-Control/Referrer，修正 Nginx header 来源。`delivery-r3` 把通用单级 API 路由的 GET 405 误要求 404，改用不存在多级路径验证真实 404。原始 FAIL 保留。
- `delivery-r4` 为公开配置修复前组件的完整 PASS；修复前端后重新构建并执行同一套验收，最终 `delivery-r5` PASS。没有把旧镜像 PASS 继承给新前端。

## Risks / Notes

- 真实平台/PG 闭环在显式允许的隔离 loopback HTTP 上执行；consent/device 批准走平台正式鉴权 API，浏览器操作实际演练页面和 callback，不声明完整宿主审批 UI 旅程。独立镜像 HTTPS 验证是另一套受控运行，验证入口证书、Cookie、路由、S256/callback 和日志边界，未连接未提供的生产平台域名。
- 仅验收本阶段所需 Chromium；其他浏览器、生产域名/证书/客户端/容量与外部 registry 不继承通过。`cargo audit`/`cargo machete` 本机工具不可用，未宣称依赖审计通过。
- 单副本内存状态在重启后丢失；多副本需要明确粘性路由，不能重放未知交换。password 仅为显式启用的历史演练；public PKCE authorization client 未声明支持，缺配置或不支持操作会拒绝。
- 回退时整组切换演练静态镜像/API，重新发起授权；不恢复旧公开 verifier 接口，不恢复硬编码秘密，不重用旧 code/state/refresh。平台没有新增生产迁移，不回滚旧 checksum 或删除数据。
- [最终资源审计](../../target/phase-24/final-resources.json)确认 6 轮 Linux 和 5 轮交付的受管容器/网络为空，额外 Cargo 缓存卷已移除，审计 WSL lease 退出 0；本地候选镜像保留。初始中止残留审计另存，不覆盖失败事实。测试 TLS 私钥、合成私有 env 仅位于 target 的 private 目录，不作为可上传证据。

## Suggested Task Ledger Update

见[完整结构块](phase-24-ledger-update.md)。Phase 24 标为 `COMPLETED`，摘要、三项验收和证据同步；不修改整个 Current Execution Pointer，不自动执行任何下一阶段。

台账写入核验：[指针逐字节保持证明](../../target/phase-24/ledger-pointer-verification.json)为 PASS；Phase24 状态已标记 COMPLETED。
