# 项目受管的 GitHub CI

[production-build.yml](workflows/production-build.yml) 使用 Ubuntu 24.04 和仓库声明的 Node 24.18.0、npm 11.16.0、Rust 1.97.0。安装入口来自各工程 lockfile；浏览器驱动与下载位置由 `tests/browser` 管理，不读取开发者个人缓存。

| 层 | 执行时机 | 必需行为 |
| --- | --- | --- |
| `fast` | PR、main push、手工 | 当前 manifest 的无数据库契约及代码检查，包含独立工单与门店 Rust crate 的 fmt/check/严格 Clippy/lib 单元/doc 和门店构建；必跑项失败或 skip 返回非零 |
| `postgres` | fast 成功后 | 从源码构建 `toolchain` 测试镜像并按批准 digest 拉 PostgreSQL；Linux runner 创建隔离库与临时容器，分别执行平台十二组当前 76 项（保留原 74 项并新增成员策略 2 项）、工单迁移升级 27 项、只读契约 13 项、写入契约 14 项、成员契约 12 项、提醒契约 12 项，以及只读/写入/成员/提醒联合各 1 项（分别 13/12/10/12 个命名检查）；门店 SQL/联合各 1 项（分别 8/16 个命名检查），通知联合保持独立，保存各自报告并按 owner 清理资源 |
| `browser` | fast 成功后 | 从源码构建浏览器工具镜像并准备批准的 PostgreSQL；Linux launcher 为每个 Chromium、Google Chrome、Microsoft Edge 产品创建自有私网/临时 PG。十套 fixture 后执行真实 grant UI 保存/撤销闭环，报告实际版本、16 个命名检查、4 条审计关联和清理；缺少任一必需证据即失败 |
| `delivery` | main push、手工，前三层全部成功后 | Docker 29.1.3 构建 Linux amd64 固定镜像，验证 OCI 摘要、真实 HTTPS 与容器重建恢复 |
| `business-delivery` | main push、手工，前三层全部成功后 | 独立构建工单/门店静态和 Rust API 候选，以受管隔离 registry、固定 digest Compose 与三个真实浏览器验证版本 URL、回退和业务授权；typed 报告缺项或清理失败直接拒绝 |
| `pr-gate` / `candidate-gate` | 所需层结束后始终执行 | 任一层 failure、cancelled、skipped 均失败，不能以跳过伪装成功 |

建议仓库分支保护使用 `Required PR gate` 作为必需状态检查。仓库设置属于托管平台配置，本次本地实施没有修改 GitHub 设置或实际触发远端 workflow。

每层上传脱敏 summary、源码 manifest 与日志。镜像候选额外上传 `release-result.json`、`source-manifest.json` 和三个已验收镜像 archive；不上传源码 scratch、临时 TLS 私钥或数据库目录。报告分别记录构建、smoke、registry publication 与目标部署状态。可在同一源码版本下通过 run ID/run attempt 区分多次执行。

工单数据库检查的 `work-orders-postgres`、`work-orders-read-postgres`、`work-orders-live-postgres` 各自上传 `postgres-result.json` 与脱敏 `.log`，不合并为原平台 74 项。其中只读契约使用真实业务 PostgreSQL 和显式平台 HTTP 协议夹具；联合检查则从实际平台路由签发应用 token，启动独立 Rust 工单二进制并访问单独业务库，证明 Basic 内省和应用 Bearer context、有效 scope 与实时失权。联合报告必须含全部命名检查、平台/业务两个数据库清理及独立进程退出证明。该 fixture 使用显式 loopback 开发 HTTP，生产 HTTPS 验证和生产部署不属于这份本地联合结果。

Phase18 新增 `work-orders-write-postgres` 和 `work-orders-write-live-postgres` 专属报告与日志上传；前者以真实业务 PG 和明确平台协议夹具验证创建幂等、事务及预算，后者通过真实平台登录/context/grant API 与独立 Rust 工单进程验证写入链。原 `work-orders-postgres` 数量扩为 21，但历史 Phase16 的 15 项记录保持原样。阶段定向执行若未选择独立通知联合，必须明确标记其范围，不能据此宣称完整 `postgres` gate 通过。

候选架构 `linux/amd64` 是当前隔离 runner 的实际验收对象。生产架构、域名、registry、目标主机及凭据仍由部署方明确配置；候选 workflow 不向外部 registry 发布、不连接生产主机、不持有 registry 或生产秘密。`business-delivery` 的推送/拉取只发生在本次自有隔离 registry，并单列演练结果。OCI manifest digest 和 archive SHA-256 不能冒充真实外部 registry RepoDigest。受控发行必须加载同一已验收产物，推送后记录真实 registry digest，Linux Compose 使用 `repository@sha256:...`，目标部署结果单独验收。

所有 PR（包括 fork）仅使用 `contents: read`，checkout 不保留 token；候选 workflow 没有 `pull_request_target`、`workflow_run`、外部登录或发布步骤。发行凭据只允许出现在独立 [business-publish.yml](workflows/business-publish.yml) 的固定受保护环境 `business-image-publish` 中，不得加入此检查 workflow。该 workflow 只接受 main 手工触发，先通过 GitHub API 验证同仓库 main 的成功候选 run、精确 attempt、业务层与最终 gate，再下载同 run 的精确不可变 artifact 并校验原始归档；发布状态不代替公开 URL 预检、激活或生产部署验收。环境审批和外部 registry 配置由仓库维护者提供，本地实现不修改托管设置或触发真实发行。

手工执行可显式启用 Firefox 与 WebKit 扩展检查；它们不替代三个必需浏览器。WebKit 检查只表示该浏览器引擎的结果，不表示 Safari 产品支持。扩展通过独立 `tests/browser/run.mjs` 运行十套 HTTP fixture，结果独立上传；不执行必需产品的真实 PG grant 闭环。

本地检查 workflow 的结构与边界：

```sh
npm ci --prefix scripts/ci --ignore-scripts
node --test tests/ci-workflow.mjs tests/phase28-ci-policy.mjs tests/business-images-policy.mjs
```

该测试解析实际 YAML、验证权限与依赖图，并执行最终聚合 gate 的失败/取消/skip 情形。它验证 workflow 策略，实际数据库、浏览器和容器验收仍由对应 runner 执行。

Phase 19 的 `work-orders-member-postgres` 独立执行 `member_contracts` 的 12 项真实业务 PostgreSQL/HTTP 回归；`work-orders-member-live-postgres` 选择 `phase19_work_orders_tests::real_platform_member_directory_and_assignment`，要求 1 项 Rust 结果、完整 10 个命名成员检查、双库删除/连接池关闭与独立进程停止。两者分别使用 `work-orders-member-isolated-postgres` / `work-orders-member-real-platform-postgres`，不能替代原迁移/读/写或 Phase17/18 联合回归。当前 fast 另显式执行 `routes::app_members::tests`，原 `data_grants::` 单元选择保留。成员目录只允许同应用显式登记的 `memberLookups` pair；旧 opt-in platform-e2e fixture 在真实 bootstrap 获得应用 UUID 后，以窄 ticket/create、ticket/update 声明重启平台，不放宽运行默认拒绝。各阶段实际运行数量与结果以其冻结候选和报告为准；本地选择这些业务及平台模块但排除独立 notification live 的执行必须注明 scoped PG，不能称为完整 postgres 门禁。

Phase 20 新增 `work-orders-reminder-postgres` 与 `work-orders-reminder-live-postgres`。前者精确执行 `reminder_contracts` 的 12 个 ignored 父测试，必须同时给出每测试独立业务库清理与四条确定性证明：未知发布保留同键/原正文且不自动重发、外部 POST 期间业务连接及行锁已释放、未确认 intent 跨进程重启后收件人变化同键冲突、准备事务提交被拒绝时零发布 POST。后者选择实际平台 bin 的 `phase20_work_orders_tests::real_platform_reminder_publication_and_recovery`，要求 1 个父测试和 12 个唯一命名检查，以及双库删除/池关闭、业务进程重启和最终停止证据；通过真实通知 HTTP 与持久收件箱核对结果，不启动实时服务。解析器不接受仅有 Cargo 通过数量的报告，也拒绝缺失、重复、false 或不完整标记。

提醒的持久事件身份新增不可变 `0003_reminder_intents.sql`，原 `0001`/`0002` 字节及 Cargo 依赖保持。当前迁移 target 的 27 项覆盖 fresh/v1/v2/v3 Node 与 SQLx 接管、既有 receipt/intent 保留、回滚和并发升级；历史 Phase16/18 的 15/21 项证据保留当时来源。本阶段结果以冻结候选的实际报告为准，不能将新注册数量或原生开发观察当成已执行的 Linux 门禁。提醒契约和原迁移/读取/写入/成员及其真实平台联合选择分别报告，独立 `notification-live` 仍由普通完整 postgres gate 执行。

Phase 26 新增 `stores-postgres` 与 `stores-live-postgres`，分别上传各目录的 `postgres-result.json` 与直属脱敏 `.log`，不上传门店私有目录或数据库文件。前者精确执行独立门店 crate 的 `postgres_contracts` 父测试，要求 1 个 Rust 通过结果、8 个唯一业务检查及业务库删除/连接池关闭；后者精确执行平台的 `phase26_stores_tests::real_platform_stores_and_cross_app_isolation`，要求 1 个 Rust 通过结果、16 个唯一业务检查、三库/三个专属非超级用户账号的隔离与清理，以及门店、工单和平台进程停止。三个库包含真实业务/grant/审计行数证明；缺项、重复、skip、超时或清理失败均阻止通过。CI 构建两个独立业务 ELF，再由受管环境注入固定产物路径；原平台与工单选择保持独立。该套件使用显式开发 loopback HTTP，Phase 27 浏览器行为与生产部署需各自验收。

Phase 27 的 stores 前端独立安装锁依赖并进入必需 fast 构建和 Node 契约。既有 browser job 另执行受管 `stores-browser-live`，通过真实平台／两个独立业务 Rust API、Rust 登记与授权 CLI、三个隔离数据库和正常验证的 HTTPS/Nginx 验证双消费者。它使用专用 typed 报告，保持原 fixture runner 与平台、工单测试选择。上传仅增加 `reports/browser/stores/result.json`、`stores/logs/` 和 `stores/screenshots/*.png`；私钥、NSS、源码 scratch 和数据库目录保持排除。可选引擎的聚合构建也安装 stores 锁依赖并显式提供公开宿主来源；该可选 fixture 结果不替代真实双消费者门禁。

Phase 28 的 `candidate-gate` 同时要求原 `delivery` 和新 `business-delivery` 成功。新层先构建带浏览器的独立工具镜像并准备声明的基础镜像，再由 Linux 宿主调用专属 driver，不扩展普通 fixture 的数据库环境。业务候选 artifact 名称绑定完整源码 SHA、run ID 和 run attempt，只有所有业务验收成功后上传；内容精确限定为公开 candidate result、source manifest 和已校验 `images/*.tar`。直属脱敏日志、真实浏览器 PNG、三个产品各自 `browser-<product>/fixture-result.json` 原始报告、四个具名原始 ELF、四份版本静态产物、仅 20 位数字文件名的公开 inventory JSON 与联合结果独立保存；registry 存储、Docker 登录配置、私钥、inventory `.lock` 和数据库目录均不进入 artifact。Rust API 的按锁 Linux 构建后 verified ELF 装箱与 Dockerfile builder 执行分别由实际 `apiBuildMode` 证明。
