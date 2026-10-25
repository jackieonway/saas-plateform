# UUID-U02 验收记录

状态：COMPLETED。执行范围为源码、追加迁移及独占 PostgreSQL 17 测试库；现有本地 v4 预览三库未应用新迁移，尚未重编号或切流。

平台入口为 `util::uuid7`，门店为 `domain`，工单为 `uuid_v7`，通知为私有 `identifiers`。它们统一生成 v7 并校验 version 7／RFC variant；Path、DTO、可选过滤、平台上下文、会话／refresh、应用 token、普通 OAuth 用户身份及微前端 revision 均拒绝旧 UUID。机器客户端的 opaque `client_id`／`sub`、授权 code、refresh、PKCE、密码和安全 secret 保持各自合同。请求头仅沿用一个合法 v7，其余重新生成；通知 jitter 使用独立 CSPRNG 字节。

平台 44 处省略 UUID 的生产 INSERT 已显式传入 ID，包含 seed、批量通知、session、OAuth、成员 UPSERT 和 cleanup。测试准备 SQL 也显式发号；千行 fanout 和工单大数据 fixture 使用 Rust v7 数组／QueryBuilder，未用 SQL 位段或默认函数凑版本。明确的 v4、nil、错误 variant、历史 stage 和旧 schema 拒绝用例保留。

追加迁移如下，全部使用 PG17 已有的 `uuid_send/get_byte` 校验，不在数据库生成 v7：

| 数据库 | 新迁移 | UUID 列 | schema 合同 |
|---|---|---:|---|
| platform | `0012_uuid_v7.sql` | 107 | 移除 UUID 默认，追加 strict 检查函数和每列 CHECK |
| stores | `0003_uuid_v7.sql` | 23 | 验证 v2 后建立独立 v3 合同，保留 v1/v2 字节和 stored 摘要 |
| work_orders | `0004_uuid_v7.sql` | 17 | Rust／Node 的真实迁移历史、锁和 reference schema 一起扩到第 4 阶段 |

U01 的源清单和摘要保持冻结。门店目标 schema 新增 `_stores_schema_contract_v3`，后续目标 catalog／导入应把它识别为真实迁移创建的 metadata，不把源表行伪造成目标合同。

已完成实际验证：

| 范围 | 结果 |
|---|---|
| 平台 Linux 非 ignored | 149 单元 + 6 外部测试通过 |
| 平台 PG17 migration 专项 | 5/5，包括 107 列／0 UUID 默认、v4 源库拒绝且完整回滚 |
| 平台 scope | 3/3，自建库、真实迁移、v7 seed、清理 |
| 工单 Rust | PG17 142 项通过：62 unit、12 member、29 migration、13 read、12 reminder、14 write；Windows fmt/check/clippy/doc/release 通过 |
| 门店 Rust | 63 lib tests，PG17 postgres_contracts 6/6（全部 5 个 ignored 已执行）；Windows fmt/check/clippy/doc/release 通过 |
| 通知 | Windows/Linux 32/32，check/clippy/doc/fmt 通过 |
| artifact-agent | Windows 原有 1/1、Linux 3/3；Linux 真实 stage/install/verify/reuse/conflict 和 RENAME_NOREPLACE；clippy 通过 |
| deployment-contract / deployment-agent | 9/9 和 4/4，Clippy 通过；拒绝旧 job/release/nonce |
| platform-admin | Linux 50 项（23 unit、13 artifact、2 retention、12 safety）通过，fresh CLI 已构建供真实平台往返测试；Clippy 通过 |
| 平台前端 | 398/398，全量 typecheck/build/sdk:check/qiankun:check 通过；lint 0 errors、47 既有 warnings |
| SDK 维护源码 | 29/29，含同毫秒／回拨、唯一性、嵌套 typed ID、旧请求头拒绝 |
| 工单／门店前端 | 90/90、149/149，门店 TypeScript 通过 |
| 工单 Node 真实 PG | 62/62、0 skip，含迁移、scope、写入／并发、CSV、提醒幂等 |

平台 136 项 ignored 数据库用例均已有通过证据：完整运行首次 134 通过、2 失败，修正后两项分别定点重跑通过。第一项是 HEAD 实现已撤销普通 OAuth、测试却期望保留的过时基线断言，现明确验证重置后 revoked、userinfo 401 和旧 refresh invalid_grant，没有修改生产撤销 SQL；第二项是新增旧 schema 回归的 refresh fixture 格式问题，改用真实 `issue_refresh_in_tx` 后确实进入旧身份拒绝分支。未把首次完整运行描述为全绿。最终全包格式与 Clippy（`-D warnings`）通过。

平台 Windows 的三项 HTTP fixture 和 CLI 两项 HTTP fixture 曾超时，已在 Linux 全部实际通过，没有放宽断言或 timeout。[最终 Rust 扫描](uuid-u02-runtime-scan.json)显示运行生成器的 v4 命中为 0，仅剩 10 个明确负面／旧 schema 测试命中；16 个原始 migration 均未修改，三个追加迁移摘要已记录。

SDK `releases/generated`、相关 vendor 和版本化 UMD 的历史字节未修改。SDK 维护源码与已登记 2.0.0 的发行比较命令 `npm run check` 目前明确报告 artifact drift；新版本登记、发行和活动消费者同步由 U05 完成，此处不把该命令记录为通过，也不覆盖旧发行来消除差异。源码／生成隔离测试通过。

Rust／前端工具遵循既有锁定版本，UUID crate 开启 v7；JS 固定 `uuid@13.0.0`。通知的直接随机依赖复用锁中既有 `rand 0.10.3`。Windows JS 验证使用 Node 24.22.1；Docker Node PG 验证实际为 24.18.0，两者分开记录。PG17 容器为独占 `saas-uuid-u02-pg17-20261024`，测试只创建自己的 fixture 库，密码仅通过私有 env-file 传递；未连接远程或预览源库。

| Acceptance Criterion | Result | Evidence |
|---|---|---|
| 运行 UUID v7／RFC、并发唯一性与拒绝 nil/v4/坏 variant；随机安全用途不误改 | PASS | Barrier 8 线程×1000，Rust/JS 输入与请求头回归；jitter 独立随机 |
| PG17 真实历史到最新迁移、显式 seed/INSERT、v4 约束拒绝 | PASS | 平台升级专项、各服务迁移合同、自包含 scope seed、全部 DB 用例 |
| stage/jti/attempt 栅栏与有意历史负例 | PASS | Linux 真实安装／NOREPLACE、发布 DB 回归、jti version+variant、历史保原 |
| 所属构建及必要检查，固定 SDK 历史不改 | PASS | 上表实际源码验证；SDK 发行登记比较明确留作 U05 发布门禁，旧字节保持原样 |

下一阶段 U03 实施协调导出、一次性权威映射和离线变换器。SDK 发行与消费者发布仍待 U05，不能把源码完成当作预览已切换。
