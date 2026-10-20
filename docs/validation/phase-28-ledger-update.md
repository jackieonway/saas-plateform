# Phase 28 台账更新记录

拟同步汇总、详情五项验收、版本 v0.39、Decision／Change Log 与下一执行指针。正式状态由审核后的具体台账差异及 final-verification.json 确认。

## 来源与执行范围

完成日期：2026-10-19（Asia/Shanghai）。用户在本聊天明确要求“验收并完成”；仅完成 Phase 28，没有执行 Phase 29 或生产操作。原聊天的 413 终止是交接背景，不作为验收成功证据。

sourceRevision：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`。实际工作树含未提交变更；由 839 项 manifest 与输入 SHA256 `f77df330f956da2d9bcb5ec96a5ae6fe617dc752f15267be565c3033d3be05c5` 标识，不能只按 Git HEAD 声称版本一致。冻结目录为 `target/phase-28/source-candidate-r1`；正式 fast、六镜像与三个浏览器均来自这份冻结输入，执行前后及宿主输入一致。与阶段起点相比 50 路径变更，119 份历史正式证据及其他 36 阶段保持。

## 实际验证

| 验证 | 实际结果 |
|---|---|
| 完整 fast | 63 步；1083 PASS、0 FAIL、0 SKIP；filtered 1931 单列 |
| 真实镜像交付 | 14/14 PASS，四个静态镜像、两个 Rust API 镜像 |
| chromium | 9 PASS、0 FAIL、0 SKIP；156.0.8078.4 |
| chrome | 9 PASS、0 FAIL、0 SKIP；155.0.8059.84 |
| msedge | 9 PASS、0 FAIL、0 SKIP；155.0.4283.62 |
| 隔离资源及专属 cache 清理 | PASS，无遗留本轮容器／网络／临时卷；cache owner 检查后删除 |

fast 的 TAP／Cargo parent 与 TLS 具名检查采用各自解析器；filtered 不算执行通过，也不把命令成功计作额外测试。详见 phase-28-runtime.json 的 requiredRuns/countsByFormat、steps 和独立日志。三浏览器各 9 个具名业务检查，不与 fast parent 数混计；共用一次明确记录的镜像 push／purge／pull 及 Compose 拓扑。

## 实现与拒绝层

外部部署 UI 只向既有 RegisterBody 发送六个允许字段，得到 DRAFT 后单独预检，正文 PASSED 才激活；HTTP 200 + FAILED 拒绝，激活重新检查入口。平台没有 Docker socket，不拉镜像、不启停容器、不生成路由、不保存伪造 digest metadata，也不自动授权。

Rust CLI／CI 的权威持久 inventory 原子绑定 app、公开版本 URL、来源及 digest，同版本换来源／digest、并发冲突和跨环境同 artifact 均用实际 CLI 验证。inventory 是可信本地持久声明；不声称为跨机器分布式 CAS，也不代替 registry 字节或业务授权。未知写先对账，不盲目重发。

外部驱动先读取真实 Docker health，再绑定 inventory、检查并推进 Nginx 版本路由，最后登记／预检／激活。负向健康检查实际拒绝并检查路由、登记、激活均未推进；平台只检查公开入口／资源／healthUrl，不读取 Docker health。v1／v2、动态 chunk、旧资源保留、未知版本 404 和前端回退均实证。

同域 v1 使用相对业务 API base；批准外部 origin 的 v2 使用明确绝对 HTTPS API base。opaque iframe 请求 Origin:null、应用 Bearer、omit Cookie，TLS 主机名／证书正常验证，CORS／CSP、普通用户工单／门店 Rust CRUD 与独立三库实际通过。

## 命令与证据入口

下列 argv 由实际报告提取；Node 可执行文件为 D:/Software/Scoop/apps/nodejs/24.18.0/node.exe，Linux 工具容器为 Node 24.18.0／npm 11.16.0／Rust 1.97.0。

镜像交付命令还显式设置 BUSINESS_DELIVERY_HOST_SOURCE_ROOT=D:/WorkSpace/rust/saas/rust-saas 与 BUSINESS_DELIVERY_CACHE_FILE=D:/WorkSpace/rust/saas/rust-saas/target/phase-28/validation-runner/cache-owner.json；宿主与冻结源码分别核对，两个正式 gate 共用本阶段已声明的专属缓存。

```json
[
  [
    "D:\\Software\\Scoop\\apps\\nodejs\\24.18.0\\node.exe",
    "D:\\WorkSpace\\rust\\saas\\rust-saas\\target\\phase-28\\validation-runner\\run.mjs",
    "--source-root",
    "D:/WorkSpace/rust/saas/rust-saas/target/phase-28/source-candidate-r1",
    "--source-revision",
    "32956c4532bbd6eff53584fd0f5e9bb6e188437c",
    "--inputs-sha256",
    "f77df330f956da2d9bcb5ec96a5ae6fe617dc752f15267be565c3033d3be05c5",
    "--out-dir",
    "D:/WorkSpace/rust/saas/rust-saas/target/phase-28/final-fast-r1",
    "--gate",
    "fast",
    "--candidate-file",
    "D:/WorkSpace/rust/saas/rust-saas/target/phase-28/inputs-candidate-r1.json"
  ],
  [
    "D:\\Software\\Scoop\\apps\\nodejs\\24.18.0\\node.exe",
    "D:\\WorkSpace\\rust\\saas\\rust-saas\\target\\phase-28\\source-candidate-r1\\scripts\\ci\\business-delivery.mjs",
    "--out-dir",
    "D:/WorkSpace/rust/saas/rust-saas/target/phase-28/final-business-delivery-r2",
    "--source-revision",
    "32956c4532bbd6eff53584fd0f5e9bb6e188437c",
    "--platform",
    "linux/amd64",
    "--build-proxy",
    "http://127.0.0.1:7897"
  ],
  [
    "node",
    "target/phase-28/capture-source.mjs",
    "capture",
    "r1"
  ],
  [
    "node",
    "target/phase-28/capture-source.mjs",
    "boundary",
    "r1",
    "target/phase-28/boundaries-r1.json"
  ],
  [
    "node",
    "target/phase-28/validation-runner/cleanup-cache.mjs",
    "--out-dir",
    "D:\\WorkSpace\\rust\\saas\\rust-saas\\target\\phase-28\\final-cache-cleanup-r1"
  ]
]
```

严格收集器的八个显式参数见 final-audit-r1.json 的 collectionOptions；正式发布前再次收集并逐字节复核原始日志、ELF、OCI archive、截图、静态资源与清理证明。所有公开证据路径／长度／SHA256 列在 phase-28-runtime.json。私钥、runtime policy、机密客户端、Bearer 和 DB URL 不归档；该索引不包含 private 工作目录。

## Self-Check

| Acceptance Criterion | Result | Notes |
|---|---|---|
| 两应用可独立构建镜像，Linux隔离环境拉取启动；纯镜像无需平台产物卷，入口Nginx按版本代理。 | PASS | independent-static-image-builds、isolated-registry-push-pull、fixed-digest-compose-start；正式 delivery 原始报告和三个浏览器 fixture／原始日志。 |
| URL登记DRAFT→预检PASSED→激活，HTTP200+FAILED仍拒绝；界面明确外部部署与平台管理的边界。 | PASS | url-ui-draft-preflight-activate、preflight-http200-failed-rejected；正式 delivery 原始报告和三个浏览器 fixture／原始日志。 |
| v1/v2、动态chunk与回滚真实可用；持久清单拒绝同公开版本URL换来源/digest，记录实际拒绝层。 | PASS | versioned-chunks-rollback-retained、artifact-inventory-conflicts；正式 delivery 原始报告和三个浏览器 fixture／原始日志。 |
| CI/Compose检测容器不健康时不推进路由/登记/激活；平台拒绝入口/资源或已配置healthUrl的预检失败，分别记录拒绝层，不虚构平台可读Docker health。同域/批准外部origin的API解析与SDK授权实际验证。 | PASS | unhealthy-blocks-promotion、preflight-http200-failed-rejected、activate-rechecks-unavailable-entry、external-origin-api-cors-csp、ordinary-user-sdk-rust-authorization；正式 delivery 原始报告和三个浏览器 fixture／原始日志。 |
| 镜像部署不自动授权，不虚构平台Docker控制；镜像、RustAPI、数据库的版本分别可追踪。 | PASS | image-deployment-does-not-grant、independent-databases-and-services；正式 delivery 原始报告和三个浏览器 fixture／原始日志。 |

## 兼容、回退与限制

镜像是推荐交付，ZIP 保持独立兼容链，Phase 34 才做两链总验收。平台 active／previous 只切前端版本指针，保留两版本容器及完整旧 chunk 路由；Rust API 和数据库版本各自跟踪，前端回退不隐含 API／数据库回滚。业务功能权限和 data grant 保持独立，镜像部署不自动产生授权。

本轮 registry 为 loopback 隔离 HTTP，仅本阶段演练；业务 HTTPS 入口和三个浏览器保持正常 TLS。清除本轮候选镜像引用／ID并证明不存在后，使用固定 digest 重新 pull；同 daemon 的基础层缓存可能仍在，未宣称第二台机器或空 daemon。两个 Rust API 的本次执行路径为冻结源码独立 locked Cargo 编译、核验 ELF／SHA／架构，再用 runtime Dockerfile 装箱；默认 API Dockerfile Cargo builder 未运行，不冒称通过。

远端 GitHub Actions／受保护环境设置、真实外部 registry 推送、目标服务器部署及生产切流均未执行，均不是本阶段隔离验收的成功项。构建／镜像、容器健康、公开 URL 预检、激活及普通用户业务验收分别记录。


## 归档工具兼容与额外回归

证据收集器仅在 target/phase-28 内作兼容修正，没有更改冻结运行源码、原始退出码或原始日志。start.json 的 authorization 仅对已核验的固定范围说明句子作精确值识别，其他凭据仍拒绝；TLS 原日志的跨行脱敏合并按冻结 scripts/ci/report.mjs 的原函数精确重现后比较，九项独立 JSON 检查、TAP 退出和计数仍全部要求通过。

镜像不存在的预期拒绝仅接受由已核验 owner/tag/manifest 推导的本轮引用；pg_isready 的 exit 2/no response 仅在同一 owned 命令后续确实 exit 0/accepting connections 时记为启动过渡，原非零退出保留在 expectedRejections，不伪造为命令成功。上述工具边界回归分别为 8、5、8 项，共 21 项通过，独立列为观察记录，不加进 1083 个正式 fast 测试。

可选实时启动观察器曾在 inspect 后遇到容器受控重建/清理，读取日志时返回 No such container 并退出 1；不把该工具标成测试通过。platform-startup-observation-r2.json 明确从原始保存的 Docker inspect 日志只读整理状态，列明原工具退出和方法，包含三个产品真实 healthy 记录。首轮正式失败只证明 Chrome 平台当时未就绪、进程仍在运行且无 OOM；未断言具体根因。第二轮维持同一探针和拒绝门禁通过全部验收。

## 原始失败与清理

开发 services/build 和镜像交付 r1–r9 连同首轮正式失败和启动观察及收集器边界回归共 18 项记录由 development-index-r5.json 留存。早期 CLI clippy、静态构建 apiOrigin、OCI archive 身份、镜像引用、Docker 及健康问题的失败结果未改写；后续修复由新运行证明，r9 成功仍只作开发记录。首轮正式交付尝试 final-business-delivery-r1 在 Chrome 平台启动就绪时失败且完整保留；正式结论仅采用最终冻结输入的 final-fast-r1 与独立重跑的 final-business-delivery-r2。

本轮容器、网络、临时卷和前台 stdin／WSL lease 均退出并空查；跨 gate 保留的专属 Cargo／npm cache 通过 owner label 和无挂载容器检查后单独删除，证明见 final-cache-cleanup-r1/result.json。未清理其他阶段或非本轮资源。

## Suggested Task Ledger Update

Phase 28: COMPLETED。所有五条原验收 PASS。Phase 29: NOT_STARTED；下一阶段须新的明确请求。历史证据、其他 36 阶段和全局 cache 字段保持。
