# Phase 25 执行恢复指引

当前工作继续进行，未请求暂停、未完成 Phase 25、未执行 Phase 26。主报告是 [phase-25.md](phase-25.md)，台账仍指向 Phase 25；按用户全部阶段授权持续推进，不要求用户反复“继续”。

## 最新状态覆盖（r4 实际结束后）

以下覆盖后面旧的 r3 启动准备描述：apps-r3／tools-r6 已实际执行 RunId r4，详见 [r4 汇总](phase-25-r4-preflight-summary.json)。全部六服务与标准 RP ready；UI9 PASS、logout13 PASS；business 两子例后 stores read503、resource baseline503、protocol22检查后 scope 误断言 FAIL、context2后详情路由 fixture 超时、management2后客户端投影503。任何独立前置失败都会阻止后续 capture／业务 DR／HBA／identity DR。

r4 已 finally 清理四库、13 角色、全部进程与 lease，核心40私密文件257370字节；三业务 native log 2165字节在13个真实秘密内存比对0发现后删除。当前没有 root 实际运行 session；不恢复已清理环境，不重用 RunId r4。旧 protocol 原始120926字节含113363前导NUL与7个HTTP状态覆盖 verdict，原 bytes/SHA 不变，另有 decoded audit，不能把它冒充有效全 PASS JSON。

下一轮预计为 **apps-r4 / tools-r7 / RunId r5**，冻结之前先等这些最终信号：

- `runtime_matrix_plan`：修 platform `remote_admin` 错误要求 public clientId==internal id 的生产投影；合同允许有界无control opaque public clientId，internal id/route/credential receipt UUID语义保持。做定向真实PG/投影回归，生产 FINAL 后 `candidate_audit` 全构建12二进制 apps-r4。
- `business_matrix_extension`：stores503 尚不能归责。已新增同原真实carrier的冻结只读诊断 helper，下轮失败时保留private authorize／directory／business status+固定code/形状布尔；原token只stdin不输出。另修确定性 STORE child 显式 stores organization_applications fixture，不能用此后续空目录问题冒充503原因。
- `matrix_audit`：protocol三项已 FINAL 修复（原grant ceiling之外 offline_access负例、原scope恢复合法正例、position0且短写处理、HTTP状态不覆盖verdict），11回归通过，等新真实issuer。主矩阵仍54/18/38/1，r4真实事实尚未全部接入。
- `deployment_audit`：C13显式隐藏详情 MENU+精确links fixture与安全stage诊断已 FINAL，10控制回归＋原前端路由policy1通过；实际update仍需新跑。
- `ui_acceptance_completion`：新增小UI验收（打开modal首次Tab前焦点、pageerror0、BLOCK错误页URL/storage无credential降级），不改生产；其 FINAL 前不复制工具。旧UI9真实PASS不涵盖未测新断言。
- `candidate_audit`：负责各FINAL的独审／apps-r4 fullbuild／tools-r7 compiled闭包+八dist+12bin证明，不修改已冻结快照。tools-r4仍是未运行中间输入；tools5/r3和tools6/r4的历史FAIL保留。

root 当前 wrapper 已改为同一新安装中汇总所有独立 preflight 的退出码，显式 `$PSNativeCommandUseErrorActionPreference=false`，任一FAIL仍阻止恢复窗口和整体PASS。不要边跑边修改 live wrapper；各script用自身finally并只按实际完整组结果升级。

## 当前可用输入

- 完整应用候选 **apps-r3** 已 locked/offline 全构建 12 二进制，SHA `a078317efaaa94abeb8cf2d96c37077dee5d5eb438f64351e837f75982dbbe49`，见 [新完整构建](phase-25-candidate-build-apps-r3.json)。
- 修后门店／工单正常 API 只 verify canonical 基线；离线 migrator 负责初始化。实际受限 LOGIN 启动及校验失败负例通过，没有给 runtime DDL。
- **tools-r4 是中间输入，不得选择运行**：复制后独审又发现业务 DR cleanup guard，需要用下一新 label 冻结；不可修改既有快照字节。
- `candidate_audit` 等待 `business_matrix_extension` 和 `runtime_matrix_plan` 的业务 DR 最终修复／终审信号后，冻结 tools-r5，并以 apps-r3 检查 1154 编译输入闭包、八套冻结 dist 和 12 个 binary SHA。apps-r3 不需因纯测试工具变化重建。
- 任务目录仍为 `/tmp/rust-saas-unified-auth-20261028`；真实 cluster 必须从受保护 `phase23-cluster-binding.json` 与实际 PG 核对，不能抄历史 system ID。组件已全部结束，无 PG／Cargo 测试占用。

## 下一实际执行

旧 r3 命令仅作历史；下一轮按上面的最新覆盖改用全新的 apps-r4/tools-r7/RunId r5，并要求其实际等价证明通过后才运行：

```powershell
pwsh.exe -NoProfile -File saas-plateform/backend/tests/unified-auth-https/phase23-execute.ps1 -Phase 25 -Snapshot phase25-tools-r5 -BuildSnapshot phase25-apps-r3 -RunId r3
```

root wrapper 已接入：全新 core/business fixture → 核心和业务服务 → UI → 业务 → 资源收缩 → 标准协议 → 真实双 tab／微应用／通知 context → 管理 → 登出 → fresh employee recovery capture → stop business → 两业务库 restore-new → business fixture cleanup → 本次 HBA → 完整签名密钥与 auth/portal DR → finally。

finally 含 stop business、DR `--cleanup-only`、业务 fixture 清理、核心进程清理、HBA 原字节恢复和最终数据库／角色／私密文件清理；所有失败保持真实证据。下一 RunId 不复用已有结果。r1、r2 已失败并清理，不修改历史 FAIL。

业务 DR 只创建任务专属两个新恢复库并复用原 owner/runtime，不改原库／角色／HBA，保留 owner/ACL/原 grantor，比较数据、history、schema、权限摘要及原真实未过期 bearer 的目标 HTTPS GET；RPO=0 只指停 writers fixture，RTO 不冒充正式生产承诺。恢复 handoff 是 TASK 0600 文件，formal report 不含 raw 值。early/no-resource cleanup-only 必须幂等。

本轮尚需实际容量故障套件；必须等完整环境 finally 和全部 PG 测试结束后另建其独占 fixture：

```text
wsl.exe -d Ubuntu -u root --exec python3 -B <frozenTools>/phase22-run.py --phase 25 --label <最终tools标签> --build-label phase25-apps-r3
```

不得并发两个实际环境或混用旧 binary。每次源码修复按受影响范围重建、重新冻结并明确等价范围。

## 当前已验证及仍未验证

当前组件：身份142、平台432个唯一用例分层合成、门户34、通知39、修后门店98、修后工单149、恢复纯组件119。见 [组件套件](phase-25-component-suite.md)。平台原始单次 FAIL 保留，同测试 binary 的 TLS opt-in 定向 PASS 和遗漏 integration 组成透明合成，不声称单次全绿。

主矩阵现为 **54 PASS／18 PARTIAL／38 PENDING／1 BLOCKED**；所有需要实际 X/B/R 的条目没有误升 PASS。[原准备稿](phase-25-matrix-preparation.json)保留。`phase25-matrix-refresh.py` 从原稿按直接 named assertions 重建；后续新实际结果需要显式接入并逐条复审。C14-S02 特别保留“SCOPES 合法写成功”正例缺口，不能只因撤权拒绝就 PASS。

r1／r2 业务失败后发现遗漏一个 163B native stores log；现已独立读取固定 `PHASE25_APP_DATABASE_BASELINE_VERIFICATION`、记录原 SHA 并精确删除。原 ENV 已消失，不能回溯完整旧 canaries，见 [纠正](phase-25-r1-r2-native-log-correction.json)／[清理](phase-25-r1-r2-native-log-cleanup.json)。新 business stop 在 ENV 仍在时内存扫描精确三 log，再按 SHA 删除；59 组件通过。

## 不能遗忘的阻塞与结束清理

Windows 五个临时镜像树（此前四个前端/npm 加一个 identity DOM）、31 个依赖 junction 尚保留，约 61989798 逻辑字节。原生定向删除被自动审批在启动前拒绝，理由仅 `blocked by policy`；不得换 shell、拆分或低层删除绕过。详见 [旧前端清理](phase-25-frontend-cleanup-retry.json)及 [identity DOM](phase-25-identity-dom.json)。浏览器／HTTP listener 已关闭；此项未解决不能宣称完整清理 PASS。

当前共享任务 PG、源快照、Cargo cache、构建日志、运行时和 binary 仍保留供验收；整个目标结束前须按明确 OWNER／绝对路径定向清理不再需要者。正式源码、报告和需要交付的候选工件保留；Docker 创建／复用均0，df四类0B，不声称 Windows VHDX 已缩小。

Phase 26 尚须最终候选、目标四新库／13角色／入口／秘密路径及针对性授权、真实审核发布／邀请核验／运维恢复人员和政策事实。先完成全部可执行技术验证与具体可审阅清单，再请求最后缺少的用户信息或操作授权。
