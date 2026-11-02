# Phase 24：退役旧客户端与旧协议运行路径

日期：2026-10-29。状态：**COMPLETED**。四项阶段验收均通过。结论适用于当前源码、全新基线和下列分层验证；Phase 25 的全服务／浏览器验收及 Phase 26 的真实目标切换继续按台账执行。

## 范围和输入

台账 Phase 1–23 已完成；本阶段依赖的 Phase 5、13–21、23 报告存在并有相应分层证据。它们只作替换前置，不作为当前源码通过证据。[原准备盘点](phase-24-preparation.md)保存历史 R01–R16，最终处置见[退役清单](../phase-24-retirement-list.md)。工作区包含八个独立仓库，开始时已有大量前序阶段未提交变更，本轮在原内容上继续实施，没有重置工作树或已有数据库。

本阶段删除旧平台 OAuth issuer／registry／device／consent、平台同进程 SSO/OIDC provider，以及 identity 旧 `/oidc/*` 和旧 consent/client/context 实现。当前只保留 identity canonical `/oauth/*` 权威；平台和门户为 RP，平台内部能力仍使用独立 token use、audience、原来源与成员代际。

## 当前交付

- `0001`、provision、verify、readiness 删除旧 authority 表、函数、触发器及 ACL；上下文登记只允许 AUTH，精确应用及应用代际不可为空。
- canonical 签名对象改为 `auth_signing_keys`／`auth_provider_settings`。密钥管理使用独立 KEY_ADMIN、`/api/identity/keys*`、上游 `/internal/identity/auth/keys*` 及原 key-operation 恢复。
- 命令存储改为 `identity_auth_command_receipts`／`platform_auth_command_intents`；原 operation、摘要、用途、method/path、原来源、细权限与 MFA fence 保留。两个 runtime 的 append-only／跨 owner 边界均由 catalog readiness 检查。
- canonical 来源函数保留语义并改中性名称；BFF 中描述 RP 来源的 `oidc_client_id/grant_id/public_sid/public_sub` 字段保留，不把它们当成第二 registry。
- 内部能力改为 `/api/apps/session-token`，资源服务预算改为 `RESOURCE_SERVICE_*`；内部续期配置为 `BFF_CONTEXT_RENEWAL_TTL_SECONDS`。旧 provider／issuer 配置明确拒绝。
- 旧客户端/token/device/consent 前端、旧 playground 源码和网关配置能力退役；CLI 只管理业务应用／菜单，独立 `oidc-rp` 为标准客户端示例。当前指南与索引同步。
- 恢复 inventory 使用新表名和 950 个字段；旧 TTL 覆盖在任何恢复材料写入前拒绝。实际 token 截止仍取数据库，profile 请求标签不冒充 TTL 配置。
- 业务 fixture 迁到 canonical source，纯旧 issuer 场景退役。独立审查指出 BFF 并发覆盖缺口后，补回九组真实数据库事务回归；最终当前分组全部通过，包括真实锁等待、短 ID／online proof 截止、refresh 丢包、UNKNOWN／迟到 logout、back-channel 和 step-up fence。
- 独立审查发现登录页 restart 拒绝合法 `/oauth/login` 的 P2，已修复 canonical restart／interaction 路径，新增六项 VM 回归及待实跑浏览器用例。

本轮选择中性签名／命令对象名和独立 KEY_ADMIN，是对原准备稿“可保留旧名”建议的实施细化：移除重复管理门面后，统一固定路径和按用途隔离的凭据，并同步全部消费者、ACL、恢复工具。没有改变单一权威、幂等或来源锁合同；不为旧安装建立别名或迁移兼容。

## 当前验证证据

| 检查 | 实际结果及边界 |
|---|---|
| schema 空库 | [最终 schema 103 项 PASS](phase-24-empty-database-final.json)：`phase24-final-r1`／新集群下的真实 migrator、同版重复迁移、provision／verify、105 表（含 SQLx 历史表）、双 LOGIN runtime、跨 owner／DDL／owner role 拒绝、86 张共享恢复表 catalog 对比；`finally` 后数据库及六角色均无残留。包含成员 FK 修复，不沿用此前旧 schema 结果冒充当前通过 |
| 空库前轮失败 | r1 是驱动漏写 provider.created_at；r2 发现 typed inventory 的两个 nullable 声明未同步；均已清理，分别保留失败记录，不合并为成功轮次 |
| identity 当前完整 lib | [最终新集群证据](phase-24-identity-final.json)：141／141 PASS、0 ignored（86 普通、24 小 SCHEMA/mock-intent PG、31 最终 canonical 基线 PG），main 1、fmt、strict Clippy 通过。100 个编译输入与当前字节一致，包含成员 FK 后续变化 |
| identity 先前失败 | schema-r2 为 126 PASS／15 FAIL；13 项 Linux fixture 调用 Windows 工具、2 项测试小 SCHEMA 缺当前 logout/admission 字段。修复仅在当前测试设施，失败轮 DB／roles 清理无差异，不把旧结果改写为通过 |
| 平台当前门禁 | [最终分组汇总](phase-24-platform-final.json)：432 个唯一当前用例均有 PASS 证据，191 个原 ignored 已实际执行。其构成为 final-r3 七组 50 PASS（45 PG）＋输入等价的 382 个既有 PASS，**不是一次运行 432 PASS**。final-r1 全量 424 PASS／7 FAIL 和中间 static FAIL 均独立保留。完整 fmt、all-target/all-feature 编译、strict Clippy 通过 |
| 消费者／前端／CLI | [组件证据](phase-24-consumer-components.json)：前端 474、门店 83、工单 77、标准 RP 46、Linux CLI 57 均通过；最终类型及构建检查单列。无测试前全量 manifest 的早期组件结果已明确披露 |
| 网关 | 便携 Ubuntu Nginx 1.28.3 实际 14 个顶层检查全 PASS；总计 17 tests／16 PASS／1 nested browser SKIP。Node 显式验证 CA／hostname，缺 CA 和错误 hostname 均在上游前拒绝。独立浏览器子测仍未运行，不借此替代 Phase 25 浏览器验收 |
| 恢复／harness 纯组件 | recovery 119、harness Python 156、fixture 12、collector 5、Node 52、UI VM 6 均通过；其后移除任务网关旧 context-source 别名的定向 6 项通过。Source copier 补齐真实编译／配置输入及 0600 模式，最终实际冻结／编译成功；这些组件不是实际 DR／多服务 HTTPS 演练 |

完整平台 PG 发现并修复两个生产遗漏：已签发内部能力的 membership 外键会阻塞成员永久删除，现仅将 `platform_app_capabilities.membership_id` 改为 `ON DELETE CASCADE`，缺失能力记录立即拒绝原 jti，独立安全审计／来源事实保留。永久离职、其他成员能力隔离及审计失败的完整事务回滚已实测通过。应用策略失效 helper 原先仍调用已删的旧 SQL 函数，现于原事务精确撤销目标 application 的内部能力，与版本验证及当前触发器一致；scope 收缩和原子审计回归通过。

七项最后失败分别为账户 fixture 未启生命周期、旧 Cookie 合同应为 409、已退役 404 空响应、UUID CHECK／validated-FK 约束闭包识别，以及 data-grant fixture 必填字段；当前测试修复保持生产校验及正负控制。最终纯样式修正后重新执行七组 50 项，独立差异审计证明其余 382 项输入等价。完整来源、逐用例结果和去重规则均在汇总 JSON 中，旧失败没有覆盖成 PASS。

原 R15 通用 recovery 工具还实际调用旧 OAuth `invalidate.sql`。现已删除该脚本和旧 restore 实现，公开 `restore` 在任何配置读取、SQL、子进程或目录写入之前以 `CANONICAL_IDENTITY_RECOVERY_REQUIRED` 拒绝。备份／完整性核验继续保留，新 identity-recovery 的共享认证库／门户范围与业务库独立恢复责任已在文档区分。[R15 正式组件证据](phase-24-recovery-retirement.json)记录 RED 复现、修后 7 单元＋1 CLI、strict Clippy、fmt 和当前源码摘要；不是实际恢复演练。

当前最小空库复现：

```sh
python3 -B saas-plateform/backend/tests/phase24_empty_database.py \
  --task-dir /tmp/rust-saas-unified-auth-20261028 \
  --snapshot phase24-final-r1 \
  --migration-bin /tmp/rust-saas-unified-auth-20261028/target-linux/debug/platform-migrate \
  --output saas-plateform/backend/docs/designs/unified-auth/validation/phase-24-empty-database-final.json
```

该命令须从已核对归属的 task 环境运行，使用与快照同版的 migrator；不能对任意业务库重放。最终平台测试快照为 `phase24-final-r3`，SHA256 `78d6b1604ae590888353619d40c32f0758926388729346459f75cab08ad97f45`。其与空库／身份证据的复用输入已逐文件审计，参见[最终输入审计](phase-24-platform-final-input-audit.json)。Root 再独立核对当前 390 个平台输入、100 个 identity 输入、最终 schema 和 22 个正式报告引用，均匹配；两个生产源文件末轮差异仅空白。

## 资源及限制

WSL 在 2026-10-29 15:36 左右重启，uptime 10.45 秒、整个任务 tmpfs 目录丢失；触发原因未建立，本任务未执行 shutdown／terminate。不能将临时文件丢失计为定向清理。原正式身份／消费者报告和平台失败 checkpoint 位于持久工作区，未丢；原 raw／snapshot 不可再读取的限制独立记录。见[环境重建记录](phase-24-environment-restart.json)。

当前由单一 owner 重建原生 PostgreSQL 55494，**新** system identifier `7701992637128815755`，重新初始化绑定并核对持久缓存 OWNER。Windows 隐藏 supervisor PID 61268 持续持有任务；当前冻结 `phase24-final-r1` 的 SHA 为 `5f191160608a118298727e3027304f05cf710e88ea403a853a063c696b029df0`。新 Node 24.22.1 经官方 SHASUMS 校验恢复，notification 从当前 15 个输入构建，CLI 的 32 个输入／binary 字节相同后复用。当前 runner／raw 改放持久缓存中各 owner 的独立 0700／0600 清单目录，结束按清单定向删除。

没有操作 5432、原 443、HBA、正式数据库、DNS 或在线服务。Docker context 为 default，创建 0、复用 0，四类 `docker system df` 均 0B。数据库测试的临时库／角色按独占前缀和 marker 清理；任务共享 PG、源码快照、Node 与 `/home/jackie/.cache/rust-saas-unified-auth-20261029` 编译缓存仍供本次后续阶段复用，整体目标结束须按 OWNER 定向清理。

[平台专属清理](phase-24-platform-cleanup.json)已 PASS：按四个 OWNER 清单删除 76 文件／309,448,977 逻辑字节和 8 个目录，包含 raw、runner、状态和旧诊断 binary；数据库、角色、临时运行文件和匹配 PID 无残留，188 个正式报告 SHA 保持。身份最终清理另删除 9 个自身文件／45,879 bytes、密钥／CA／PID 无残留，未清理并行平台资源；旧轮清理范围在各自证据中独立保留。

CLI 本次 Windows 专用 target 已经 cargo clean，删除 2,359 文件／717.4 MiB，残留项 0。便携 Nginx 的 37 个文件／2,443,133 逻辑字节及进程／测试目录已清理。不同文件系统的这些数字是各自逻辑统计，不相加冒称 Windows 实际回收量。Docker 创建／复用 0，前后四类均 0B；共享任务环境继续供 Phase 25 使用。

旧 playground 预存 ignored dist／node_modules 的 48,198,145 逻辑字节仍在；自动审批以 `blocked by policy` 拒绝其精确目录递归清理，未换工具绕过。它们已无当前源码、构建、网关或 CI 引用，不是本次新建验证资源。未执行 WSL 停止或 VHDX 压缩，不声称 Windows 磁盘已完全回收。

## Self-Check

| Acceptance Criterion | Result | Notes |
|---|---|---|
| 无独立运行的业务／登录两套客户端和 OAuth 签发链路 | PASS | 当前新安装源码／路由／网关仅 identity canonical 签发，旧入口真实拒绝；既有部署的实际切换属于 Phase 26 |
| 当前空库仅按目标基线建立，owner／readiness／初始化一致 | PASS | final-r1／新集群的最终 schema 103 项通过，数据库及角色清理完成 |
| 保留 BFF／内部能力用途明确，不是第二授权服务器 | PASS | 当前 BFF／客户／资源／通知用途、来源、audience、事务和失效回归通过；无旧 provider/SQL 覆盖 |
| 原切换清单每项有去向或删除证据，无旧 token／checksum 兼容旁路 | PASS | R01–R16 均已对照闭合；当前 helper 闭包及严格 checksum 检查通过，R15 旧 restore 明确关闭 |

## Suggested Task Ledger Update

Phase 24 → COMPLETED；四项验收全部 PASS。当前阶段组件和 source 退役已无 actionable 未完成项。按已授权的全部阶段依赖顺序继续 Phase 25 → IN_PROGRESS，对当前完整版本执行新安装、十八类矩阵和多服务／实际浏览器／外部标准客户端验收；Phase 26 仍依赖真实目标、责任事实和针对操作的授权。Q3／Q8／Q9／Q10 门禁不因本阶段组件通过而取消。
