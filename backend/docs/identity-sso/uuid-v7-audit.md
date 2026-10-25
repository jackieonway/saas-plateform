# UUID v7 全系统审查与保数据重建方案

> 版本 v0.1，审查日期 2026-10-24。本文记录已完成的源码与指定预览库只读审查，以及尚未实施的改造方案。本轮没有修改生成器、依赖、数据库结构、数据、配置凭据或发布产物，也没有执行数据库初始化。

总体身份方案见[主方案](README.md)，UUID 专项执行依据见[专项台账](uuid-v7-task-ledger.md)，原员工/客户与 SSO 阶段见[身份台账](task-ledger.md)。只读数据库聚合证据保存在[清点结果](uuid-v7-inventory.json)，不包含行 UUID 原值、个人资料、密码或连接字符串。

## 1. 审查结论与覆盖范围

**当前系统没有统一采用 UUID v7，主要生成路径使用 UUID v4。** 仅把 `new_v4()` 改名或修改初始迁移不足以完成目标；必须同时处理数据库默认值、初始化种子、运行时 SQL、前端/SDK、输入校验、历史业务引用和失效凭据。

| 审查对象 | 已确认结果 | 证据性质 |
| --- | --- | --- |
| 六个工作区仓库的活动源码、迁移、配置和协议 | 存在 Rust v4、PostgreSQL v4 默认、JS 原生 v4 生成；未发现活动生产 v7 生成入口 | 静态审查，历史和测试另分类 |
| 本地平台预览库 | 107 个 UUID 列、23 个 v4 默认；2,706 个非空 UUID 单元格全为 v4 | 实际只读聚合查询 |
| 本地门店预览库 | 23 个 UUID 列；28 个非空 UUID 单元格全为 v4 | 实际只读聚合查询 |
| 本地工单预览库 | 17 个 UUID 列，非空 UUID 单元格为 0 | 空数据不能证明后续生成器正确 |
| 配置的远程开发库 | 只读 catalog 连接尝试未成功，未取得版本分布 | 不能套用本地预览库结论或声称已转换 |
| 生产、其他运行库、外部 RP、对象存储和外置发布卷 | 未提供完整目标清单，尚未清点 | UUID-U01 实际执行前清点 |

源码范围包括 `saas-plateform`、`saas-stores`、`saas-work-orders`、`saas-notification`、`saas-micro-app-sdk`、`saas-examples` 及必要工具。活动源码统计不包含 node_modules、target、dist、Git 对象和压缩构建产物；固定发行、副本和测试按单独口径审查，不混算为生产生成点。

本轮没有连接生产数据库。数据库读取均使用 `REPEATABLE READ READ ONLY` 事务，设置单语句 10 秒超时，仅查询 catalog 和按 UUID 版本聚合的计数，没有执行 DDL、UPDATE、DELETE、DROP、导入或初始化。

## 2. 实际预览库清点

三库来自本机 WSL Ubuntu 的 PostgreSQL 18.6，分别是：

| 数据库 | UUID 列数 | UUID 数组列 | v4 默认数 | FK 数 | 非空 UUID 值 | v4 | v7 | 其他版本/非 RFC variant |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `qiankun_platform_preview_20261024` | 107 | 0 | 23 | 64 | 2,706 | 2,706 | 0 | 0 |
| `qiankun_stores_preview_20261024` | 23 | 0 | 0 | 2 | 28 | 28 | 0 | 0 |
| `qiankun_orders_preview_20261024` | 17 | 0 | 0 | 1 | 0 | 0 | 0 | 0 |

以上是 UUID 类型单元格出现次数，包含同一个用户、租户、组织在多条外键中的重复值；不能解释为 2,734 个不同实体。三库各自处于一致只读快照，但没有建立跨库同一时间点的停写快照，因此本轮不是可直接导入的数据备份。

版本检查使用 canonical UUID 的版本位和 RFC variant 位，聚合结果逐列保存于 JSON。后续可在隔离库按同一方式复核，例如：

```sql
BEGIN TRANSACTION READ ONLY;
SELECT substring(id::text, 15, 1) AS version_nibble, count(*)
FROM public.users
GROUP BY version_nibble;
COMMIT;
```

清点时还必须检查 UUID 数组、domain、JSON/TEXT 引用和数据库外配置。当前预览库没有 UUID 数组列，并不代表未来目标或 JSON 内不存在列表引用。

## 3. 源码生成器与校验审查

### 3.1 平台数据库默认和初始化

平台 11 个迁移中静态解析到 39 个 CREATE TABLE、107 个 UUID 列声明，23 个 UUID 默认值均为 `gen_random_uuid()`，生成 v4。64 个含 REFERENCES 的静态声明行与本次预览库实际 64 个 FK 数相符；静态声明数量仍不能替代其他环境的真实 catalog。

| 迁移 | v4 默认数 |
| --- | --- |
| `0001_init.sql` | 19 |
| `0004_release_operation_protection.sql` | 1 |
| `0005_managed_image_deployments.sql` | 2 |
| `0006_zip_distribution.sql` | 1 |

涉及应用、发布、组织、租户、用户、成员、角色、菜单、部门、数据授权、通知、OAuth 请求/客户端/凭据、平台会话/刷新、发布操作、镜像及分发任务。关键依据为[初始迁移](../../migrations/0001_init.sql)、[发布操作迁移](../../migrations/0004_release_operation_protection.sql)、[镜像迁移](../../migrations/0005_managed_image_deployments.sql)、[ZIP 迁移](../../migrations/0006_zip_distribution.sql)。

[seed.rs](../../src/seed.rs)本身不生成 UUID，多种 INSERT 省略 ID，依赖这些默认值；所以只修改 Rust 的 `new_v4()` 不会改变初始化数据的 ID。另有[部署 cleanup 路由](../../src/managed_deployments/routes.rs)直接在 INSERT 中调用 `gen_random_uuid()`，也必须移除。

现有 UUID 类型只保证能存储 UUID，没有 v7 版本/variant 约束。现有外键也没有普遍配置 ON UPDATE CASCADE，不能在现库中逐表更换主键并期待引用自行更新。

### 3.2 Rust 与安全用途

按 Rust 生产编译路径人工分类，发现 24 处 v4 生成用途、19 个文件；其中一处用于随机 jitter，应改用随机数而不是 v7。另有上述 1 处生产内嵌 SQL v4 调用。排除 vendor、构建和历史发行后，包含测试的 `.rs` 扫描发现 229 处 `Uuid::new_v4` 符号引用：225 处带括号调用，4 处作为 map_or_else/unwrap_or_else 的函数值传入；这个全文计数不代表 229 个生产实体生成器，函数值传入同样是生成入口。

| 工程 | 生产调用数 | 代表用途 |
| --- | --- | --- |
| 平台后端 | 11 | OAuth 请求、jti、入口 revision、任务 attempt nonce、request ID、暂存目录、名称后缀、人工地区 source ID |
| artifact-agent | 1 | 暂存目录 |
| stores | 6 | 业务档案、日报、审计、请求 ID |
| work-orders | 3 | 工单、请求 ID、迁移校验临时 schema 后缀 |
| notification | 3 | HTTP/连接请求 ID、重连随机 jitter |

代表代码：[OAuth 签发](../../src/oauth_issuance.rs)、[OAuth 路由](../../src/routes/oauth2.rs)、[平台 observability](../../src/observability.rs)、[ZIP 安装](../../src/release_archives/installation.rs)、[门店写入](../../../../saas-stores/backend/src/stores.rs)、[日报写入](../../../../saas-stores/backend/src/reports.rs)、[工单写入](../../../../saas-work-orders/backend/src/writes.rs)、[通知 store](../../../../saas-notification/src/store.rs)、[通知 server](../../../../saas-notification/src/server.rs)。固定 telemetry 副本仍应纳入协议兼容核对，但不计作这四处函数值生成引用的来源。

五个直接声明 uuid 的 Cargo 包目前只开启 v4（部分另含 serde），锁定 uuid 1.26.1 或 1.28.0 已支持 v7；可以启用 v7 并使用 `Uuid::now_v7()`，不需要为此先升级依赖版本。实现阶段核对实际锁文件，并移除不再必要的生产 v4 生成依赖。

必须分别处理两类易遗漏逻辑：

- `release_archives/installation.rs` 的 staging 目录识别明确只接受版本 4；生成改为 v7 后，目录识别、owner.json、清理策略也须同步更新。
- `oauth_issuance_tests.rs` 有 jti 版本为 4 的断言；正常生成断言改为 7，明确验证旧格式或拒绝行为的负例按测试目的保留。

### 3.3 前端、SDK、Node 与固定发行

| 分类 | 静态结果 | 未来处理 |
| --- | --- | --- |
| 自有活动浏览器/SDK 源码 | 7 文件、9 处 `crypto.randomUUID()`，均 v4 | 统一可靠 v7 入口，覆盖宿主、请求和幂等 ID |
| 平台浏览器测试 | 8 文件、8 处原生 v4 生成 | 正常 fixture 改为 v7，负例标注 |
| SDK 固定发行和消费者副本 | 19 文件、19 处 | 新发行后逐消费者升级，不手改历史 vendor |
| 两份版本化工单 UMD | 各 2 处，共 4 处 | 重新构建新版本，产生新摘要，不修改旧产物 |
| 活动 UUID 形状正则 | 6 文件、7 处 | 当前能接受 v7，但不足以强制 v7 |
| 固定测试 UUID 字面量 | 118 次、28 个不同值 | 区分正常 v4、nil/占位和人工负例 |

9 个活动调用包括平台会话协调、qiankun 资源租约和挂载 ID、镜像/ZIP 请求 ID、通知幂等键、SDK 请求 ID、工单 IntentStore。Node 兼容后端和测试工具另有导入 `node:crypto.randomUUID` 的调用，尤其[工单旧 server](../../../../saas-work-orders/backend/server.mjs)及[observability](../../../../saas-work-orders/backend/observability.mjs)，仍在交付或兼容范围时也必须改造，不能只查 `crypto.randomUUID` 的浏览器写法。

活动生产前端发现的两处固定 UUID 为 nil 拒绝哨兵，不是实体数据。UUIDv4 负例、nil 哨兵、上游资料、不可变历史发行字节应保留其测试或证据含义；“系统统一 v7”的验收针对活动生成器、正常 fixture、系统自有 UUID 数据与配置引用，不要求把负例或历史证据伪造为 v7。

浏览器 v7 实现应按 RFC 9562 生成，使用 Web Crypto 安全随机源，明确同毫秒多次调用和时钟回拨行为；通过固定 SDK 新发行或明确版本依赖提供，不能各仓复制未经审查的位运算实现。无安全随机源时明确失败，不回退 `Math.random()` 或 v4。

新活动 SDK 发布时同步 `supportedSDKVersions`、门店 `validContext` 中硬编码的版本、生成的公开 DTO、来源记录、摘要、检查脚本及所有消费者测试。历史 2.0.0 字节冻结不阻止活动消费者升级；SDK 包版本变化也不必然改变 MicroProps 2.0 / Context 1.0 的线协议含义。

## 4. UUID 与安全随机值的边界

| 值的用途 | 统一规则 |
| --- | --- |
| 系统自有主键、UUID 外键、jti、revision、request ID、暂存及公开任务标识 | 新生成和迁入目标库均为有效 RFC variant UUID v7 |
| 发布 attempt/receipt nonce | 目前是防陈旧栅栏，可改 v7，但仍须独立 Bearer 认证及 attempt/nonce/摘要复验 |
| notification 重连 jitter | 使用专门随机字节/随机数，不读取 v7 的时间前缀 |
| 密码、密钥、OAuth code、device code、refresh、PKCE verifier、OIDC nonce | 保持密码学安全随机值；不以 UUID v7 替代熵来源 |
| client_id、业务编号、地区编码、Socket.IO ID、文件摘要 | 按原协议类型保存，不强制改造成 UUID |
| 外部系统拥有的 UUID、签名原文、历史备份 | 保留原语义和来源，明确映射或协同迁移，不能冒充本系统可任意改写的主键 |

`Uuid::new_v4().as_bytes()[0]` 的 jitter 不能改成 `now_v7().as_bytes()[0]`，因为 v7 高位是时间。截取 v7 前缀也不能当随机秘密。UUID 不代替访问控制；业务排序继续使用 created_at、序号、version、attempt，不能声称分布式 v7 是严格全局单调序列。

请求 ID 的输入合同也要统一：新内部请求必须生成 v7；外部传入非 v7 时按合同拒绝，或生成本系统 v7 并把原值作为非 UUID 的关联字符串保存。不能通过“兼容请求头”继续把 v4 写入要求纯 v7 的 UUID 列，也不能不通知消费者就改变关联语义。

## 5. 新数据生成与数据库兼容决策

### 5.1 保持 PostgreSQL 17 工程基线

本机预览库版本为 18.6，但平台 CI 和恢复工具仍以 PostgreSQL 17 为基线。PG17 的 `gen_random_uuid()` 是 v4，没有 PG18 内置 `uuidv7()`，因此本方案选择：

1. Rust/Node/浏览器自有 UUID 生成入口统一到 v7，后端负责最终验证。
2. 所有持久 ID INSERT、seed、bootstrap、CLI、worker 和导入器显式提供 UUID v7。
3. 用追加迁移删除平台 23 个 v4 默认，修正内嵌 SQL 生成，不修改已执行迁移。
4. 对系统自有 UUID 列和受控输入补版本/variant 检查；空值按原可空规则，未知 UUID 形状或版本不悄悄转换。
5. 不为这项改造引入 PG18 升级前提，也不自行拼接未经验证的 SQL UUID v7 算法。未来改用数据库原生生成时，另有版本与恢复合同验收。

新库按原迁移加新增 UUID 策略迁移的真实顺序初始化，然后才运行 v7 seed 或保数据导入。当前历史迁移中除设置与 schema 合同外没有依赖 UUID 默认的业务种子插入；执行前仍需重新核对迁移头。不能删改 `_sqlx_migrations`、改历史 checksum 或简单把旧初始迁移全部替换为新文件。

### 5.2 检查与失败行为

生成器及正常 fixture 检查 UUID 版本为 7、variant 正确、时间布局可解码，同毫秒批量唯一。数据库约束可使用 UUID 二进制版本/variant 位检查，需在 PG17 和实际目标版本验证。

删除默认后，遗漏 ID 的 INSERT 必须明确失败，不能偷偷恢复 v4 默认。正常 API 创建、初始化、worker 和导入全链路必须覆盖，不能只靠 UUID crate feature 配置证明目标达成。

## 6. 已有数据如何变成 v7

### 6.1 采用一次性身份基线切换

用户要求已有 UUID 一并调整，本方案按保留业务数据的重编号处理：非 v7 系统自有实体分配新 v7，已有有效 v7 可以保持；随后初始化独立目标库并导入变换后的数据。不会仅把 UUID 字符串的版本字符从 4 改为 7，也不会把所有表的同名 id 各自随机替换。

这项用户要求取代 v0.1/v0.2 中“原 users.id 永久不变”的前提：先完成一次 UUID 身份基线切换，再以新的 v7 `users.id` 作为后续稳定 SSO `sub`。不按邮箱、电话或名称合并账号。已有外部 RP 若以旧 sub 为主体键，必须提供协同迁移或明确重新绑定流程；本地改库无法修复外部账号关联。

长期保留受保护的 old→new 主体映射和批次证据，不把此映射暴露成匿名账号枚举接口。旧 ID 仅留在隔离备份、原始证据和映射档案中，不能继续作为新运行库的业务 ID。

### 6.2 映射键与时间来源

映射键至少包含 `authority/entity_kind/old_uuid`，同一个权威实体只分配一次新 ID。示例：平台 user 的新 ID 在平台、stores、work-orders 的 owner、actor、manager、assignee 和历史关联中复用同一映射。不同实体域即使恰好使用同样的旧值，也不能因字符串相同被合并。

- 优先使用实体可信 created_at 构造 v7 时间；没有可信时间时使用批次冻结的迁移时间并记录来源，不伪造历史创建日期。
- 随机部分使用可靠库的安全随机源；不能声称从 v4 推导出了原创建时间。
- request ID 等非实体 UUID 按用途命名空间统一映射，保留同一请求在多条日志/审计中的关联。
- 业务金额、业务编号、counter、状态、业务 version、created_at 等原值保持，除明确为安全撤销的版本升级外不改变业务事实。
- 映射文件有批次、来源快照、计数、摘要、冲突检测及断点恢复规则；同一批次重跑复用映射，不能再次生成另一套 ID。

### 6.3 必须覆盖的引用

1. UUID 主键、外键、组合外键、自引用、唯一键、数组及声明的 UUID domain。
2. 平台与门店、工单之间没有 FK 的 tenant/org/user/app/member 软引用。
3. JSON/TEXT 中明确登记的 ID 路径，包括通知 request_body、审计结构化信息、任务 manifest/policy/receipt 和业务请求快照。
4. 环境策略、grant 文件、bootstrap 输出、外部客户端保存的主体、URL/revision 与必要文件系统关联。
5. 审计中 actor、owner、目标实体 ID 和 request ID 的区别；不能给每次引用分配新值。

未知 JSON 字段、自由文本和看起来像 UUID 的业务字符串不做全文件正则替换。先分类、登记变换器和验收规则；不能证明覆盖的引用使该批次不能切流。

### 6.4 任务、令牌与签名数据

- 旧平台 session、refresh、OAuth access/refresh、code、device、登录票据、SSO/RP 会话和 app token 全部失效，要求重新登录/授权。密码哈希可保留，不能借重建偷偷改密码。
- JWT 内有 sub/tenant/org/sid/jti，不能改字段后继续沿用旧签名和 token_hash；旧认证临时表不作为可用会话导入。保留的认证审计按映射转存，原文证据另归档。
- idempotency/outbox/提醒、发布与部署任务先停写、停止领取和重放。保留业务完成事实与去重语义；在途或未知结果逐项核对，不能把重建变成重复工单、重复通知或重新部署。
- 签名回执、payload_sha256 和外部签名原文不靠字符串替换维持有效；保留原始证据，需要新身份绑定的活动操作重新预检、签发新任务和凭据。

## 7. 配置、文件与发行物的映射清单

已确认存在以下非数据库引用，执行时应读取当前实际文件并输出不含秘密的字段级清单：

| 对象 | 当前发现 | 处理 |
| --- | --- | --- |
| 平台本地 DATA_GRANT_POLICY | backend/.env 有 3 个 v4 引用 | 按 app、tenant、organization 权威映射重写并验证 |
| 预览私有运行策略 | private env 与 preview policy 各有 4 个 v4 引用 | 新目标配置完整生成，禁止打印秘密或只改模板 |
| preview-state 私有状态 | 74 次 UUID 引用、54 个不同值 | 按字段映射，不能把旧报告当新环境初始化来源 |
| grants 示例 | 4 份共 36 个占位引用 | 实际配置按新权威 ID 填充，模板占位不计为业务实体 |
| bootstrap manifest | 4 份采用 app code/menu key，无固定 UUID | 保留逻辑键，新生成报告输出新 ID |
| bootstrap 报告和 micro-proxy 深链 | 报告有 revision UUID 路径 | 重生成活动入口与报告，旧验收证据只读归档 |
| staging 目录与 owner.json | 目录 UUID 和 stagingId 严格绑定，当前只识别 v4 | 停写后按所有权处理旧暂存；新建 v7 并同步识别规则 |
| ZIP 已安装版本与 .artifact-manifest.json | 路径主要为 appCode/version，清单为文件和摘要 | 无身份引用的不可变字节保留，不因换 DB 主键改 SHA-256 |
| SDK/vendor/UMD 固定交付 | 有 v4 生成代码和来源摘要 | 从源码生成新版本、正式发布并逐消费者升级 |

本次没有在被扫描的非 artifacts 文件中找到名为 `release.manifest.json` 的实体文件；真实外置卷和对象存储仍需 UUID-U01 清点，不能据此声称不存在 UUID 路径。历史验收报告不得改成貌似验证过新库的数据。

## 8. 数据库重新初始化与保数据导入路线

推荐路线是“冻结源数据 → 完整备份 → 离线映射 → 初始化新库 → 导入并核对 → 协调切流”，旧库保留只读备份。这里的重新初始化以新目标库重建为默认，不直接 DROP 当前库，也不默认丢弃已有账号和业务数据。

### 8.1 目标与快照

UUID-U01 登记实际 host、端口、数据库名、服务配置、数据 owner、快照时间、发布卷和外部消费者。不能以“所有数据库”为目标执行删除，也不能从预览库名称推断正式库。

平台、门店、工单及相关 worker/通知/部署/CLI 必须协调停写，取得可证明的跨服务一致快照。此次独立只读预览统计不能代替停写备份。

### 8.2 初始化与导入

- 建立独立、空白的目标库，由各库真实迁移初始化；v7 策略迁移在 seed/导入前生效，运行态与 CI 的支持版本经过核对。
- 干净 seed 模式与保数据导入模式互斥。导入模式关闭自动 demo/bootstrap seed，避免重建目标产生另一套租户、用户、应用后再冲突合并。
- 数据离线变换后导入新行，不在源库关闭所有触发器逐表 UPDATE 主键。对于循环 FK、不可变/只追加触发器和需要 UPDATE 的导入关系，设计独立、限用途的导入路径，并在目标提交前验证所有约束和触发器恢复。
- 保留真实业务状态、时间、金额、计数器和必要版本；递增安全凭证版本并维持所有旧凭据不可用。worker、outbox、提醒、通知重放默认禁用，确认去重和在途状态后才能恢复。
- 门店/工单的 schema fingerprint 与发布数据库 catalog 通过新迁移和新合同升级，不改写旧迁移或伪造 expected fingerprint。
- 现有 recovery 工具拒绝已有目标，且只会原样恢复和撤销凭据，不会做 UUID 映射；必须扩展受控重建流水线，不能把当前 restore 命令当作已实现的 v7 转换器。

### 8.3 核对与切流

切流前全部自有目标 UUID 字段非空值应为 v7 且 RFC variant 正确；外键孤儿、跨库孤儿、未登记软引用残留、映射冲突和不可解释的数据损失均为 0。逐表行数、业务金额汇总、状态分布、counter 和关联计数与源快照核对。

新环境先以隔离入口启动，检查员工登录、应用授权、门店/工单、通知、CLI、SDK 与部署预检。切流修改服务配置与可信策略，清除旧缓存、旧 socket 和旧凭据，只允许新 ID 进入正式流量。旧请求或不在映射合同内的旧链接返回明确失效，不按姓名或邮箱自动查找目标。

无新写入且没有未对账的数据库外效果时，才可按验证合同恢复旧快照到受控恢复副本；原始备份仍只读保留。任何恢复/回退在开放入口前都重新推进不能随快照倒退的认证失效边界，提升相应凭证版本、撤销 session/refresh/code/device 并核验旧 JWT/签名策略，不能原样恢复有效旧凭据。回退至旧身份 epoch 不代表 v7 专项完成，运行代码、配置和数据须按兼容恢复合同一致。

回退条件同时覆盖部署安装、通知投递、外部 agent 和在途请求。恢复前冻结 agent、对账已发生的外部效果，使用迁移 epoch、轮换任务凭据或等效栅栏拒绝旧 nonce/lease/receipt 重放；历史签名证据不能成为恢复旧任务执行资格的依据。已有新写入或库外效果时，简单切旧库会丢数据或重复执行，必须先停写并完成反向/增量变换及外部效果对账。原库及映射档案删除属于后续保留策略，不是本轮审查或重建成功的必需步骤。

## 9. 验收标准与实施顺序

| 编号 | 必需验收 |
| --- | --- |
| V01 | 五个 Rust 包及自有 JS/Node/SDK 所有正常生成入口产生 v7，版本/variant/批量唯一性正确 |
| V02 | 23 个 v4 默认和生产内嵌 v4 SQL 不再产生值；seed、CLI、worker、正常 API 与导入不遗漏显式 ID |
| V03 | jitter 使用随机数，密码/secret/code/refresh/PKCE 不退化成 v7 或时间前缀 |
| V04 | 所有系统自有目标 UUID 列、数组和登记软引用符合 v7；v4 负例/nil 哨兵/历史归档不混入正常数据 |
| V05 | old→new 映射唯一、可重跑，平台主体在三库引用一致，无 orphan 或错误主体合并 |
| V06 | 行数、金额、业务编号、counter、状态、时间和历史关联核对通过；未丢业务数据 |
| V07 | 切流及任何恢复后旧 session/refresh/code/device/JWT 均不可复活；认证失效边界不随快照倒退，外部 RP 关联与用户重登归属正确 |
| V08 | schema/fingerprint、历史 migration checksum、所有约束与触发器按正式新合同验证 |
| V09 | SDK 新发行、活动消费者、版本允许列表/生成 DTO/来源摘要一致；staging 与 owner 匹配，旧发行和原始签名证据未被篡改 |
| V10 | 策略/env/grants/入口引用与新 ID 同步，无旧 UUID 被当作活动业务 ID；报告重新生成 |
| V11 | 在途任务/agent/库外效果对账完整，旧任务栅栏不因恢复复活，幂等与提醒不重复执行，切流后新写入有保数据恢复方案 |
| V12 | 明确每个实际目标的只读清点、备份、导入、切流与恢复证据，未审目标不能被宣称已完成 |

执行顺序是 UUID-U01～UUID-U06，再进入原身份台账 Phase 1～13。原阶段编号保持，UUID 基线切换后这些阶段所称“保留成员 ID、稳定账号 sub”均指重建后的 v7 ID；普通身份变更和 SSO 服务拆分不再重编号。

## 10. 当前完成与未完成

已完成：源码生成器/默认/校验与引用风险审查；本地三个指定预览库只读 catalog 和版本聚合；保数据重编号、初始化新库及后续验收路线。

未完成：配置远程开发库及生产数据清点、生成器改造、v7 schema/seed/SDK 实现、映射程序、备份导出、目标初始化、数据转换、切流和恢复演练。专项阶段全部 `NOT_STARTED`；本次文档不能作为数据库已转换或已重新初始化的证明。
