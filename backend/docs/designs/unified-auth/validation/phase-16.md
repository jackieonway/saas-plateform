# Phase 16 — 门店 API 统一资源接入

日期：2026-10-28。状态：COMPLETED，提交主代理按台账统一验收。

## 范围与源码

本阶段在 Phase 15 正式验收后实施，只修改独立 `saas-stores` 仓库的后端授权、配置、对应测试及当前文档，并生成本报告；没有修改工单、平台 schema 或门店结构基线。Phase 15 的平台资源合同与已验收输入见[资源能力合同](../phase-15-resource-capability-contract.md)和[Phase 15 报告](phase-15.md)。平台冻结输入 SHA-256 为 `605ac9ef8aba39fbeed7037287380559bab9a0c075bbc61a49362f71fc1d87f3`。

门店基础 HEAD 为 `01ea4eb30801e89429b4a4f1469cc0792f17b1f9`，开始时工作区干净。本阶段未提交 Git。最终 19 个修改文件的内容清单摘要为 `78da0ed96e5a5c100c88129c749c514e34610cfeff284b90e5d926878591c6c4`；计算方式为：仓库相对路径按 Ordinal 排序，每行写入 `path + 空格 + 文件原始字节的 SHA256 小写十六进制`，以 LF 分隔并保留最后 LF，再对整段 UTF-8 字节计算 SHA-256。该摘要包含配置示例和当前文档，报告本身不在门店仓库中。

## 最终行为

- 生产授权只调用 Basic 认证的 `POST /api/apps/stores/authorize`，Form body 仅传 token、业务 resource、action。`PLATFORM_AUTHORIZATION_MODE` 默认且仅允许 `combined`，旧值启动即拒绝，没有运行 fallback。
- 授权外层要求 `resource-authorization/1`、USER、准确 stores 受众／canonical resource／动作、准确目标应用，以及相互一致的 issuer、tokenUse 和 introspection。内部 `app_session` 使用固定 `urn:saas:platform:internal-capabilities`、client_id=stores、scope=app:access；外部 `access_token` 准确匹配独立 `PLATFORM_IDENTITY_ISSUER`。
- 内部账号、成员、租户和组织由严格 USER principal 提供，UUID 均为 v7，账号／成员版本均为规范正 i64 十进制字符串。公共 `introspection.sub` 保持不透明，不能替代内部 userId。principal、context、目标应用及独立 dataGrant 绑定不一致即拒绝。
- `PLATFORM_RESOURCE_SCOPE_BINDINGS` 是明确的 `resource:action → 单个已审核 OAuth scope` JSON 对象。解析拒绝重复 JSON 键、未知操作、非字符串、非法 scope 和错误 shape；外部 token 必须含当前操作的准确映射 scope，缺项失败关闭。配置与平台 `platform_resource_bindings`、身份资源登记使用同一审核来源，服务不猜测注册名；内部能力仍独立要求操作权限和数据授权。
- 成员与组织目录改为 Basic 认证的私有 Form POST：`/internal/platform/apps/stores/members[/userId]`、`/organizations[/organizationId]`。token、组织、分页和搜索不进入 URL。响应要求准确的 `resource-directory/1`、目录名及最多三秒的外层截止，并与原 1.0 数据的截止取较早值。
- 保留有效 STORE、租户、组织硬边界、SELF owner、独立数据授权、精确金额、幂等／唯一性、乐观版本和审计事务。组织／成员目录在租用本地业务连接或持有行锁前取得，原短期证据不会续期。
- 资料与日报四个写入口在本地 SQL 和审计之后、COMMIT 之前重新在线验证同一来源、成员代际、客户端／应用、contextVersion、scope 和数据授权。来源撤销、漂移或不可用会回滚业务和审计。提交前后同时检查原证据与最新授权截止，已得知的更早截止不会被旧证据覆盖；已提交但截止在 COMMIT 期间耗尽时报告结果未知。
- MACHINE 明确返回 `403 SUBJECT_MODE_DENIED`。现合同没有为机器提供本服务必需的 STORE 类型证明和人员审计模型，本阶段未批准机器业务范围，也没有构造虚假的用户或员工。

主要实现位于[授权链](../../../../../../saas-stores/backend/src/authorization.rs)、[HTTP 客户端](../../../../../../saas-stores/backend/src/authorization/client.rs)、[身份和数据策略](../../../../../../saas-stores/backend/src/authorization/policy.rs)与[真实数据库业务测试](../../../../../../saas-stores/backend/tests/postgres_contracts.rs)。当前部署说明见[门店 README](../../../../../../saas-stores/backend/README.md)和[组织／成员合同](../../../../../../saas-stores/backend/docs/member-contract.md)。

## 验证结果

验证使用 Windows Rust 1.97 工具链；本机 Rust 入站限制通过现有测试专用 `STORES_TEST_NODE` 监听 fixture 处理，生产 Rust HTTP 客户端与原有请求／响应断言仍实际执行。测试和脚本均没有容器创建或 Docker fallback。

| 检查 | 最终结果 | 证据范围 |
| --- | --- | --- |
| `cargo fmt --all -- --check` | PASS | 当前门店后端 |
| `cargo check --locked --all-targets` | PASS | 最终代码全部 target |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS | 无 warning |
| `cargo test --locked --all-targets` | PASS：92 个普通测试；4 个数据库测试按设计 ignored | lib 83、启动拒绝 4、迁移 CLI 4、数据库归属门禁 1；ignored 不算作通过 |
| `cargo test --locked --test postgres_contracts -- --include-ignored --nocapture --test-threads=1` | PASS：5/5，0 ignored | 四个显式隔离数据库用例和一个归属门禁均实际运行 |
| `git diff --check` 与本报告／修改文档的相对链接核对 | PASS | 仅当前源码与当前文档链接 |

实际 PostgreSQL 用例覆盖：

1. 营业资料、日报、汇总、成员候选和联系人创建／更新的真实 SQL 与 HTTP 请求；七个组织用途、三个成员用途、同账号员工／客户隔离、停用成员、保留失效联系人拒绝及显式清空成功。
2. 20 次 GET／PATCH 负矩阵：错误 tokenUse、受众、资源、scope、principal、context、组织，停用成员、MACHINE 和来源不可用。每次均确认目录没有被越权调用，业务／版本／审计快照逐值不变。
3. 同一数据库中缺少 `store:update` scope 映射的独立配置：PATCH 在目录和业务写入前返回 401，所有业务／审计数据不变。
4. 第一次授权成功、SQL 已执行但第二次授权撤销或不可用：用连接池状态证明写事务仍持有连接，分别返回 401／503，整笔业务与审计回滚。
5. 非 STORE／停用组织的历史行排除、SELF 本人子组织行、非本人行隔离，以及目录不占用本地业务连接／行锁的原约束。
6. 通过 `pg_stat_activity` 和 `pg_blocking_pids` 证明实际 FOR UPDATE 锁等待；开始等待时剩余组织证据约 962ms，到期后释放锁，要求拒绝且营业字段、版本、updated_at、审计不变。
7. 新库和重复启动、并发日报唯一性、精确大额汇总、不可变归属、复合门店外键、乐观版本、审计原子回滚与 append-only、结构漂移／checksum 拒绝、未追踪库不得接管。

最后一轮显式数据库套件耗时 5.95 秒。最终四个随机库为：

```text
phase26_stores_sql_01a1813274157301b4524b63abb97fb7
phase26_stores_sql_01a18132791270d5802a8a537ff5406e
phase26_stores_sql_01a181328440774ea1d46cad8d6857dd
phase26_stores_sql_01a181328a0771418d233467a3e13a04
```

`phase26_stores_sql_` 是既有测试 helper 的资源前缀；这里的证据来自本次执行，不引用旧 Phase 26 验证。每个库均输出 `assertions_ok=true database_removed=true pool_closed=true`。

## 资源与清理

实际参与项目为门店后端；创建容器 0，复用容器 0。复用主任务自有原生 PostgreSQL `127.0.0.1:55494`，执行前后核对 cluster system identifier `7701548153213487841`，管理员 URL 明确选择 `/postgres`。测试另外要求 `CI_TEST_POSTGRES_OWNER` 与 `STORES_TEST_LOCAL_POSTGRES_OWNER=<owner>@127.0.0.1:55494` 准确相等，拒绝默认 5432、无归属回环地址和其他服务器。

两轮显式数据库套件共创建 8 个随机门店测试库，全部删除；没有新建角色、持久卷、网络、镜像或导出日志。清理后 `phase26_stores_sql_%` 数据库数为 0，逻辑临时数据库占用为 0 字节，与执行前均为 0。共享 PostgreSQL 保留供其他阶段使用，不停止其他阶段的服务、数据库或 namespace。这里不声称共享集群 WAL、WSL 虚拟磁盘或 Windows 物理空间已压缩回收。

## 证据边界与后续

本阶段验证的是独立门店 API 的生产 HTTP 客户端、受控平台协议 fixture 和真实独立 PostgreSQL 读写。平台来源、scope 签发与原始 canonical token 验证由已验收的 Phase 15 合同负责；本阶段没有把 fixture 响应当作实际 OAuth 签发或跨服务 TLS 部署证明。当前任务网关曾因 private origin 不路由 authorize 而无法在单个 origin 上完成协议，主代理已接管精确私有 POST 路由修复；实际全版本 HTTPS 互通由 Phase 25 验证。本阶段部署文档明确私有 origin 必须同时服务授权与私有目录。

组织／成员短窗口与跨服务来源复验是授权检查点，不是跨数据库原子事务；本地写入保留精确 SQL 过滤和提交前后的截止检查。机器业务开放仍需独立 STORE 证明与审计设计。

旧 `policy::validate_intro` 和 `client::{introspect,context}` 仅在 `cfg(test)` 编译，用于原有 HTTP 限额／重定向／错误消毒与策略回归 fixture；生产没有旧协议路径。Phase 24 应连同这些测试适配器清理，不能把它们当作 canonical 接入证据。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 合法 STORE 身份完成授权范围内的资料／日报请求 | PASS | USER principal + STORE 目录 + 实际资料／日报／汇总读写 |
| 错资源、错组织、停用成员、机器冒充人和来源不可用被拒绝 | PASS | 20 次负矩阵、scope 缺项拒绝、提交前撤销／503 回滚 |
| 业务数据边界与现有功能回归通过 | PASS | SELF／租户／组织／owner、联系人、金额、唯一性、版本、审计及真实锁等待 |
| 仅使用专属测试数据并完成资源清理 | PASS | 精确集群归属、8 个随机新库删除、剩余测试库／逻辑占用均为 0 |

## Suggested Task Ledger Update

- Phase 16：COMPLETED。
- Output Summary：门店 canonical-only 资源客户端、严格 USER principal／issuer／scope、私有目录与原三秒 STORE 证明、写提交复验和最新截止传播；当前文档及本报告。
- Completed Items：全部四项阶段验收通过，普通与显式 PostgreSQL 门禁通过，临时资源清理完成。
- Remaining Items：本阶段无；当前全版本跨服务 HTTPS 验证属于 Phase 25，机器业务未获本阶段授权。
- New Decisions：显式 scope 映射来自同一审核配置源；缺映射外部操作失败关闭。MACHINE 返回 403，不能伪造人员。最新来源截止与原证据同时约束提交。
- New Open Questions：本阶段无。
- Next Recommended Phase：Phase 17（工单），由主代理协调已有并行任务；本代理不修改台账。
