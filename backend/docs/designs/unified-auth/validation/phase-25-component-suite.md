# Phase 25 当前候选的组件与数据库验证

本报告记录 2026-10-29 的新执行，不引入旧阶段测试结果。它证明相应冻结输入的组件、真实 PostgreSQL、角色或 TLS fixture 行为；实际多服务 HTTPS、浏览器、标准外部客户端、恢复及整个 Phase 25 的结论仍由主报告单独给出。

## 输入与结果

身份、平台、门户、通知及恢复组件使用 `phase25-apps-r2`，快照 SHA-256 为 `e1d4706a6ecb0cabfc988327a43d9e4d75e5cad3b9cac3ef204bfd1b2516ac1d`。门店与工单修订后完整重跑使用 `phase25-apps-r3`，SHA-256 为 `a078317efaaa94abeb8cf2d96c37077dee5d5eb438f64351e837f75982dbbe49`；[r3 构建与逐组件等价证明](phase-25-candidate-build-apps-r3.json)显示，生产输入仅两业务库的 `lib.rs` 改变，未变组件按其明确闭包组合。

Rust 1.97.0，非 root 用户，原锁文件、离线依赖及共享 Cargo cache。每份执行报告保留选中源输入、命令、测试名、实际统计、输出摘要和资源后查；未来修订候选仍须逐项目证明输入等价，不能直接把本报告当作改变后的版本通过。

| 组件 | 当前结果 | 证据与边界 |
| --- | --- | --- |
| 身份 | 142 PASS，0 ignored、0 failed | [执行报告](phase-25-identity-components.json)：lib 141 及 main 1，显式选择原 ignored PostgreSQL 测试；单位、局部 schema、canonical schema、真实 LOGIN 及 TLS fixture 分层保留 |
| 平台 | 432 个唯一测试名最终通过 | [分层合成](phase-25-platform-final-components.json)及[独立审查](phase-25-platform-component-aggregate-review.json)；不是单次全绿，具体历史见下节 |
| 客户门户 | 34 PASS，0 ignored、0 failed | [执行报告](phase-25-portal-components.json)，包括当前独立库回归；不代替实际跨库浏览器故障恢复 |
| 通知 | 39 PASS，0 ignored、0 failed | [执行报告](phase-25-notification-components.json)，含真实 PG / Socket.IO 与受控 HTTP authority；不代替实际平台与身份服务互通 |
| canonical 恢复工具 | 119 PASS，0 skipped / failed / error | [新组件报告](phase-25-recovery-components.json)：真实文件、加密和 CLI 保护，PG 被 mock；不是实际 restore 或 RPO/RTO |
| 门店 | 修后 r3 全部 98 PASS，0 ignored、0 failed | [执行报告](phase-25-stores-components.json)：85 lib、4 imported 启动保护、4 CLI 和5真实 PG 契约；普通 API 只读启动的实际 LOGIN 回归另由业务启动报告记录 |
| 工单 | 修后 r3 全部 149 PASS，0 ignored、0 failed | [执行报告](phase-25-orders-components.json)：79 lib 及 migration/读/写/成员/提醒/启动和 CLI 契约，真实 PG 均显式执行；实际跨服务链仍单列 |

[跨项目补证](phase-25-components-cross-input-supplement.json)核验 r2 全部 1,527 项输入，以及候选 CLI、notification 二进制和 Node runtime 摘要。该补证明确记录实际观测时点，没有伪装成早于执行的检查。初期 identity / platform 使用的旧 runner 不具备后来增加的完整中断回收能力，本次正常退出后的资源情况另由独立后查确认。

## 保留的失败与平台合成范围

身份初期 Cargo cache 配置不全的尝试未执行测试；随后一轮因 runner 漏传 Phase 20 / 22 / 23 的显式数据库变量，实际为 119 PASS / 22 FAIL。两份原报告保留为 [attempt 068e84a3a71b](phase-25-identity-components-attempt-068e84a3a71b.json)和[attempt 7a4100ae400c](phase-25-identity-components-attempt-7a4100ae400c.json)。补齐变量后重新执行完整 lib 和 main 得到当前 142 PASS，未导入此前阶段的结果。

平台第一次新执行的[原报告](phase-25-platform-components.json)为 408 PASS / 1 FAIL。唯一失败是 `phase12_customer_authority_real_tls_validates_ca_hostname_redirects_and_response_limits`：runner 没有提供 `IDENTITY_PHASE12_TLS_TESTS=TASK_OWNED`，fixture 在创建资源前拒绝。补齐明确 opt-in 后，[精确执行](phase-25-platform-selected-338b490efae8-components.json)为 **1 PASS / 431 filtered**，原三项测试可执行文件 SHA 与精确执行完全相同。

Cargo 在原 bin 失败后没有执行剩余 integration targets，因此分别新执行 `authorization_policy` 4、`context_selection` 9、`context_selection_database` 1、`migration_command` 7、`retention` 2，共 23 PASS。合成共 433 条实际执行行，最终覆盖 432 个唯一测试名；原 TLS 失败后精确成功是唯一重复。历史实际统计仍保留 failed 1 / filtered 431；不将其重写为一次 432/432 全绿。

## 可复现入口与工具审查

在已由协调方核对归属的 WSL task PostgreSQL 和快照存在时运行：

```sh
python3 -B saas-plateform/backend/tests/unified-auth-https/phase25-component-run.py identity --snapshot phase25-apps-r2
python3 -B saas-plateform/backend/tests/unified-auth-https/phase25-component-run.py portal --snapshot phase25-apps-r2
python3 -B saas-plateform/backend/tests/unified-auth-https/phase25-component-run.py notification --snapshot phase25-apps-r2
```

平台的完整选择同理使用 `platform`；精确回归追加 `--selector 完整测试名`，单个 integration 使用 `--test-target 目标名`。筛选报告单独落盘，`fullServiceSuiteClaimed=false`。门店与工单分别使用 `stores --snapshot phase25-apps-r3` 和 `orders --snapshot phase25-apps-r3`，完整新执行均使用 `--all-targets --include-ignored --test-threads=1`。实际命令和时间以各 JSON 为准。

[runner 独立审查](phase-25-component-runner-review.json)覆盖完整输入及辅助二进制绑定、白名单环境、独占临时目录、原进程组及 pidfd 回收；实际验证正常退出、父进程退出后孙进程存活、超时、SIGTERM，以及 Cargo 返回 0 但选中 0 测试时 runner 必须 FAIL。该预期负测另行标记，不计为服务测试失败。

[执行身份映射](phase-25-component-execution-map.json)将实际通过的测试名与矩阵 source ref 和源摘要相连。它只提供导航，保留 SMALL_SCHEMA_PG、canonical PG、LOGIN、mock port 和 HTTP fixture 的边界；“至少一个组件测试匹配”不等于整项场景 PASS。

## 资源与清理

参与本节验证的服务是身份、平台、门户、通知、门店和工单；恢复部分只有纯组件测试。创建及复用容器均为 0，没有创建第二套 PostgreSQL、修改 HBA、触碰 5432、现有线上服务或 root 的 HTTPS 运行库。数据库测试仅创建自己的 UUID 名 fixture，并按其实现结束清理；runner 不按全局差集删除任何数据库或角色。

[独立后查](phase-25-components-resource-audit.json)确认这些组件前缀的数据库与角色、runner 独占临时目录、旧 `/tmp` 已知密钥/CLI/upload/zip fixture 前缀及 runner 进程均为 0；r2 全部 1,527 项与 r3 全部 1,558 项输入仍一致。Docker context 为 `default`，后查 images / containers / volumes / build cache 均为 0B。共享任务 PostgreSQL、Cargo cache 和公开候选输入保留给后续验证，最终由协调方按归属清理；本节不宣称 Windows WSL 虚拟磁盘已经缩小。
