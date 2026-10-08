# Phase 12 原生部署与联合验收证据

> 目标基线：用户于 2026-10-02 确认的本机隔离 Linux 原生准生产验收；2026-10-07 继续执行。最终正常完整运行 **23/23 PASS，0 fail／skip／cancel**；独立清理审计通过。台账／工程最终交付由阶段总报告汇总。

## 部署产物与工具

`deploy/native/build.mjs` 在新的私有目录内复制明确允许的源文件，排除项目安装目录、已有构建、`.env` 和 Git 数据；独立执行宿主、通知 API、工单 API、Vue 应用的 `npm ci`，再执行 TypeScript、Vite 和 `cargo build --release --locked`。使用 Node 24.3.0、Rust 1.99.0，平台 23 条迁移嵌入实际 release 二进制。

最终持久构建目录为 `/var/tmp/dante-phase12-clean-build-7fDcZs/build`，来源 259 个文件，SHA256 清单摘要为 `73a753f4d7bc28ff47c95f50fc19f48af9efe7508a6ee9003d9a165be632d76a`。本机 WSL `/tmp` 为 tmpfs，实例停止后丢失早期候选构建，因此没有把早期构建通过当作部署成功；最终目录位于 ext4，权限 `0700`，完整重新构建后用于实际部署。

平台使用实际 release 二进制；宿主、通知及工单 API 使用冻结的独立安装／构建产物。工单 v1/v2 从同一干净源码中的独立 Vue 构建脚本生成，各自打包为不可变版本。没有 Vite 开发服务、伪造 JWT 或替代业务响应。

原生 Nginx 1.28.0 单独构建 SSL／HTTP2 模块，链接系统 OpenSSL 3.5.5；保留原有非 TLS 测试工具。浏览器使用 Chrome 151.0.7922.34、Playwright 1.62.1；PostgreSQL 服务为 17.6，固定客户端 17.11，局部 libpq 18.6。

## 实际 HTTPS 与浏览器隔离

专用 CA 与服务证书仅用于本机演练，SAN 包括 localhost、127.0.0.1、::1。独立 TLS 证明确认：

- Node 和 Chrome 未加载该 CA 时拒绝证书。
- 加载 CA 后，实际 HTTPS 200，协议 TLS 1.3；主机名不匹配仍被拒绝。
- Chrome 使用自有 `XDG_DATA_HOME/pki/nssdb`，导入前检查旧全局 NSS 库不存在；全局信任元数据保持不变。
- 未覆盖 HOME，未创建全局账号，未使用 `ignoreHTTPSErrors`、忽略 SPKI 或关闭 TLS 验证。
- 浏览器环境只包含运行时路径、语言、字体及 XDG／临时目录，没有数据库 URL、JWT、客户端密钥或指标 token。

TLS 原始证明位于 Git 忽略的 `target/phase12-runtime/tls-tools/browser-ca-proof-RCgxrY/proof.json`。准生产网关／平台／业务内省／通知平台调用均验证专用 CA，实际浏览器通过同一 HTTPS 网关运行。

## 联合验证覆盖

| 领域 | 实际验证 |
| --- | --- |
| 生产启动 | 显式随机管理员与 JWT／各服务指标密钥；release API 拒绝演示密码写入且数据库没有新增弱密码用户 |
| 网关与 CSP | TLS1.2/1.3、HSTS、nosniff、精确来源；默认不允许 eval／inline script；opaque iframe 正常执行；私有指标与缺失 API／静态文件严格 404 |
| 迁移停止 | 独立坏库预置冲突表，真实 release 进程非 0 退出且不开放健康监听，原 marker 保留，原正常部署仍 UP |
| 两租户与权限 | 原完整工单矩阵重用：四级范围、同全局用户跨租户、祖先角色不扩大直接数据授权、独立操作／数据撤销、应用／成员失效和组织切换 |
| 真实业务 | HTTP 与生产 Vue 创建、版本冲突、草稿、状态流转／重开、软删除保留、实际 CSV 下载和公式中和 |
| 通知 | 真实持久发布、Socket.IO、收件箱已读、响应丢失后的同键重试、幂等冲突及动态收件资格 |
| 独立发布 | v1→v2→v1，宿主 100 个文件逐个 SHA256 保持，宿主 DOM／会话／JS 请求数保持，旧 iframe 移除 |
| 请求关联 | 同 UUID 响应、应用通知审计 requestId/applicationId、可信代理覆盖伪造 IP；普通租户审计不能通过 tenant filter 越权 |
| 微应用失败 | 自有当前版本入口临时移走，真实宿主错误卡发送受认证 LOAD_FAILED 事件，私有指标计数；恢复相同字节并手动重试后重新挂载 |
| 服务故障 | 只停止自有业务／通知 PID，连续两次失败产生 DOWN，恢复两次产生 RECOVERED；不发送外部消息 |
| 数据库／LISTEN | 仅禁止自有平台库连接并终止其服务连接，平台数据库指标 0、通知 LISTEN 指标 0、健康 DOWN；恢复连接后健康、LISTEN 和对应告警闭环恢复 |
| 加密恢复 | 停止所有写入者，两库 AES-256-GCM 备份；认证两份密文后恢复到新库，业务软删除／授权／通知／发布／迁移校验和一致 |
| 恢复认证 | CLI 强制提升 user version、撤销会话／令牌并追加审计；保持旧 JWT key 时旧令牌已拒绝，再轮换 key、重新登录，真实 CRUD／CSV／通知／跨租户／同 token 撤销再验证 |

测试上下文递归检查子测试是否完成，任一子测试失败立即停止后续验收，不因 Node 子测试 Promise 已结束而误标通过。数据库名、Linux PID／start time／exe／cwd、浏览器身份实时写入 Git 忽略的 ownership 清单，便于中断后精确清理。

## 配置检查与部署边界

`tests/production-native-config.mjs` **4/4 通过，0 skip**：HTTPS／路径注入／回环上游政策；默认 CSP 与显式可信 eval；真实 Caddy adapter 解析原生自带证书与 Compose 自动证书策略；真实 Compose parser 拒绝缺少密钥并接受两个声明 profile。

Docker 配置移除开发默认密码／JWT，显式生产 HTTPS，独立指标 token，Rust/Node 非 root、只读根文件系统、最小 capability、健康探针及 Caddy 证书持久卷。网关不能等待“依赖其 HTTPS 的通知健康”后才启动，因此 Compose 网关只等待通知进程开始，随后就绪探针收敛。

本机没有 Docker daemon，未声称容器实际构建／运行。systemd 模板使用四个不同服务身份防止读取其他服务秘密；本次实际进程使用当前 Linux 用户，未安装系统 unit、账号或 timer。生产数据库最小角色及现网 DNS／TLS／防火墙仍由真实部署环境按运行手册执行。

## 历史失败与修复

首轮完整运行中 LISTEN 断言遗漏固定的 `{service="notification-ws"}` 指标标签，判定失败；已修正为真实指标格式，失败证据没有标成通过。随后正常完整 23/23 通过。进一步补齐数据库故障后通知告警的 RECOVERED 配对，并清除测试内部未取消的长 deadline 定时器。

后续运行中一次恢复 CLI 返回粗粒度 POSTGRES_TOOL_FAILED；该次停止并清理自有资源，没有隐去或自动重试数据库修改。恢复工具新增只包含固定 operation stage／失败类别／SQLSTATE／退出码的诊断，仍禁止输出原始 SQL、客户端 stderr、数据库 URL 或密码；其新增实际 PostgreSQL 诊断门禁已经通过。最终联合重放记录在下一节。

## 最终正常运行与清理

最终正常完整运行于 **2026-10-07 10:37:59（Asia/Shanghai）**完成，Node test runner **23/23 PASS，0 fail／skip／cancel**，总耗时 **305.493 秒**；实际顶层部署／业务／恢复流程 **301.040 秒**。恢复安全诊断新增后，此次通过相同冻结的 release、宿主和 Node 产物重放全部门禁，没有自动重试恢复数据库操作。

| 测量项目 | 最终实测 |
| --- | --- |
| 授权容量样本 | 40 请求，并发 4，10.598 秒，约 3.774 请求／秒 |
| p50／p95 | 569.871／2007.932 ms |
| 失败认证保护 | 30 次 401、5 次 429；合法机密客户端随后继续 200 |
| 配额 | 合法客户端 600／分钟，失败认证 30／分钟，未关闭保护 |
| 本地告警 | 8 条，业务／通知进程各一组，数据库故障下平台／通知各一组，所有 DOWN 有相应 RECOVERED |
| 加密两库备份 | 6.045 秒 |
| CLI 新库恢复 | 8.930 秒 |
| 完整恢复就绪与终验 RTO | **23.466 秒**，包括认证重置、旧 key 下旧 token 拒绝、签名密钥轮换、服务就绪、新登录与 CRUD／CSV／通知／隔离／撤销检查 |
| 认证重置 | 8 用户 version 提升；9 活跃会话、12 令牌失效；该样本无活跃 request／code／device |
| 构建／宿主 | 最终 source manifest 摘要如上；宿主 100 个文件字节保持 |

RPO24h／RTO1h 是本次演练目标，不承诺商业 SLA。容量来自当前本机、WSL／VPN 环境及测试进程 TCP_MAXSEG=1000 配置，不能外推为生产承诺。

主流程清理及其后的独立 Linux／PostgreSQL 查询均确认：**自有数据库 0、这些库连接 0、自有服务进程 0、浏览器 0、临时目录 0**。独立审计以事先登记的 5 个精确随机库名及 PID/start time/exe/cwd 读取状态，不枚举删除其他资源。

最终 Git 忽略证据：

- `target/phase12-runtime/native-evidence-dante-work-orders-platform-e2e-G7c7kt.json`：安全聚合、容量、8 个告警、完整 RTO、认证重置和清理结果。
- `target/phase12-runtime/native-ownership-dante-work-orders-platform-e2e-G7c7kt.json`：精确资源归属和退出状态，无连接串／密码／token。
- `target/phase12-runtime/qa-dante-work-orders-platform-e2e-G7c7kt/`：浅色、编辑、冲突、深色和窄屏截图。

复现入口为 `deploy/native/acceptance.sh`／`tests/production-native-e2e.mjs`；必须显式提供当前构建、TLS/NSS/浏览器/PG 工具路径和隔离测试环境文件。缺少 opt-in 配置时测试会跳过，该跳过不能算门禁通过。

所有非秘密 QA 截图按已有保留约束留在 `target/phase12-runtime/qa-*`，没有尝试绕过此前的删除拒绝。构建及公开工具缓存按计划保留；不提交真实环境文件、私钥、数据库备份或测试数据。
