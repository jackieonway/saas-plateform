# Phase 3 执行与验证证据

日期：2026-10-15（Asia/Shanghai）。阶段：恢复 Nginx 部署基线并交付宿主静态镜像。结果：**COMPLETED**。仅执行 Phase 3；台账指针移至 Phase 4 NOT_STARTED，未自动进入后续阶段。

## 1. 执行边界与源码绑定

- 用户明确要求按台账执行 Phase 3；前置 Phase 1、2 已 COMPLETED。用户确认可使用 WSL，本次只在隔离 Linux Docker 环境验收。
- Git HEAD 为 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，分支 main。保留之前阶段及用户全部未提交内容，本阶段未创建提交、推送镜像或发布生产服务。HEAD 不是本次实际工作树内容摘要；本阶段 49 项输入清单见 [phase-03-inputs.sha256](phase-03-inputs.sha256)，其 SHA-256 为 `216c2d1fe1f3b52f9f0fbc0d3e63fe356a1758fc0de5957a9f43514de8b763cf`，结合 Phase 1/2 输入清单及原生 289 项完整快照绑定实际源码。清单按 Windows 工作树字节生成，不包含自身、台账或本证据正文，避免循环摘要；Git 换行转换可能改变字节摘要。
- 不迁移 Node 业务逻辑，不执行 Phase 4 或数据库/业务迁移，不启动真实平台、通知、工单或业务数据库。独立 Rust crate 只用于传输验证，不能证明 Socket.IO 事件兼容、平台身份/授权或 Phase 22/23/32 已完成。
- Windows 构建工具：Node **24.18.0**、npm **11.16.0**、Cargo/Rust **1.97.0**。宿主 Vite 保持锁定版本，不顺带升级依赖。WSL Ubuntu：Docker client/server **29.1.3**、Linux/amd64、内核 `6.18.33.2-microsoft-standard-WSL2`；显式使用本地 `unix:///var/run/docker.sock`。
- WSL 原来没有 Compose 插件。官方固定 **v2.39.4** 经 Windows 正常 HTTPS 下载、WSL 官方 SHA-256 校验后装到用户 `/home/jackie/.docker/cli-plugins/docker-compose`；未改变系统配置。二进制 SHA-256：`7af95166a730b87e172d4fc9aefea8725d3c6c7327d59149267b452114ddb7d4`。来源/校验日志：`target/phase-03/compose-plugin-installed.log`。

## 2. 实现与交付

- `frontend/Dockerfile`：干净锁定安装、多阶段构建；运行阶段 Nginx 1.28.0，UID/GID 101，端口 8080，只写 `/tmp`，直接入口绕过修改只读配置的脚本。Node/npm 仅在构建阶段。
- `deploy/gateway`：独立 TLS 入口、精确 HTTPS origin/服务白名单、可信私网单 IP、覆盖伪造 forwarding headers、Docker DNS 五秒重解析；API/OAuth/业务前缀、body/query/Auth/Origin 保留，polling 与 Upgrade 直达 Rust。业务 CORS 由上游负责。无 Caddy 配置或兼容模式。
- 宿主只对已知导航、GET/HEAD 和明确 HTML Accept 提供 SPA fallback；API、Vite、缺失资源及未知微应用版本严格 404。HTML no-cache、hash assets immutable、错误 no-store；入口拥有单一宿主 CSP。`imageOrigins` 是精确 HTTPS 品牌图片白名单，只扩展 img-src。默认无 eval；显式 host trusted 配置不放宽 opaque 版本 HTML。
- 默认 Compose 使用固定镜像、外部预建私网 edge、数据库 internal network，只有 gateway 发布 80/443；实际 gateway IP 同时用于平台 TRUSTED_PROXY_IPS。默认没有 MICRO_APPS_DIR 挂载。ZIP override/profile 独立平台 RW 与 micro-static RO、create_host_path:false；入口/宿主均不挂版本目录。
- `compose.legacy.yml` 仅显式启用已有 Node 通知/工单，私网无发布端口、不启动业务数据库、独立密钥/数据库 URL；不算 Rust 迁移。根 Caddyfile、Caddy 卷及旧 Caddy 网关测试已退出。
- `deploy/static`：实际源内容/buildId/资产摘要 metadata；currentAssets 记录合并前当前产物，assets 记录交付全集。保持 schema 1 及旧 metadata 导出兼容；从固定 registry digest 或完整本地 image ID 导出并核对历史 assets，构建时拒绝同 URL 不同内容，再合入新镜像。更新和回退均从当前在线镜像导入；没有运行时共享产物卷或自动删除旧文件。
- `deploy/native/build.mjs`：全新白名单快照、四工程锁定安装及构建、Rust release 编译、源码/命令/工具/产物 manifest；失败不发布 artifacts。精确公开模板/.npmrc 例外经校验，其余私有 env/npmrc/证书/缓存不读不复制。不打包 Node 业务后台。
- CI 仅修正本阶段缺失构建消费者、缓存锁清单和真实配置/构建器 gate；完整分层 CI、监控、加密恢复分别归 Phase 4、30、31。本次未运行 GitHub Actions。

## 3. 实际执行结果

各组测试存在重叠，不能相加为独立总数。`PASS` 为实际成功，`FAIL` 保留失败原因，`NOT_RUN` 不算通过。

| 实际命令/检查 | 分类 | 结果/证据 |
| --- | --- | --- |
| `node --test tests/nginx-render-policy.mjs tests/production-native-config.mjs deploy/static/tests/static-image-policy.mjs deploy/native/tests/build.test.mjs` | PASS | **38/38，0 fail/skip/cancel**。含 8 gateway、3 native、1 实际 Compose、16 static、10 builder；日志 `target/phase-03/deployment-policy-tests.log` |
| 实际 `docker compose config --format json` | PASS | 默认/ZIP/legacy 均解析；缺必需值拒绝。断言只有入口端口、默认无共享目录、ZIP RW/RO持久路径与私网/trusted IP，未启动数据库。包含在上行，不能重复计数 |
| `npm run test:contracts` | PASS | **70/70，0 fail/skip/cancel**；`target/phase-03/contracts.log` |
| Rust fixture `cargo test --locked --manifest-path tests/fixtures/gateway-rust/Cargo.toml` | PASS | **8/8，0 ignored/skip**，独立 crate/锁；`target/phase-03/rust-fixture-test.log` |
| Rust fixture fmt/check(all-targets)/严格 Clippy/doc/build | PASS | 独立 crate 全部通过；对应 `rust-fixture-{fmt,check,clippy,doc,build}.log` |
| Windows fixture 实进程 HTTP/WebSocket smoke | FAIL（环境） | 127.0.0.1 已监听但 Node/PowerShell 均连接超时；未充当通过，进程已清理。后续 Linux Docker 验收独立记录；`rust-fixture-smoke.log` |
| 原生真实干净 release 构建 r2 | PASS | 四工程 npm ci/build、finalizer、Rust release 成功，283 输入/136产物全部摘要匹配。Windows x64，不算 Linux 容器通过；最终模板 r3 结果另列 |
| `node deploy/native/build.mjs --out-dir <workspace>/target/phase-03/native-release-20261015-r4 --cargo cargo` | PASS | 最终源码真实构建 **SUCCEEDED**，289 输入/141产物逐字节摘要一致，currentAssets/合并 assets 均 97，0历史；Windows x64，buildId `32956c4532bb-cdc5bd8779d36262`。包括全部部署/环境模板和最终 currentAssets 实现，当前源与快照无差异。`native-r4-audit.json` |
| 实际 Docker 干净宿主构建（第一轮） | PASS | `target/phase03-1792036045074-12cba0/first-build.log`；完整镜像身份见该运行报告。后续 Rust 基础镜像未导入导致整组 FAIL，没有把镜像 build 当联合运行通过 |
| `PHASE03_BUILD_PROXY=<已验证的本机loopback代理> node --test tests/micro-release-gateways.mjs` | PASS / 0 | **8/8（父测试+7验收组），0 fail/skip/cancel**，288791 ms；最终 `target/phase03-1792036915997-705006/result.json` 为 PASS，清理无错误。正常验证 TLS，非 Node 服务端替身 |
| `git -c safe.directory=<当前仓库> diff --check` | PASS | 仅有已有 CRLF/LF 提示，未发现空白错误 |
| 平台 Rust 全量 fmt/Clippy/DB/生产联合/恢复、浏览器完整业务流程、GitHub runner | NOT_RUN | 平台业务 Rust/数据库未改，不借用 Phase 1/2 或历史通过。Phase 1 的既有 fmt/Clippy、52 ignored、宿主 lint 64 warnings 和旧管理 UI fixture 失败仍按原证据保留 |

最终 Linux 实测摘要已持久保存于 [phase-03-runtime.json](phase-03-runtime.json)，包含完整本地 image ID、版本、七组结果及原始报告摘要；原生逐文件审计摘要保存于 [phase-03-native.json](phase-03-native.json)。本地 image ID 是当前 daemon 中的不可变对象身份，未伪称已经取得可跨机器拉取的发行 registry digest。

| 实际验收组 | 结果 | 关键实证 |
| --- | --- | --- |
| 非 root/只读与私网 | PASS | UID 101；实际 nginx -t；只写 /tmp；宿主静态根写入拒绝；无 node/npm/bun/caddy；只有入口发布端口 |
| 正常 HTTPS 与代理 | PASS | 专用 CA + SNI/主机名正常验证；HTTP 308 保留路径/query；三 API 前缀 POST 原字节/query/Auth/Origin 保留，伪造 forwarding 被覆盖；合法/null CORS 单一，非法 origin403无授权头 |
| Rust polling/Upgrade | PASS | 实际 polling GET/POST200；WSS101、metadata 首帧及文本 echo。仅传输层，不是完整 Socket.IO |
| 静态/导航/版本 | PASS | 正确 HTML/JS MIME、HTML no-cache、hash immutable、单一宿主 CSP；缺资源/API私有/Vite/未知版本404，HTML Accept q=0无fallback。镜像版本路由只声明 URI 映射验证，可嵌入静态 CSP 由下一组覆盖 |
| 换 IP 自动恢复 | PASS | Rust 由 fixture-a/172.18.0.3 重建为 fixture-b/172.18.0.6；旧 IP 被占住防止复用，入口依 Docker DNS重新解析，未手工reload |
| ZIP 持久化/嵌入 | PASS | HTML单一CSP允许宿主frame-ancestors、无XFO且no-cache；mjs/css MIME正确、JS单一immutable/匿名ACAO*；staging/missing404单一no-store；RO写入拒绝，重建static后字节保留 |
| 旧入口资源保留 | PASS | 新入口 `/assets/index-CtIJinRL.js`，旧入口 `/assets/index-BS_ZdeWt.js` 已不在新currentAssets、仍在合并assets；旧URL200/immutable且SHA256=`0a975ff4254d849aa578253fb0eb7d6af61a138a1aa5f950dbe3ac60253c5e60`，宿主Mounts=0 |

最终旧/新宿主本地镜像 ID 分别为 `sha256:084557b0b25843641454c79524b481c3a8c0b57c6b8d6d0d71404cb4cc8cd7c8` / `sha256:f1d1d054f11009bc25ebc6789e01a923faf2a2ec77b3327e5fe8b30172abbb48`。源码内容 SHA-256 由 `6aeb812f9175a06b7e9f91046bfb51def6cfc58c7d66f9f1fcfc7db942e67e90` 变为 `72c45bba0ec789810a40d9a81d570d2a0b8d297351f628a72f8e45d36b646306`。下一镜像实际重新执行构建及历史合并，不靠共享目录或只保存旧镜像。

最终运行于 `2026-10-15T04:06:44.547Z` 完成。容器/网络/卷创建前登记、删除前检查精确 owner label，清理错误会使测试失败。最终 cleanupErrors 为空；父任务另行查询所有本次 `saas.phase03.owner` 标签的容器、网络、卷，结果均为空。保留自有镜像及构建日志用于审计，未删除无关 Docker 资源。

## 4. 真实失败、环境恢复与独立审阅

首轮原生构建实际失败于源码筛选误排除 `frontend/src/components/micro/runtime.ts`；已收窄规则并增加回归，旧 FAILED 输出保留于 `target/phase-03/native-release-20261015`，未覆盖或伪标成功。构建器另拒绝含凭据 URL 的 npm 锁、非公开 .npmrc、已有输出及环境秘密继承；受控编译器测试只证明控制流，真实 release 单独执行。

最终原生构建实际输入摘要为 `cdc5bd8779d3626265392d9c7d88eac038cafb9509c596b1e2ff523c5fd5ccd9`；source-manifest 文件 SHA-256 为 `8cbb822cd663a834732981532d8cae652b99765553cc7e9e953568c66d01f070`，build-result 文件 SHA-256 为 `ac716b3928ee1f2daf94be82a97e820f8e4ce304c011dd84fee0e59a3e32ac64`。源码快照和交付目录分别记录，部署只选择成功 artifacts 与 manifest，不使用 work 缓存。r3 也曾成功，随后为 currentAssets 证明增强重新执行 r4，未修改旧快照/manifest。

WSL 直接访问官方 registry/GitHub 的 fake-IP 路由超时，未关闭 TLS 或修改 Docker daemon。通过 task-local 官方 `google/go-containerregistry crane v0.22.1`（官方 checksum 核验）在 Windows 正常 HTTPS 拉取官方基础镜像 archive，再用本地 Docker load。registry manifest digest 与重建 archive 摘要分别记录于 `target/phase-03/images/base-image-sources.json`，二者不能混为同一发行 digest。构建依赖出网使用已存在的本机代理，仅通过 Docker 预定义 proxy build args 和构建期 host network；运行时仍为隔离私网，不持久化代理环境、凭据或 Docker socket。

基础镜像来源和摘要的持久证据见 [phase-03-base-images.json](phase-03-base-images.json)。联合测试第二轮发现 Docker 29 的 internal network 不发布端口，setup FAIL 已保存于 `target/phase03-1792036657330-016a8b/result.json`，自有资源已清理；改为仅入口接入单独 bridge，同时保留 internal 上游网络，再完整重跑。生产 Compose 本来使用外部 bridge，不需改变生产网络配置。

独立代码审阅发现并修复：Compose fixture 仍引用旧服务/变量，以及新 CSP 无配置出口导致既有外部 HTTPS Logo 被拦截。最终测试覆盖新 default/ZIP/legacy 边界、精确 imageOrigins、拒绝通配 origin；未通过删除断言、放宽全局 CORS/CSP 或编造 Node 生产 stub 消除失败。

运行驱动的独立审阅另补强微应用真实响应头/重复缓存检查、从真实 index 选择已退出当前构建的旧入口，以及失败创建/清理不能漏报。最终只读复核确认三项均闭环，没有剩余必须修复项。

## 5. 兼容、回退与验收边界

部署方仍须提供实际域名/证书/registry/digest/CPU/主机/网络、受限服务凭据与数据库升级条件。本次隔离 CA 只用于测试；正常信任/主机名验证，未导入系统信任。未以示例 `.invalid` 值虚构生产配置，没有外部推送或现网部署。

镜像、ZIP 和外部 URL 的完整业务发行、ZIP 原子发布、Socket.IO 协议选型仍跟随后续阶段。保存镜像 tag/digest 不能代替旧 hash URL 在线；回退须重建包含当前 assets 的候选。该保留策略暂不自动清理，容量/正式保留窗口归 Phase 29。

## 6. Self-Check 与台账更新

| 台账验收标准 | 结果 | 证据 |
| --- | --- | --- |
| 干净源码构建、非 root/只读 Nginx，无 Node/Bun/Caddy 运行阶段 | PASS | 实际 Linux build/run/nginx -t/UID/禁止写入/二进制缺席；原生构建另列 |
| 独立入口、私网静态/Rust、换 IP 恢复，仅入口公开 | PASS | 自有 ingress bridge + internal 上游网络，实际响应与不同 IP/instanceId 恢复；生产 Compose 另经真实 parser检查 |
| 实际 HTTPS、API/polling/Upgrade、CORS/CSP/MIME/缓存/严格404 | PASS | 上述七组实际响应与 TLS/WebSocket 帧断言，无关闭证书验证 |
| 默认无 MICRO_APPS_DIR 挂载、ZIP重建持久化、旧 chunk 在线 | PASS | 实际 RO持久卷重建及无卷宿主；retained-only旧入口真实字节/HTTP200 |
| 环境缺口不能伪标通过 | PASS | 初始失败与恢复单列；最终 Docker/Compose/HTTPS 必需检查均实际执行，零skip；生产/完整协议/DB等明确未验收 |

建议台账更新（已按台账全局要求同步）：Phase 3 **COMPLETED**，详情/汇总/决策/变更记录同步；当前 Phase 4 **NOT_STARTED**。本阶段无剩余必需项、无新增阻塞问题。下一阶段为“建立项目受管的分层 CI 与隔离测试环境”，需新的阶段执行指令。实际生产参数及正式保留政策仍是既有开放问题，不影响本次隔离验收，也未被填成虚构值。
