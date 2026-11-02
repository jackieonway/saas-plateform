# Phase 23：密钥生命周期与隔离备份恢复

状态：**COMPLETED**。日期：2026-10-29。依赖 [Phase 22](phase-22.md) 已验收。本阶段在任务专属原生环境验证 identity、平台、客户门户和独立标准 OIDC RP；没有操作正式数据库或执行发布。

## 1. 验收结论与证据范围

| Acceptance Criterion | Result | Notes |
|---|---|---|
| 发布、激活、窗口保留及紧急撤销有完整校验 | PASS | r19 的实际完整密钥套件覆盖预发布、原 operation 恢复、双实例签发、真实高水位退役及无活动签名者 503；r21 另验证 C 紧急撤销及恢复后 D 继续服务 |
| 并存密钥窗口内合法 token 可验证，错误／已撤销密钥拒绝 | PASS | 真实标准 SDK 保留同一进程和正常缓存，A/B 并存验证；真实发布窗口均超过 61 秒；新权威拒绝 C。错误材料由apps-r3当前普通lib配置测试拒绝，不冒称HTTPS启动故障；缓存公钥数学验签边界明确 |
| 恢复后旧撤销事实不丢失，已注销／失效来源不复活 | PASS | r21 实际加密双库备份、最新 deny 见证、隔离恢复、reconcile/rearm/gate、旧原始载体拒绝、只读不变量及新业务全部完成；邀请原结果保持，旧邀请不创建账号 |
| 实测恢复结果和目标差距明确，fixture 私钥与备份临时资源清理 | PASS | RPO 26.463 秒、RTO 98.942 秒；4 库、8 角色、78 个私有文件及进程／lease 均清理；HBA 原字节和 stat 恢复。共享任务资源单独列明 |

阶段采用可追溯的分组证据，并非宣称一次驱动执行了所有场景：

- [r19 实际记录](phase-23-r19-integration.json)使用 `phase23-tools-r19` 与 `phase23-apps-r3`，`KEY_LIFECYCLE=PASS`。该轮整体 **FAIL** 于后续 `INVITATION_ACCEPTANCE_FAILED`，仅其已完成密钥用例作为本阶段密钥证据；`K07_CHECKPOINT_CARRIERS_READY` 是 `READY / PRECONDITION_ONLY`，不计恢复通过。
- [r21 最终实际记录](phase-23-integration.json)使用 `phase23-tools-r21` 与相同的 `phase23-apps-r3`，**8/8 PASS**，外层驱动退出码 0。显式 `validationProfile=recovery`、`keyLifecycleExecuted=false`；本轮没有重跑 A 退役／B 生命周期／无活动签名者套件。
- [r21 文件负向实测](phase-23-file-negative-probe-r21.json)是独立的真实认证文件加载器与只读 PG 预检，不执行 restore SQL。最终驱动核验其原始字节摘要、结果相等及六个当前 loader 摘要后引用 [证据包](phase-23-file-negative-evidence.json)，未跳过实际恢复认证、ACL 漂移或 gate。
- [387 项冻结组件测试](phase-23-acceleration-components.json)与 [真实 runtime LOGIN-role 同意撤销回归](phase-23-consent-regression.json)分别作为组件和数据库证据，不冒充 HTTPS 演练。

## 2. 输入、构建及复现

三仓 HEAD 分别为 identity `7025a0a39cff5577bc3aabf239aaae8df529714e`、平台 `34105757b040165daa5477bf6afff9637f617466`、门户 `648e71e082929605c67e4e9f7e19fc9ecdbd89d4`。均存在先前阶段未提交变更，HEAD 不能代表实际验证源码。实际依据为 [冻结记录](phase-23-current-freeze.json)、[完整应用构建 attestation](phase-23-current-build-attestation.json)和[前端构建记录](phase-23-current-frontend-build.json)。

| 输入 | SHA-256／范围 |
|---|---|
| apps-r3 | `51b024e351c3ec842b30a6588a6f37ff5647afb3d989474e4bdd8420ba99fe1e` |
| tools-r19 | `356299b6e3062928f802633c798480a1dc6ad45d0772a3c3584bc06fcec79242`，对应 [组件输入记录](phase-23-r19-components.json) |
| tools-r21 | `0b7fee81ac40653f82e57da6447a416190e35b8504d266816db63da95900528f`，941 个输入，844 个编译输入与 apps-r3 一致，6 个 binary 摘要核验 |
| identity binary | `765d7ed7257da4bf90a5b740284c1064b6a09c0e84b6d7c68ec41186d710a291` |
| r21 文件负向原报告 | `376cff1a695b3ae66452c1414617975ef8b6ab7b6978b8045f9d94113017e3ce` |

apps-r3 完整构建了 identity、平台、门户、两个 migrator 和 SCRAM probe。相对上一冻结应用输入，重新纳入实际修复的 `consents.rs`、`keys/lifecycle_tests.rs` 和 `keys/pg_fixture.rs`。工具快照不冒充重新编译。执行期间工作区前端另有独立变更，未覆盖这些改动；本阶段只验证 attestation 中的冻结前端，Phase 25 必须针对当时完整当前源码重新构建和验收。

环境：Rust/Cargo 1.97.0，Node 24.22.1，Python 3.14.4，PostgreSQL 工具 18.6，Chromium 156.0.8078.4。任务根 `/tmp/rust-saas-unified-auth-20261028`，PG 端口 55494、system identifier `7701881991704683791`。本轮没有沿用旧集群标识。

最终实际命令（工作区根 PowerShell）：

```powershell
& ./saas-plateform/backend/tests/unified-auth-https/phase23-execute.ps1 `
  -Snapshot phase23-tools-r21 -BuildSnapshot phase23-apps-r3 `
  -ValidationProfile recovery `
  -FileNegativeEvidence ./saas-plateform/backend/docs/designs/unified-auth/validation/phase-23-file-negative-evidence.json
```

完整密钥组由 r19 使用相同驱动、`-Snapshot phase23-tools-r19 -BuildSnapshot phase23-apps-r3` 的默认 `full` 模式实际执行。命令依赖既有归属验证、冻结快照、受保护 TLS 材料及任务 runtime；不应直接对任意库重放。当前流程见 [HTTPS 驱动说明](../../../../tests/unified-auth-https/README.md)、[密钥轮换手册](../../../../../../saas-identity/docs/signing-key-rotation.md)及[恢复手册](../../../../ops/identity-recovery/README.md)。

## 3. 密钥生命周期

r19 真实浏览器先注册并确认 TOTP，敏感写入复核当前 console BFF 绑定的精确 canonical 来源，未通过 SQL 预种正向 MFA。时间较长时再次完成密码＋TOTP step-up。

| 场景 | 实际结果 |
|---|---|
| K02 预发布 | 未发布 B 激活 409；发布后 JWKS 含 A/B，仍由 A 签发；实际预发布 61.264 秒，同一 SDK 缓存年龄 65 秒，无缓存改写 |
| K03 激活与原结果 | 忽略应用回包后查询原 operation 得到已提交结果；B 唯一活动，ID Token／logout 使用 B，未过期 A 仍可验证，opaque subject 不变 |
| 双实例 | 两个独立 PID、同一 identity binary／keyset，实际 token upstream 23441/23442 各完成兑换，均经权威 JWT、缓存签名及 UserInfo 200 验证 |
| K05 保留窗口 | 活动 key 与仍在窗口内的旧 key 正常退役均 409，高水位实际读取 |
| K06 实时退役 | A 的 `last_signed_exp=1793248242.819181`，保留至 `1793248302.819181`；实际 DB 时间 `1793248303.124377` 后退役 200，A 退出 JWKS。未调小时钟或高水位 |
| K08 无签名者 | B 撤销、C/D 已发布而均不活动时，真实人员 token 签发 503；实际预发布 61.336 秒后 D 激活并由标准 SDK 正常验证 |
| 紧急撤销 C | r21 在 checkpoint 后实际撤销 C；当前权威拒绝 C、D 仍合法；opaque grant 在恢复前仍有效，证明签名 key 撤销不等于自动撤销所有 opaque grant |

r21 恢复准备另实际等待 C/D 预发布 **61.557 秒**并完成两次 SDK 签发。恢复后的 C 吊销事实及 D 唯一活动状态由精确不变量复核；该准备组不替代上述完整套件。旧缓存 RSA 公钥仍可数学验签，不能承诺第三方本地缓存瞬间撤销；本系统当前权威及在线来源检查的拒绝单独验证。

配置格式、匹配私钥、终态不复活、签名事务锁／MAX 高水位、审计与 receipt 原子性、权限和失败窗口的补充测试见 [密钥组件报告](phase-23-keys.md)。该报告明确保留其当时输入、旧集群、fixture MFA 及 Windows socket 失败范围，不将旧报告当作本次完整应用构建证明。

错误材料另由apps-r3当前普通lib **88 PASS／0 filtered**中的 `phase23_keyset_rejects_invalid_configuration_and_mismatched_private_material` 直接调用真实 `KeySet::load` 验证，不需PG且无ignore。`keys.rs`、`config_tests.rs`、`test_keys.rs` 的当前字节、apps-r3输入、attestation与上述组件报告摘要相同；覆盖不匹配私钥、错误指数、非法／过大PEM、非RSA及重复kid。这是当前配置组件通过，未称错误材料的真实HTTPS进程启动通过。

## 4. 备份、拒绝事实与实际恢复

当前 typed inventory 为共享库 86 表、门户 5 表、953 字段。源 checkpoint 在三服务／worker 停止且无源客户端连接后顺序 `pg_dump` 两库，随后确实恢复源业务，再发生实际故障；不宣称跨库原子快照。

dump、私有材料及原始 delta 用独立随机 nonce 的 AES-256-GCM 加密，AAD 绑定任务、集群、两源库、issuer 与材料用途。checkpoint／floor／start receipt 及 witness 使用分离的 HMAC 密钥；恢复集外独立 minimum-sequence=2 阻止整组旧状态回放。单机 HMAC 链不冒充异地不可变日志。

最终八阶段依次为：PREFLIGHT、RECOVERY_KEY_PREPARATION、CHECKPOINT_PREPARATION、POST_CHECKPOINT_DENY、DISASTER_CUT、OLD_RAW_CARRIERS_REJECTED、RECOVERED_OLD_INVARIANTS、FRESH_RECOVERED_BUSINESS，全部 PASS。

- checkpoint 后真实产生 C 撤销、refresh 重用拒绝、两个 BFF logout、标准 RP logout receipt、offline consent 撤销和展示删除，以及邀请撤销、凭据轮换／撤销、scope 收缩、成员退休等拒绝事实。
- 源保持隔离；恢复创建两个新目标库，保留原 deployment、owner、grantor、ACL、schema／SQLx checksum、函数、触发器及索引，不能用重新 provision 掩盖漂移。实际撤去目标 CONNECT 后 gate 返回 `RECOVERY_DATABASE_ACL_CHANGED`，恢复原有效 ACL 后才继续。
- SOURCE 新连接 SCRAM 正／错对照 **6/6**；RESTORE 三 runtime 的旧／新／错密码 **9/9**，旧及错误密码均 `28P01`，新密码成功。只在任务精确库／角色、127.0.0.2/32 上安装两条临时规则；不声称共享集群全部强制 SCRAM。
- 同一协议进程保留并重交原始载体：4 个 USER access、2 个 refresh、1 个 MACHINE、4 个 client secret、4 个主密码、1 个未消费 code、2 个 BFF Cookie，以及原 SSO／业务 bearer 均拒绝。原 DB 截止在每次拒绝前后检查，access 最小剩余 **176.590 秒**；canonical code 固定 60 秒，在拒绝后仍有 **5.039 秒**，检查先于平台／门户连接池启动。旧 OIDC TTL 配置没有延长 canonical 期限。
- 消费邀请的原 operation 恢复 **200**，原不可变 tuple、1 receipt、1 account 保留，account 仍 DISABLED、业务权限新增 0；同一旧 consumed/revoked token 使用新 operation 均 **409**，新增账号 0。历史邀请 key 仅保留原摘要验证用途，新 activeVersion 独立生成。
- 重启前只读不变量：211 个 immutable facts、192 个 old carrier rows、446 个 version floors、47 个 policy rows、18 个 quarantined principals、5 个 closed consents、9 个 delivery rows、8 个 checkpoint 后缺失行和 1 个 GC 行均符合严格预期。
- 然后实际完成新员工、新客户、客户组织选择、平台／门户业务、标准 SDK 登录＋UserInfo＋稳定 opaque subject、MACHINE 资源调用。11 个私有 service secret 均有恢复前成功对照及恢复后旧拒绝／新成功：5 GET＋4 access proof／profile＋2 context selection/result。详见 [逐服务矩阵](phase-23-service-secret-http-todo.md)。

九项独立文件负向覆盖密文／manifest 篡改、错误加密 key、混源绑定、真实旧 floor/CURRENT/record、缺链与额外链。每项前后源连接和目标库均 0，原字节精确还原。此组不宣称验证了磁盘耗尽或所有 restore SQL 故障。

## 5. RPO、RTO 与目标差距

| 观察点 | UTC／实测 |
|---|---|
| checkpoint 停写开始／结束 | 05:34:50.665Z／05:34:59.974Z |
| 实际故障开始 | 05:35:17.128Z |
| 完整新业务验证完成 | 05:36:56.070Z |
| RPO | **26,463 ms**，保守从 checkpoint 停写开始计龄；marker A 恢复、checkpoint 后 API 写入的 B 丢失，变化 marker 仍 DISABLED |
| RTO | **98,942 ms**，单调时钟计时，另计 UTC wall clock 同为 98,942 ms；包含隔离、恢复、对账、轮换、旧拒绝及完整新业务 |
| 内部候选比较 | RPO ≤15 分钟、RTO ≤4 小时；本次均满足，仍为 INTERNAL_CANDIDATE |

业务 marker 使用没有成员的专属子组织，和客户正向选择组织分离。主动失效全部旧认证载体属于安全隔离，不记作业务数据 RPO。恢复 cycle 仅 10,273 ms，状态为 `GATE_PASSED_QUARANTINED / opensTraffic=false / completeRtoClaim=false`；该局部耗时不替代完整 RTO。

实际恢复步骤另记录：停写 925 ms、最新 deny capture 2,091 ms、源隔离 498 ms、restore/reconcile/rearm/gate 11,235 ms、实际 ACL 负测 4,040 ms、DB 密码轮换 499 ms、新连接 SCRAM 1,076 ms、目标 identity 双实例启动 1,021 ms、业务池启动 2,542 ms。剩余时间包括协议拒绝、只读验证与新业务；步骤之和不冒充完整耗时。

这是单机小型 fixture，未验证持续 WAL、异地恢复、自动备份频率、生产数据规模或值班响应。Q9 的容量／正式 SLO、人员和发布事实仍为后续门禁，不能从本次耗时推导 99.9% 或对外承诺。

## 6. 修复与组件回归

1. 同意命令读取 append-only receipt 时误用 `FOR UPDATE`，真实 runtime 无 UPDATE 权限而报 500。改为原 advisory lock 下的普通 SELECT，不扩 ACL；实际 LOGIN-role 回归验证并发原 operation 重放、唯一 receipt、撤销和展示删除。
2. 恢复适配器原先丢弃历史邀请 key，导致原 activation receipt 摘要无法验证。改为保留已认证历史验证材料并增加独立新活动版本，16 项上限在输出前检查；不恢复旧邀请能力。
3. 客户端恢复 policy version 推进而选择登记仍保留旧版本，导致新业务无法选择。对未变且显式选择的 eligible client/consumer 精确推进登记，并验证 application 历史 floor；未选、缩权或绑定漂移继续禁用，gate 和只读不变量核对全表。
4. 修复 typed inventory 漏字段及等价 owner 默认 ACL 比较；额外授权仍拒绝。补充 carrier 实际截止门禁、RPO marker 隔离、短 code 检查顺序、有界无秘密交互诊断及绑定原报告的文件负向证据。

| 最终组件组 | 通过／跳过 |
|---|---|
| recovery Python | 119／0 |
| harness Python | 143／0 |
| prepare Python | 12／0 |
| Node | 96／0 |
| fixture／collector 自测 | 12＋5／0 |
| 总计 | **387／0**，全部 exit 0，输入测试前后摘要相同 |
| apps-r3 Rust 普通 lib | **88 PASS**，58 个需要显式 PG 的测试 ignored，不计通过 |
| apps-r3 实际 runtime receipt PG 回归 | **1/1 PASS**，临时 1 DB／6 roles 清理 |
| apps-r3 strict Clippy | `--all-targets --all-features -- -D warnings` PASS |

组件的精确命令／输入摘要见相应 JSON，未对冻结之后独立变化的整个当前工作区重跑所有 Cargo／前端门禁。阶段范围内实测结果和 Phase 25 当前全版本验收分别保留。

## 7. 保管、操作权限及清理

签名私钥由外部文件在启动时加载，数据库仅存公钥及生命周期。fixture 使用私有目录0700／文件0600；恢复管理员、runtime、BFF／context／TOTP／邀请／服务／RP／机器秘密分离，不将私钥、token、Cookie、PHC、连接串或原始回调写入正式报告。TLS 材料仅做受保护备份一致性检查，没有 CA 私钥，不宣称 TLS 密钥轮换。

运行身份不具恢复管理员权限。密钥管理须当前根 EMPLOYEE、实际细权限、来源／CSRF 及最近五分钟真实 TOTP；未知提交查询原 operation。恢复工具只接受本次精确 task/cluster/database 绑定；新环境开流、数据库密码及 HBA 由唯一环境 owner 执行。正式恢复仍需明确资产、可信人员、外部控制日志和对应任务授权。

[最终清理](phase-23-cleanup.json)为 PASS：4 DB、8 roles、78 个被逐项记账的私有文件 **161,473 bytes** 删除；备份／witness等本次专属目录另递归清除，其内容不全部计入该字节数。私钥、秘密、加密备份、运行日志及临时目录均无残留，owned process／DB／role／lease 均 0，错误 0。[HBA 恢复](phase-23-hba-restoration.json)核对原字节及 stat，原／最终 SHA256 均为 `e6305f548fd8096217d02a8f93b5c50716d58df1d2441b12346dfe57db7027fd`，外层另删除 root journal／receipt。独立终核见 [资源审计](phase-23-final-resource-audit.json)。

协议／owner 分别扫描 15 个日志，内存比较115／269个候选，发现均0；最终清理再次扫描14个日志、105个候选，发现0。固定报告不保留原秘密。端到端 TLS 均为 TLS 1.3、没有忽略证书错误、没有用 HTTP issuer fixture 或 SQL 正向 MFA 冒充实测。

实际参与服务为 identity、平台、门户和标准 RP；**Docker 创建0、复用0**，context `default` 的 images／containers／volumes／build cache 前后均0B。清理前4 DB逻辑尺寸合计53,558,012 bytes；该数不是物理磁盘回收量。`/tmp` 为 tmpfs，清理前用量1,445,126,144、后1,445,273,600 bytes，因共享任务仍运行，净变化 **+147,456 bytes**，不虚报空间下降。

共享任务 PostgreSQL、公开冻结输入／构建产物、Node 运行时／RP依赖和任务专属 Rust 缓存为后续台账阶段保留，未停止原业务服务、未操作5432或原443。缓存链接目标 `/home/jackie/.cache/rust-saas-unified-auth-20261029/target-linux` 必须在整体任务结束按 OWNER 定向清理，不能只删 `/tmp` symlink。没有执行全局 prune、停止 WSL 或 VHDX 压缩；本报告不声称 Windows 磁盘已完全回收。

独立终核之后另按OWNER、绝对父路径、非symlink目录及manifest摘要定向删除不再使用的tools-r16/r17/r18/r20四套公开工具副本与对应manifest，保留密钥证据所需r19及最终r21/apps-r3，正式证据与工作区源码未删。见[过期快照清理](phase-23-snapshot-cleanup.json)：目录分配133,726,208 bytes、manifest逻辑488,814 bytes；该清理窗口`/tmp`实际用量由1,445,138,432降至1,310,916,608 bytes，减少**134,221,824 bytes**。这属于tmpfs用量变化，不是Windows VHDX回收。其余保留资源的终核时点占用见资源审计，外部target-linux分配3,393,560,576 bytes，仍未清理。

## 8. Task Ledger Update

独立终审确认四项均可PASS：r19/r21/apps-r3三份manifest的844个编译输入键集及逐文件字节一致，当前六个bin与构建attestation一致，文件负向原始report及六个loader摘要一致；没有发现动态技术阻断。最终六份相关文档共110个相对链接均可解析，本次相关Rust与文档的`git diff --check`通过。此后的修改仅为结果、台账和操作说明，不改变已冻结的恢复代码或应用binary。

按台账全局约束中“完成阶段后同步摘要、详情、指针、决策、问题和变更日志”的既有授权执行：Phase 23 → COMPLETED，四项验收勾选；下一指针 Phase 24 → NOT_STARTED，依赖已满足。Q3／Q8／Q9／Q10 的运营、发布及正式环境事实仍待后续核实，没有新政策决策或新增阻断。Phase 24 退役、Phase 25 当前全版本矩阵及 Phase 26 获授权切换未在本阶段执行。
