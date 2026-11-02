# Phase 25 当前完整候选验收

状态：`IN_PROGRESS`。本报告记录当前执行事实，不构成 Phase 25 完成、Phase 26 切换或整个统一项目完成的结论。

## 执行边界

前置为 [Phase 24](phase-24.md)，当前台账指针为 Phase 25。参与项目为身份、平台、门户、门店、工单、通知、SDK 和标准 OIDC 示例。保存所有已有未提交工作；不改动实际 5432 业务库、正式入口、DNS 或既有密钥。

容器新建和复用均为 0。复用受保护 binding 对应的任务 PostgreSQL 55494 和编译缓存，单一 owner 控制临时服务、网关、数据库和 HBA 写入窗口。并行组件测试使用独立数据库前缀；其完成前不开始同集群的 HBA／恢复验证。资源计划见 [本轮预查](phase-25-current-execution-preflight.json)。

## 当前候选与证据

- [完整候选构建](phase-25-candidate-build.json)：`phase25-apps-r2`，应用与迁移共 12 个二进制，八套新前端产物。它证明构建与输入完整性，不能代替实际运行。
- [tools-r2 等价证明](phase-25-tools-r2-equivalence.json)：1154 个编译输入和八套冻结前端共 227 个文件与 apps-r2 完全等价，12 个二进制摘要核验通过。后续工具快照另行绑定，不修改本记录。
- [tools-r3 等价证明](phase-25-tools-r3-equivalence.json)：纳入监督器权限修复、资源权限与标准协议扩展、管理／登出适配及脱敏执行 journal；1551 输入中相同的 1154 编译输入、227 前端文件和 12 二进制均核验通过。它仍不证明实际运行通过。
- [修后完整候选 apps-r3](phase-25-candidate-build-apps-r3.json)：全量 12 二进制、8 条 locked/offline 构建命令通过，保留旧 apps-r2 报告。新旧逐组件输入闭包仅门店、工单两个 `backend/src/lib.rs` 生产文件变化，其余身份／平台／门户／通知／SDK／示例同源；八套冻结前端仍为同一 227 文件。下一轮实际验证须使用 apps-r3 和另行冻结的新工具。
- [当前身份组件](phase-25-identity-components.json)：142/142，含本轮实际 PostgreSQL；首次运行的环境配置失败独立保留。跨项目 include_str、CLI、通知二进制和运行时补充核验见 [输入闭包补证](phase-25-components-cross-input-supplement.json)。
- [平台组件合成](phase-25-platform-final-components.json)：当前 apps-r2 对应 432 个唯一用例通过。原全量 408 PASS／1 FAIL、补齐显式 TLS opt-in 后的定向 1 PASS 和其余 23 集成用例分别保留，合成证明核对测试二进制摘要，没有改写原失败。
- [门户组件](phase-25-portal-components.json) 34/34 和[通知组件](phase-25-notification-components.json) 39/39 通过；其输入仍明确标记 apps-r2。门店／工单启动修复后重建候选，未变化组件必须另有新旧输入等价证明才能组合使用。
- [修后门店组件](phase-25-stores-components.json) 98/98 和[修后工单组件](phase-25-orders-components.json) 149/149，均从 apps-r3 执行，0 failed／ignored／filtered，真实 PG 显式执行，前后源码和辅助二进制核验、专属资源清理通过。各组件的可复现命令与分层说明集中于[本轮组件套件](phase-25-component-suite.md)。
- [恢复组件](phase-25-recovery-components.json)：119/119，仅组件，不冒充本轮实际恢复。
- [业务浏览器修复回归](phase-25-business-review-fixes-r2.json)、[监督器 Linux 权限回归](phase-25-business-supervisor-permission.json)、[真实浏览器 Cookie 阻断回归](phase-25-ui-cookie-components.json)、[资源权限编排回归](phase-25-resource-components-r1.json)、[标准协议回归](phase-25-protocol-components.json)和[管理适配回归](phase-25-management-adapter-components.json)均按各自实际证据层级使用，尚不能代签后续实际 SaaS HTTPS 结果。

## 实际运行与失败保留

`r1` 使用 tools-r2，完成当前 canonical 共享库、门户库、门店库和工单库的新安装；身份两个实例、平台、门户和独立标准 RP 的严格 HTTPS 就绪检查通过。业务监督器随后因非特权用户读取 root `/proc/<pid>/ns/mnt` 触发 Linux 权限限制而退出。浏览器矩阵与 HBA 安装尚未开始。

该轮失败独立保留于 [r1 启动失败](phase-25-r1-startup-failure.json)。finally 完成四库、13 个角色和全部本轮进程清理；核心清理证明见 [r1 cleanup](phase-25-r1-cleanup.json)，业务两个库和五个角色的清理另由启动失败记录对应的实际命令输出记录。核心删除 38 个私密文件、27310 逻辑字节，进程与 lease 残留均为 0，Docker 四类占用均为 0B。此结果不等于 Windows WSL VHDX 已缩小。

监督器修复继续保留 root 父进程的完整 namespace 检查，非特权子监督器改为受保护 lease、精确 controller argv／PID／startTime 与自身 mount namespace 校验。真实 Linux 权限回归和组件回归通过；下一轮实际服务结果另记，不能把 r1 改为通过。

`r2` 使用 tools-r3，监督器已能启动真实业务子进程；子进程随后退出，固定诊断为 `PHASE25_BUSINESS_CHILD_EXITED`。尚未运行 UI 或安装 HBA。执行阶段和失败事实见 [r2 journal](phase-25-r2-execution.json)，核心 finally 见 [r2 cleanup](phase-25-r2-cleanup.json)。本轮同样删除四库、13 角色，核心 38 私密文件／27324 逻辑字节，进程与 lease 残留为 0；应用退出原因继续诊断，不能将其记为服务或浏览器通过。

后续[真实受限 LOGIN 实验](phase-25-readonly-startup-pg-r3.json)证明：已有 `_sqlx_migrations` 可 SELECT，SQLx 同款 `CREATE TABLE IF NOT EXISTS` 仍因没有 schema CREATE 返回 `42501`，原表继续可读。实验一个专属库和两个角色均已清理。门店／工单正常运行原来仍调用 SQLx migrator，现已修为仅验证当前 checksum／fingerprint；独立迁移程序继续负责建库后初始化，runtime 不增加 DDL 权限。[门店受限启动](phase-25-readonly-startup-stores-r2.json)、[工单受限启动](phase-25-readonly-startup-orders-r1.json)均证明真实 runtime HTTP ready 200、schema/history 前后摘要不变、空库／错 checksum／错 fingerprint／缺历史拒绝，资源 0 残留。修后完整重建与消费者组件已通过，当前全服务 HTTPS／浏览器结果仍须另记。

后查纠正：r1／r2 的核心 cleanup 只包含 `phase23-*`／`service-*` 日志，独立盘点发现 `phase25-business-stores.log` 仍有 163 字节。因此，上述核心 cleanup 的 PASS 只支持其原记录范围，不能扩张成所有业务原生日志已经删除。[纠正记录](phase-25-r1-r2-native-log-correction.json)保留固定 `PHASE25_APP_DATABASE_BASELINE_VERIFICATION`、原 SHA 和旧 ENV 已不存在的限制；[后续定向清理](phase-25-r1-r2-native-log-cleanup.json)已删除该 163B 文件，精确三项业务日志残留为 0。不能回溯的旧 canary 扫描不伪称零泄漏。新 stop 流程在业务 ENV 仍存在时，对真实秘密、DB URL／password 和 Basic 凭据执行内存比对，记录计数／SHA 后定向删除；59 项回归通过。新的完整运行仍须提交其实际日志扫描结果。

## 矩阵和发布门禁

[111 项准备矩阵](phase-25-matrix.json)必须逐条绑定本轮实际执行和具体断言。[证据层级独立审查](phase-25-evidence-layer-audit.md)区分数据库安全不变量、标准 SDK、真实浏览器、跨服务、故障恢复和清理；不要求每项后端负例重复浏览器主链，也不允许用组件总数提升整个场景结论。当前没有全部通过结论。

[部署计划](phase-25-deployment-plan.md)列出本机四个候选新库、13 个角色、服务与入口、受保护参数、操作顺序和整体回退门禁。正式目标授权、真实审核／发布／恢复与值班人员、邀请核验渠道、保留和恢复作业等仍须按 Phase 26 核实；不以 fixture 人员、任务网关或历史盘点代填。

## 已知清理阻塞

[前端原清理记录](phase-25-frontend-cleanup.json)和[收窄后重试记录](phase-25-frontend-cleanup-retry.json)均记录自动审批在命令启动前拒绝，原因仅为 `blocked by policy`。四个 OWNER 标记的 Windows 临时目录、30 个指向现有 node_modules 的 junction 仍保留，原记录合计 61607240 逻辑字节。两次均未执行删除；没有换 shell 或继续拆分重试。原依赖目录保持保留。该项未解决前，不能将完整资源清理验收标为 PASS。

本轮 [identity DOM](phase-25-identity-dom.json) 13/13 通过，浏览器与 HTTP listener 已关闭，但其独占临时镜像清理也在命令启动前被同一自动审批拒绝。准确目录和 OWNER／依赖链接盘点见该 JSON：17 普通文件、382558 逻辑字节、5 目录、1 junction，删除 0。它与上面的四个目录分别记录，不能用测试通过代替清理通过。

## Self-Check（执行中）

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 十八类矩阵所有条目有当前可追溯结果 | PENDING | 组件新执行与实际 HTTPS 正在补齐，未以旧报告替代。 |
| 空库安装所有参与服务与标准示例，失败恢复可用 | PENDING | r1 新安装和核心就绪通过，业务监督器失败已修，需新实际验证。 |
| 风险、容量、密钥、告警、运维责任及外部说明齐备 | PENDING | 候选部署清单已形成，当前容量与完整密钥恢复待执行，真实运营事项留 Phase 26 硬门禁。 |
| 实际目标、授权操作、回退触发及完整清理 | PARTIAL | 具体清单已列，Windows 前端临时资源清理被自动审批拒绝；其余本轮验证仍继续。 |

## Suggested Task Ledger Update

当前保持 Phase 25 执行中，暂不推进 Phase 26，不直接覆盖台账。待本轮验收与独立审查完成后，按真实 PASS／阻塞事实提供最终阶段状态、剩余事项和下一阶段建议。
