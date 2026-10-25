# UUID 类型化引用与保留合同

本文是 UUID-U01 的导出合同，机器可读来源为 [uuid-references.json](../../ops/recovery/uuid-references.json)。它与 [UUID 审查](uuid-v7-audit.md)、[UUID 台账](uuid-v7-task-ledger.md)配套。读取注册表不产生 ID、不改数据库，也不授权访问远程或生产环境；本次实际执行边界仍是明确登记的本地预览三库。

## 1. 注册表覆盖与格式

注册表针对当前活动迁移定义的 schema，不提前加入员工/客户或 SSO 尚未实施的表。三个 `databases` 条目使用 `kind=platform/stores/work_orders`；`columns` 的每项由 `schema/table/column` 精确定位，包含 `authority`、`role`、`nullable`、`retention`、源码证据和数据库外键组。`tables` 为每张实际表指定保留策略，包括没有 UUID 的表及迁移元数据。

| 数据库 kind | UUID 列 | 表（含元数据） | 外键组 |
| --- | ---: | ---: | ---: |
| platform | 107 | 40 | 64 |
| stores | 23 | 7 | 2 |
| work_orders | 17 | 6 | 1 |
| 合计 | 147 | 53 | 67 |

平台 64 组 FK 中有 1 组是 `regions.version → region_versions.version`，并非 UUID；仍登记在 `foreignKeys`。组合 FK 保留有序列组，不把其中几列拆成相互无关的约束。实际 catalog 与注册表的表集合、UUID 列集合、数组/domain 和 FK 必须逐项比较；数量相等不能替代集合相等。

当前三库没有 UUID 数组或 UUID domain；对应空清单是当前事实。新出现的 UUID 数组/domain、表、UUID 列或未登记 FK 均使批次失败，先补充元素类型、权威实体与证据，再导出或转换。

`authority` 使用 `authority.entity`，例如 `platform.user`、`platform.organization`、`stores.store`、`work_orders.ticket`。映射键是该语义域加原 UUID；同一权威实体只分配一次新值，外键和跨库软引用复用它。不同实体恰好有相同旧字符串时仍是不同映射。`mappingScopeColumns` 是关联/栅栏核验上下文，不能让每次出现重新分配一个 ID。

`role` 的允许集合由 `validationContract.allowedColumnRoles` 声明：实体主键、引用、作为主键的引用、用途标识、多态引用。`work_order_counters.tenant_id` 和 `notification_prefs.user_id` 是对已有租户/用户的引用，不是独立发号实体。

## 2. 必须遵守的权威归属

- 平台的用户、租户、组织、成员、角色、菜单、应用、发布、授权和通知由各自平台实体负责；门店/工单中 owner、actor、manager、assignee、recipient 的用户 ID 均复用 `platform.user`。
- `stores.id` 是 `stores.store`，`stores.organization_id` 是 `platform.organization`，不能因业务称“门店组织”而把两个字段映射成同一个实体。
- `store_audit_events.entity_id` 依据 `resource_code` 分派：`store → stores.store`、`store-staff → stores.staff_legacy`、`store-daily-report → stores.daily_report`。未知资源值拒绝转换。
- 审计的 `request_id` 属于 `telemetry.request`；部署/ZIP 的 `request_id` 分别属于各自意图幂等域。attempt nonce、verification nonce、JWT jti、URL entry revision 和 staging ID 都是独立用途，不能映射成其所属行主键。
- `application_url_entry` 审计的 `objectId` 是 application ID，`membership_data_grants` 审计的 `objectId` 是 membership ID，`oauth_consent` 审计的 `objectId` 是授权请求 ID。不能从对象名称猜测它们分别是 revision、grant 或 code ID。
- OAuth `client_id/clientId` 是不透明客户端协议标识；即使字符看起来像 UUID，也不是 `oauth_clients.id`。Docker image ID、内容摘要、group/node/target/template ID、业务编号、地区编码和幂等业务键同理。

缺少现存权威行的普通 FK/业务软引用是错误，不能临时创建用户或租户使其“通过”。历史审计可引用已删除主体或被排除的凭据行；这类引用只在保留的源快照中分配类型化墓碑映射，记录来源证据，不能因此创建可登录的运行主体。

## 3. 每表保留策略

`tables[].retention` 引用顶层 `retentionPolicies`。该表的全部行与非 UUID 业务字段都在该策略约束内；列中重复策略便于机器检查，不构成另一种表策略。

| 策略 | 目标处理 |
| --- | --- |
| `RETAIN_REMAP` | 保留主数据、配置、关系和业务事实；只转换登记的类型化引用。 |
| `CREDENTIAL_EXCLUDED` | 旧平台会话、刷新记录、OAuth token/code/device/授权请求不作为可用凭据导入。需要的原始证据在受限归档保存，新凭据重新认证签发。 |
| `HISTORY_TYPED_PROJECTION` | 原始字节与摘要独立归档；目标仅保留已证明正确的类型化历史投影及其证据关联，不重放历史动作。 |
| `TASK_RECONCILE` | 保存发布操作与任务事实；先核对库外效果及未知结果，再建立目标投影。旧 lease、nonce、verification nonce、receipt 不具有执行资格，禁止因导入而重新领取或部署。 |
| `RETAIN_IDEMPOTENCY` | 保留业务完成与去重范围，转换已登记请求/响应引用并遵守其 hash 合同；它们不是待重放队列。 |
| `REBUILD_METADATA` | 由目标库真实迁移创建 `_sqlx_migrations`、工单历史迁移表和门店 schema contract；源记录只作为核验，不直接伪写目标元数据。 |

`store_staff` 使用历史策略有源码依据：[门店第二次迁移](../../../../saas-stores/backend/migrations/0002_platform_owned_store_identity.sql)明确将原关联保留为历史，并拒绝 INSERT/UPDATE/DELETE/TRUNCATE。当前有效员工来自平台成员目录，不能在导入时重新激活旧本地员工关系。

OAuth client 配置保留，`client_secret_hash` 只进入私有导入材料；不能把 client 配置与应排除的会话/token 表混为一类。用户密码哈希按既有合同保留；账号凭据失效版本按恢复边界前推，不能修改密码来代替撤销旧会话。

必须对账的非 UUID 数据也已登记：工单 `next_number` 和 `number`、门店金额/营业日期、业务 version/状态/时间、通知 recipient/delivery 计数及已读事实、审计 bigint ID 与序列高水位。`audit_logs_id_seq` 恢复后不能低于导入最大 ID 或更高的源序列位置。工单计数器不得重置为 1。恢复告警 `sequence_file` 是严格递增的 u64，不能重置或改为 UUID。

## 4. JSON、TEXT、数组与配置路径

`structuredReferences` 的 `paths` 使用 JSON Pointer 风格；`*` 表示数组元素或对象 map 的值。通配符只用于已声明容器，不能按键名在任意深度递归。可空字段与前后快照只在确实存在且符合具体 action 的源 DTO 时处理，不制造缺失字段。输入 JSON 重复键、错误类型、未知 discriminator 或无法解析时失败，不能回退字符串替换。

已登记路径包括：

- 通知发布的 `target.kind` 条件化 user/org 引用，通知工单路由及既有投递事实。
- 工单创建幂等响应 `Ticket` 的 id/tenant/org/owner/assignee；提醒请求的 recipient、目标组织及 ticket 路由。
- 平台审计 `{objectType,objectId,before,after}` 按 objectType/action 分派，另有应用 setup、部署、ZIP 动作的专门合同。`oauth_history` 中 nil 是历史聚合哨兵，不能转换成实体。
- managed receipt 的 nonce，以及 ZIP receipt 中 work.jobId 与按原操作阶段判定的 nonce。原始回执作为证据归档，不导入为活动资格。
- `DATA_GRANT_POLICY`、grants manifest、bootstrap 输出和本地私有预览状态中的明确字段；不记录其值、秘密或连接串。应用/menu/client 的 map key 是逻辑键，值才是登记的 UUID。
- `.staging/{uuid}/owner.json` 的 `stagingId` 必须与目录 ID 一致。停写后由所有权流程核对旧暂存，不能只改 JSON 或盲目重命名未知目录。
- `regions.source_id` 中人工创建的 compact32 UUID。导入地区来源 ID 则保持外部语义；不能仅凭 32 位十六进制外观认定为平台 UUID。同一 source 被复制到多个地区版本时复用同一映射。

当前 artifact manifest 是闭合的 schema 1 文件/摘要结构，ZIP group policy 使用逻辑 group/node/application code，均没有可直接改写的平台实体 UUID。注册表把它们登记为“验证闭合 schema 并保留内容”，没有把所有 manifest/policy 的 `id` 强行视为 UUID。未知扩展仍需先分类。

私有状态中的 session/token 必须丢弃并重新认证；其用户/组织字段登记用于核对和生成目标状态，不允许通过改字符串继续使用旧签名。旧 bootstrap、重建报告和验收报告保原归档，目标报告重新查询生成，不把旧报告改写成新的验收证据。

## 5. 派生 hash 与历史证据

工单创建幂等 `request_hash` 是原始 Create DTO 的规范化请求摘要，并非 `response_json` 的摘要。Create 可含 assigneeUserId，而工单之后可能已经更新。缺少原请求时不能从当前 ticket 反推当时请求并重新计算；这类非空记录需要 owner 的明确迁移规则，否则阻止转换。

提醒意图的 hash 绑定精确存储 JSON 字节；只有在验证旧请求与各引用列一致后，才可通过登记 DTO 重新序列化已转换请求并计算目标摘要，原字节与旧摘要仍归档。导入不得触发提醒重发。

role menu/user binding digest、tenant/org configuration/authorization hash 可能没有保存当时完整集合，不能用当前集合重算并称为原审计。原值属于历史证据，类型化投影附映射来源。

managed/ZIP 的 `payload_sha256` 绑定 Intent，包含 release/request ID；重编号后的新活动意图需要重新预检和一致的新摘要。原签名、回执和已签 JWT 不进行字符串替换。未知库外执行结果、未知在途任务或无法证明的新 epoch 栅栏使切流失败，不能假定任务失败然后重放。

## 6. 未知路径与验收边界

发现看起来像 UUID 的值只是清点信号，不是变换授权。已知用户自由文本保持原样；未知 JSON/TEXT/URL/config 路径须在私有材料中分类，证明其 authority 或明确的不透明/历史证据语义，并补注册表。无法证明时阻止批次导出/转换/切流，不做全局正则替换。

活动 URL 只由所属应用解析已登记路由，例如 `/micro/work-orders/tickets/{ticketId}` 与 `/micro-proxy/{appCode}/{revision}/`。外部 URL、任意 query、用户正文及未知深链不能靠 UUID 搜索替换。外部 RP 的旧 sub 必须由 RP owner 协同重新绑定，本地改库不能代替外部处理。

恢复包本身的 recovery_point、时间、migration/catalog 和文件摘要有原认证语义；源包不可修改后仍当成同一个包。变换后的导出是带新身份 epoch 的独立认证材料。每次恢复/回退开放入口前，独立推进凭据失效边界和任务重放栅栏，不能只依赖备份里可回退的版本或“目标没有新写入”。

本合同尚未实现转换器：UUID-U02 负责新生成与 schema 策略，UUID-U03 实现导出和路径分派，UUID-U04 执行隔离初始化/导入。后续 schema 或协议新增字段时先更新注册表及覆盖证据，不能沿用当前 147 列清单宣称永久全覆盖。
