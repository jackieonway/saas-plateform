# Phase 23 加速验证：验收与证据映射

记录日期：2026-10-29。范围为 [台账 Phase 23](../../unified-oauth-oidc-sso-task-ledger.md)；本文是并行只读核验结果，不是阶段通过报告，也不改变台账指针。实际环境、HBA 和最终演练由原任务环境 owner 独占。

后续接管更新：原任务聊天因 413／full-history replay 中断，加速协作根代理已接任唯一环境／HBA owner。[当前交接顶部](phase-23-execution-handoff.md) 已记录独立恢复代理的实际短 probe 全链 PASS：catalog 零差异、capture/capture-deny、隔离 restore-new、额外 ACL 拒绝与精确还原、reconcile、gate，随后 4 库、8 角色、43 私有文件及日志／备份、process／lease 清零，未改 HBA。该 probe 使用 `--workspace-ops --through-restore`，修复后的 core SHA 为 `608eefc20644778649a32a91c531d70c30723cde3555418f5f7b5871cc0e4baa`、inventory SHA 为 `5ca4449c7adbc794c642bf34eec584a4ffbbeb176be71980ee92df175f054a9f`；没有非空授权、rearm、旧载体或浏览器验收。这是接管交接证据，本协作没有重复运行。以下 r15 HTTPS 失败仍是最近完整动态结果，下一步冻结新工具并完整演练。

## 当前可确认的终点

[当前 HTTPS 记录](phase-23-integration.json) 的 `startedAt` 为 `2026-10-29T02:51:09.316Z`，总结果为 `FAIL`，失败阶段为 `CHECKPOINT_PREPARATION`，错误码为 `RECOVERY_REJECTED`。`checks` 只有 `PREFLIGHT`、`KEY_LIFECYCLE` 两项 `PASS`。因此可以保留已完成的密钥与源端证明，但不能据此完成 Phase 23。

| 台账验收项 | 当前可复核证据 | 尚需当前完整演练证明 |
|---|---|---|
| 发布、激活、窗口保留及紧急撤销完整校验 | 当前 HTTPS 的 K02/K03/K05/K06/K08；K03 两独立 identity 进程、实际 SDK/JWKS/UserInfo；K06 记录真实 `last_signed_exp + 60`，实际等待约 347 秒；K08 实际无 signer 503 和 D 恢复签发。配置、权限和事务补充见[密钥组件报告](phase-23-keys.md) | 最终冻结工具与应用输入对应；将所有 K01–K09 的组件与真实 HTTPS 层级明确映射。当前 `K07_CHECKPOINT_CARRIERS_READY` 为 `READY/PRECONDITION_ONLY`，不能当作恢复后 C/D 吊销验证已通过 |
| 并存窗口合法 token 可验，错误／已撤销密钥拒绝 | K03 `oldAStillValid=true`；K06 `oldKeyAbsentFromJwks=true`；双进程当前签发与 cached/fresh 验证均已记录 | 保留实际旧 C/D 载体，完成 checkpoint 后撤销与恢复后的当前权威拒绝；区别第三方缓存数学验签与系统在线拒绝，不将自然到期计为撤销 |
| 恢复后旧撤销事实不丢失，注销／失效来源不复活 | 恢复工具、witness、deny-only 和 invariant 组件存在；源端邀请消费、策略样本已准备；SOURCE 11 项服务用途正控制已记录 | 完成 capture、最新 deny witness、文件负测、隔离 restore、reconcile、rearm、gate、真实 ACL 漂移；验证原始未自然到期 USER/MACHINE/code/SSO/BFF/派生凭据、旧 refresh／秘密／口令拒绝；验证墓碑、receipt、代际、邀请终态及新双 RP 业务 |
| 实测恢复结果、目标差距和临时材料清理 | [r15 cleanup](phase-23-cleanup.json) `PASS`：2 源库、8 角色、49 个私有文件／99,896 bytes 已清，process/DB/role/lease 为 0；[HBA 恢复](phase-23-hba-restoration.json) 原字节与 stat 还原；Docker 前后均 0B | r15 未完成 checkpoint，尚无本轮完整 RPO/RTO；最后成功演练还须独立清理与残留核对。共享 PG、公开构建输入和专属编译 cache 仍保留，不能称全部任务资产已回收；没有执行 VHDX 压缩 |

SOURCE 11 项分组为 GET 5、context 2、access/proof 4，当前记录均 `source=true,target=false`。恢复后的新 secret 正控制与旧 secret 拒绝尚未执行，不能由 SQL 摘要或 rearm 成功替代。

## 在长密钥窗口之前完成的短门禁

1. 先由环境 owner 确认独占恢复 probe 已交还环境，且前轮最终清理/HBA 恢复已完成。协作代理只跑纯组件，不与 probe 或完整 driver 同时启停同一环境。
2. 对修复后的同一输入运行 `phase23-recovery-probe.py --snapshot <最终tools标签> --build-snapshot phase23-apps-r2 --through-restore`。它覆盖 prepare/start/stop-writers、adapter、init、真实 catalog 对照、capture、capture-deny、source quarantine、restore-new/reconcile/gate，并在 finally 执行两段 cleanup。**此命令创建任务专属数据库和进程，只能由原环境 owner 串行执行；本文没有运行。**
3. probe 的 workspace 修复模式只用于诊断；最终重跑使用冻结工具，核对 app 编译闭包与 apps-r2 的六个 binary attestation 一致。若编译闭包改变，则重新编译受影响应用；不能改写旧 attestation。
4. 将真实 catalog／restore／reconcile 的短失败先修完，再启动完整 HTTPS driver。无授权样本的短 probe 即使通过，也不覆盖非空 grant/同意/SSO/BFF/邀请／投递／代际恢复，不能代替最终动态验收。
5. 完整 driver 内保留实际 61 秒 SDK 预发布和 exp+60 退役等待；不清缓存、不篡改时钟或缩小数据库高水位。旧载体按逐项实际截止检查：canonical Access/ID 当前约 300 秒，旧 `OIDC_TOKEN_TTL_SECONDS=30/600` 参数不改变 canonical 有效期。

以上顺序减少的是“已知恢复工具错误后再次白等密钥窗口”，不会减少协议需要的真实缓存和退役窗口。

## 可并行纯检查

[当前纯组件记录](phase-23-current-components.json) 的 309 项通过绑定 `phase23-tools-r14`，分组为 recovery 109、Python harness 126、prepare 12、Node 62。它不是 r15 后修复的完整通过证明；修复后只重跑受影响组，最终冻结前再记录所用输入 SHA。

以下命令由 Linux POSIX Python 环境运行，须保持 `-B`，并先阅读对应 test 文件。现有测试使用 fake/临时文件和局部 Python 进程；不连接 PG、不启动应用或 Docker。原目录中不能用字面量 `test_*.py` 路径作为 PowerShell `rg` 搜索目标，应给目录及 `-g`。

```sh
python3 -B -m unittest discover -s saas-plateform/backend/ops/identity-recovery -p 'test_*.py'
python3 -B -m unittest discover -s saas-plateform/backend/tests/unified-auth-https -p 'test_phase23_*.py'
python3 -B -m unittest discover -s saas-plateform/backend/tests/unified-auth-https -p 'test_prepare_*.py'
```

本次协作已在 Windows Node `v24.22.1` 实际运行下列 5 个文件的 12 项纯测试，全部通过，持续约 254ms。读取对应 imports 和用例后确认只调用内存 fake，不创建 socket、进程、数据库或容器，不导出秘密。

```powershell
# cwd: saas-plateform/backend/tests/unified-auth-https
node --test --test-reporter=tap phase23-protocol-diagnostics.test.mjs phase23-dual-identity-cases.test.mjs phase23-service-get-cases.test.mjs phase23-service-context-cases.test.mjs phase23-service-access-cases.test.mjs
```

该 12 项覆盖固定错误码脱敏、双实例归属门禁、service 正反控制和 AEAD 绑定，只证明组件合同；没有新增动态 HTTP 通过证据。

## 交接中被覆盖的旧状态

[执行交接](phase-23-execution-handoff.md) 采用顶部新段覆盖后文旧段：

- 顶部 r15 已结束；后文“session 76438 正在运行”已过时，不能据它判断当前环境占用。
- 当前恢复阻断已到 checkpoint/capture；后文“尚未运行 browser”“只实现 4 个 RP secret”“11 项 HTTP 尚未实现”是旧进度。当前 SOURCE 11 项已实际通过，TARGET 11 项仍待恢复后实际对照。
- 当前复用应用为 apps-r2；后文 apps-r1 的 SHA、编译和集群标识只作历史。当前集群不能沿用旧 system identifier。
- Q9 `RPO≤15分钟/RTO≤4小时` 是内部候选比较目标，不是已批准正式承诺，也未被当前失败演练验证。

本协作仅新增本文。没有修改运行工具、台账或既有正式报告；没有创建或复用任何容器，没有新增数据库、日志、临时材料或清理范围。检查时仓库 HEAD：platform `34105757b040165daa5477bf6afff9637f617466`、identity `7025a0a39cff5577bc3aabf239aaae8df529714e`、portal `648e71e082929605c67e4e9f7e19fc9ecdbd89d4`；工作区已有大量未提交实现，HEAD 本身不能代替冻结输入摘要。
