# Rust 工单服务：业务库基线、授权、读写与恢复

Phase34当前入口：Node index与legacy服务已退役；npm start分派Rust，历史Node说明仅供迁移/协议回归参考。

独立Rust crate位于 `business-apps/work-orders/backend`，包含自己的Cargo.toml／Cargo.lock、薄入口和窄library，不依赖平台实现、不读平台身份表、不需要Node运行时。Phase16建立库接管，Phase17增加授权／读取／CSV，Phase18增加幂等创建和版本写入，Phase19接入普通应用Bearer的成员目录及真实分配预校验；Phase20增加持久提醒意图和正式平台通知发布。Phase21独立Vue／Vite前端消费这些Rust契约，提供创建header与显式未知结果恢复，不改变后台语义。旧Node保留兼容，生产未切流。本文描述当前契约，实际执行与验收结果由各阶段报告单独记录。

## 独立构建与启动

从业务目录执行：

```sh
cargo build --locked --release
cargo test --locked --lib
cargo test --locked --doc
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo doc --locked --no-deps
```

构建后的 `target/release/work-orders-api`（Windows 为 `.exe`）直接运行。使用部署系统向进程注入[配置模板](../business-apps/work-orders/backend/config.rust.env.example)中的环境变量；程序不会自动读取 `.env`，也不会把平台的 `DATABASE_URL` 当成业务连接。不得将真实连接或凭据放入源码、Vite 配置、URL参数或日志。

`WORK_ORDERS_DATABASE_URL` 必须明确包含 PostgreSQL host、user 和独立业务 database。URI 只接受已登记的 TLS 参数及 `application_name`，未知或重复语义参数在 SQLx 解析前拒绝，防止驱动把未知参数值写入诊断。数据库 TLS 遵循 URI 的明确 `sslmode`，未指定时确定为 SQLx 的 `prefer`；使用 `verify-full` 时保留正常证书及主机名验证。内网 PostgreSQL 的传输策略与平台 HTTPS origin 策略分别配置。

| 配置 | 默认值 | 允许范围与用途 |
| --- | --- | --- |
| `WORK_ORDERS_HOST` | `0.0.0.0` | 数字 IPv4/IPv6 地址，避免启动时无限 DNS 查询 |
| `WORK_ORDERS_PORT` | `3004` | 生产1–65535，测试/开发可0 |
| `WORK_ORDERS_DB_MAX_CONNECTIONS` | `10` | 1–30 |
| `WORK_ORDERS_DB_ACQUIRE_TIMEOUT_MS` | `5000` | 100–30000；连接池获取预算 |
| `WORK_ORDERS_DB_STATEMENT_TIMEOUT_MS` | `5000` | 100–30000；服务端语句预算 |
| `WORK_ORDERS_DB_LOCK_TIMEOUT_MS` | `5000` | 100–30000；服务端锁等待预算 |
| `WORK_ORDERS_STARTUP_TIMEOUT_MS` | `30000` | 1000–300000；连接与初始化的总预算 |
| `WORK_ORDERS_SHUTDOWN_TIMEOUT_MS` | `10000` | 100–60000；退出与连接排空预算 |
| `WORK_ORDERS_HEALTH_TIMEOUT_MS` | `2000` | 100–10000；单次就绪查询预算 |
| `PLATFORM_API_ORIGIN` | 必填 | 精确 HTTPS origin，无路径、query、fragment 或用户信息；非生产仅额外允许 loopback HTTP |
| `PLATFORM_CLIENT_ID` | 必填 | 平台生成的机密客户端实际 ID，绑定 `work-orders`；1–100 个 ASCII 字母、数字、`.`、`_`、`-` |
| `PLATFORM_CLIENT_SECRET` | 必填 | 私有服务端凭据，1–1024 字节，不含 CR/LF |
| `PLATFORM_AUTH_TIMEOUT_MS` | `3000` | 100–10000；内省、context、响应读取及校验共用的单请求总预算，单位毫秒 |
| `PLATFORM_AUTH_MAX_CONCURRENT` | `32` | 1–128；当前进程同时授权上限，饱和立即 503，不排队 |
| `PLATFORM_CA_CERT_FILE` | 不设置 | 可选 PEM 根证书 bundle，最多 1 MiB，追加到正常信任根；保持证书及主机名验证 |
| `WORK_ORDERS_ALLOWED_ORIGINS` | `null` | 逗号分隔的精确 origin，或 opaque iframe 的字面 `null`；禁止 `*` |

连接还有15秒空闲事务预算。正常运行连接的 `search_path` 固定为 `pg_catalog,public,pg_temp`；迁移在事务内临时选择明确目标 schema。`NODE_ENV=production` 同时启用生产端口和精确 HTTPS origin 规则，不引入 Node 运行依赖。应用编码固定为 `work-orders`，没有可由请求或环境覆盖的 appCode。机密客户端 secret 仅进入 Rust 私有配置及平台 Basic 内省，不进入 Vite、dist、URL、公开错误或日志。

`Config` 仍允许只加载业务库配置，供迁移工具和 Phase 16 的 `router(pool, timeout)` 健康兼容入口使用；该 router 不提供受保护读取。实际二进制的 `run` 和完整 `application` 必须具备全部平台配置；缺失时在开放监听前拒绝，不能把 DB-only 配置当作正式启动条件。配置任意一个平台变量后，其余必需字段也必须完整有效。

## 空库初始化与旧库接管

启动使用与旧 `db.mjs` 完全相同的 PostgreSQL 事务锁：

```sql
SELECT pg_advisory_xact_lock(hashtextextended('work-orders-business-migrations',11));
```

所有检查和写入都在同一事务中执行。两个 Rust 实例以及旧 Node/Rust 并发初始化共享此锁；等待后读取当前状态。连接、锁、语句及总启动时间分别受限，超时或异常不会开放监听端口。

1. 校验内嵌 `migrations/0001_work_orders.sql` 的不可变 SHA-256：`a2e4224febd195fdd663a8954c3be4b76a8a06338521056c6809ebaa57d04090`。
2. 校验0001、[0002_create_idempotency.sql](../business-apps/work-orders/backend/migrations/0002_create_idempotency.sql) 和 [0003_reminder_intents.sql](../business-apps/work-orders/backend/migrations/0003_reminder_intents.sql) 的有序序列。0002建立创建receipt，0003建立 `work_order_reminder_intents` 及禁止更新／删除的触发器；0001／0002原字节保持。
3. 接受空库，以及Node历史为合法0001／0002／0003前缀、SQLx历史为空或相应合法前缀的旧库。历史必须与实际结构对应，SQLx不能领先Node兼容历史；非空无记录、缺项／未知／dirty历史、checksum或结构不符均拒绝。
4. 在同一事务的随机参考schema中构造对应迁移前缀结构，比较真实表／列／默认／约束／索引及有效性、不可变身份和提醒意图函数、安全配置、触发器、RLS/policy、rule及继承；非预期业务关系也拒绝。ACL和所有权由部署策略管理。
5. 只执行真实缺少的增量DDL，并补齐Node SHA-256与SQLx原生SHA-384双历史；已执行迁移不重放。最终必须有0001／0002／0003完整双历史和当前结构。`InitializationReport` 可返回 `Fresh`、`AdoptedLegacy`、`Upgraded` 或 `AlreadyCurrent`；`Upgraded` 表示已有Rust迁移历史推进至当前序列。
6. 删除参考schema后提交；失败时整笔事务回滚。存量工单、UUID、归属、version、计数器和已有迁移时间保持不动，不伪造旧历史。

迁移账号必须拥有**独立业务数据库**的建 schema、表和函数权限。参考 schema 的生命周期受同一事务保护，失败不会残留。业务库不得复用平台数据库；其他非系统 schema 中的永久关系也会使专用库校验拒绝。上线前应先在隔离副本预演并核对目标，不能将自动接管当作备份或生产恢复工具。

## 逐请求平台授权

每个受保护请求先用私有机密客户端 Basic 调用 `POST /oauth/introspect`，再用同一应用 Bearer 调用固定 `GET /api/apps/work-orders/context`。不合并两次请求，不跨请求缓存成功授权。客户端不跟随重定向、不采用环境代理；两次请求及其响应读取共用 `PLATFORM_AUTH_TIMEOUT_MS` 总预算，每份 JSON 响应最多 256 KiB。并发许可覆盖这两次调用和有界响应读取，饱和直接返回 503 `PLATFORM_UNAVAILABLE`。

授权核对 active、Bearer类型、`aud:work-orders`、token到期、context1.0、appCode、contextVersion、用户／租户／组织身份一致性，以及当前操作read／export／create／update／delete对应的 `ticket:<action>` permission和唯一 `ticket/<action>` data grant。提醒从同一次当前授权上下文同时核对ticket/read和独立notification/publish的功能权限、data grant及范围，不将发布解释成ticket/publish。grant的用户与租户必须匹配当前身份；组织硬上限缺字段时仅当前组织，显式null与有限组织数组保持各自语义。SELF按记录可信 `owner_user_id` 限定，处理人身份不扩大范围。

列表、count、详情和CSV共用参数化SQL范围：当前tenant、未软删除、组织硬上限与SELF／ORG／ORG_SUBTREE／TENANT grant取交集；更新／删除按相同组成限制锁定查询。创建归属来自可信用户、tenant和当前组织。Basic内省及context先于业务连接；Phase19需要分配变更时，额外先作范围内非锁预读，释放业务连接后才请求成员HTTP，再进入写事务及行锁。锁内没有外部HTTP。context与token取较早到期，每次新请求／重放重新授权，等待后及COMMIT前继续复验。

## 健康与 HTTP 契约

| 路径 | 当前含义 |
| --- | --- |
| `GET /work-orders-api/health/live` | 进程存活，返回 `scope:process` |
| `GET /work-orders-api/health/ready` | 业务库0001／0002／0003双历史、工单／计数器／receipt／reminder intent必需列及启用身份／意图不可变触发器可用，返回 `scope:business-database-schema` |
| `GET /work-orders-api/health` | ready 的兼容别名 |
| `GET /work-orders-api/tickets` | `ticket:read` 与 read grant 下的列表和同范围 count，返回 `{records,total,page,size}` |
| `GET /work-orders-api/tickets/{id}` | 同 read 范围的详情；不存在、软删除或不可见统一 404 |
| `GET /work-orders-api/tickets/export.csv` | `ticket:export` 与 export grant 下的有界 CSV，不沿用 read grant |
| `POST /work-orders-api/tickets` | create授权与必需 `Idempotency-Key` 下创建或重放原201 JSON |
| `PATCH /work-orders-api/tickets/{id}` | update授权下的版本更新／状态转换，200工单 |
| `DELETE /work-orders-api/tickets/{id}` | delete授权下的版本软删除，200 `{ok:true}` |
| `GET /work-orders-api/assignees` | create／update对应操作与grant下的有效直接成员搜索／分页，目标组织还受目录scope和硬上限约束 |
| `POST /work-orders-api/tickets/{id}/remind` | 当前read与独立publish授权下准备持久意图，再通过平台正式通知API发布；200返回经过校验的发布回执 |

完整 catalog 校验在启动时执行；运行期 ready 是明确受限的只读检查，不能据其200推断所有结构属性、每个调用方的平台授权、后续完整工单 API 或旧 Node health 的平台探测均已验收。DB故障时 ready 返回503，live仍可返回200；恢复后新检查可回到200。未知路径返回JSON404，不回落宿主HTML；不支持的方法返回JSON405。上述表中省略前缀的路径仍位于 `/work-orders-api` 下。

错误体为 `{code,message}`。凭证失效或上下文身份不一致401，缺操作／数据授权403，不可见记录404；版本竞争或预读与锁定状态不符409 `VERSION_CONFLICT`，创建同键异正文409 `IDEMPOTENCY_CONFLICT`。输入无效400，写请求体超过64 KiB为413 `REQUEST_LIMIT`。平台／业务库不可用、上游协议错误或饱和503；成员HTTP超时503 `PLATFORM_UNAVAILABLE`，已取得成员结果在提交前过期503 `MEMBER_VALIDATION_EXPIRED`并回滚，提交后才发现context／成员期限过期则返回结果未知错误。请求总预算15秒，读取耗尽503 `REQUEST_TIMEOUT`，创建及版本写入耗尽分别503 `CREATE_RESULT_UNKNOWN`／`WRITE_RESULT_UNKNOWN`，不能据超时认定未提交。

请求接受单个合法 UUID `X-Request-Id`，规范化后回传并传给该请求的内省、context、成员或提醒发布等平台调用；重复或非法值由服务重新生成。日志仅用固定路由类别，不记录实际 URL/query、正文、token、用户或数据库连接。配置及生命周期/迁移错误的公开格式脱敏，底层错误 source 只供受控诊断，不能直接打印整条 source chain。

CORS默认允许字面 `Origin:null`，仍要求正常Bearer授权。成功、授权／参数错误及404等通过同一中间件返回允许来源、`Vary:Origin`、`X-Request-Id`，无credentialed cookie CORS。允许来源的OPTIONS不访问平台或数据库，允许GET／POST／PATCH／DELETE，header为Authorization／Content-Type／X-Request-Id／新增Idempotency-Key。未知或重复Origin、非法预检403 `ORIGIN_DENIED`，不回显未授权来源。所有响应带 `Cache-Control:no-store`。

## 创建、版本写入与未知结果恢复

写请求必须为 `application/json` 对象，禁止未知／重复字段和写接口query。创建正文 `{title,description?,priority?,assigneeUserId?}`；标题trim后1–200个Unicode字符，描述默认空、最多8192字符，优先级默认NORMAL且仅LOW／NORMAL／HIGH，标题／描述拒绝NUL。description／priority显式null不等价省略。创建返回OPEN、version=1的工单，编号、身份、名称和时间由服务端生成或取自验证后的平台上下文。

`Idempotency-Key` 必须是单个请求头，长度1–128，字符仅 `[A-Za-z0-9._:-]`；不使用requestId或提醒事件键代替。验证和归一化后，按固定DTO字段顺序序列化title／description／priority／assigneeUserId并计算SHA-256。title trim、缺省值及省略／null处理人归一，因此JSON键顺序与等价默认写法不制造不同请求；服务器组织／人员名称不参与正文摘要。

Phase21前端 `client.mjs` 为创建显式发送该header，首次提交前保留不可变键和完整正文；结果未知后锁定字段并由用户选择“恢复原事件”。提醒正文中的 `idempotencyKey` 仍是独立协议，不能替代创建键。浏览器内存恢复生命周期见后文，Rust receipt与0003持久意图分别维持各自服务端保证；前端接入不代表生产流量已切换。

receipt唯一域为tenant／org／user／create／key，receipt、工单、租户编号计数器和原201 JSON在同一事务提交。同域同键同正文返回保存的原201结果，异正文409；每次重放仍要求当前create权限、grant和组织范围，换组织或失权不能取得旧域结果。工单之后编辑或软删不会改写创建receipt，重放只返回创建历史，当前记录必须另查GET详情。没有receipt自动清理／到期任务。

PATCH正文 `{version,title?,description?,priority?,status?,assigneeUserId?}` 至少含一个修改字段；DELETE仅 `{version}`。version接受1–2147483646的整数语义JSON数字，允许1.0／1e0，不允许字符串。二者先按当前授权范围锁定未删除记录，再核对version；旧版本409，不可见／已删404。成功更新或软删提升版本，软删记录保留且从列表／详情／导出中排除。状态可保持原值，或OPEN→IN_PROGRESS→RESOLVED及RESOLVED→IN_PROGRESS重开，不允许直接OPEN→RESOLVED或退回OPEN。

Phase19创建的非空assignee必须是目标组织的有效直接成员，服务以当前应用Bearer调用平台验证并保存返回的名称快照。update省略assignee或提交同一UUID保留原分配／名称快照，不重查旧成员；真正change或clear才要求目标组织目录scope，非空change还须平台验证。空字符串不是清除值，清除使用null。SELF仍按owner判断，不因处理人身份扩权，也不因能修改可见子组织旧记录就能在该组织更换分配；运行路径没有放行任意成员的替身。

| 情况 | 当前结果与恢复动作 |
| --- | --- |
| COMMIT前复验授权已到期 | 401并回滚当前事务，重新取得授权后再决定操作 |
| 创建COMMIT错误、提交后授权到期或15秒创建请求截止 | 503 `CREATE_RESULT_UNKNOWN`；保留原键和原正文，用当前有效凭据在原域恢复，不换键重复创建 |
| PATCH／DELETE的COMMIT错误、提交后授权到期或15秒写请求截止 | 503 `WRITE_RESULT_UNKNOWN`；读取当前详情／版本或确认不可见状态，不自动重试 |
| 单纯客户端取消／响应丢失 | 不能证明服务端未提交；创建按同键恢复，版本写入先读回状态 |

15秒总期限覆盖授权、正文读取、成员预校验、连接池等待和数据库事务；连接获取、SQL语句、锁等待各有前述独立预算。外部HTTP期间不持有业务连接或行锁，事务内只执行本地SQL与范围／版本／有效期校验；不自动重试PATCH／DELETE。

## 处理人目录、分配预校验与采用

平台在对应application的 `DATA_GRANT_POLICY` 中显式登记 `memberLookups:[{"resource":"ticket","action":"create"},{"resource":"ticket","action":"update"}]`。省略默认空，每应用最多128个唯一pair，且必须已存在于同应用capabilities；公开capabilities仍是resource／action／scopes三字段，不新增成员grant或assign操作。完整JSON示例与管理范围边界见 [成员查找声明](data-grant-management.md#member-lookups)。这是平台私有配置，工单不增加环境变量、数据库连接或依赖；业务用户无需grant管理权限。

`GET /assignees` 保留action=create或update（必需）、organizationId（默认当前组织）、q（原始值先限100个Unicode字符，再trim）、page（默认1、最大1000000）、size（默认20、最大50），query最多8192字节。返回 `{records:[{userId,displayName,organizationId}],total,page,size}`；搜索为显示名称的字面子串，不把 `%`／`_` 当通配扩权。服务先验证对应操作及grant，再以同一普通应用Bearer调用平台members列表／单成员HTTP，不能使用平台session、机器token或SQL替代。SELF／ORG目录仅当前组织，ORG_SUBTREE／TENANT仍受组织硬上限限制；候选出现过不代表提交时仍有效。

更新先按业务scope非锁定预读，取组织、version和oldAssignee并释放连接；需要新分配时再调用平台验证。随后BEGIN／FOR UPDATE，重新检查组织、version和oldAssignee与预读一致，否则409 `VERSION_CONFLICT`，不能用锁前结果覆盖并发修改。真正clear不需要查询新成员，但仍须目标组织scope和锁内复核。新建非空分配同样在写事务前验证；已commit的同key／同hash创建receipt重放只复验当前create授权，不重查历史member，原201不是当前详情或成员仍有效的证明。

成员校验窗口从成员HTTP请求开始使用单调时钟，期限为 `min(PLATFORM_AUTH_TIMEOUT_MS,3000ms)`；响应读取及随后pool／锁等待均消耗窗口，不能在收到响应或取得行锁时重置TTL。成员HTTP本身超时503 `PLATFORM_UNAVAILABLE`；已取得的成员结果在提交前过期503 `MEMBER_VALIDATION_EXPIRED`并回滚，同时继续复验context／token有效期。COMMIT后才发现context／成员期限过期返回503 `CREATE_RESULT_UNKNOWN`或 `WRITE_RESULT_UNKNOWN`，按创建键／正文或读回版本恢复，不能认定回滚。平台库和业务库不共享事务，最后一次成员验证与本地COMMIT之间仍可能发生撤权或成员失效；窗口只约束陈旧结果，不承诺跨库原子性，也不自动抹掉已有分配。

采用时先在平台合并显式create／update成员查找声明，保留所有既有能力和管理范围，按策略启动加载规则同步副本；再核对工单版本、目标组织及普通用户的功能权限／data grant。Phase21前端调用真实成员目录和Rust写入，提醒独立发布授权见下一节；实现存在不能代替对应实际浏览器／平台验收，也不切换生产流量。

Phase19自身未改变schema、Cargo依赖及0001／0002。其历史回退边界是：Phase18恢复显式非null assignee返回400、目录503，带非空assignee的创建正文也不能由原Phase18重放；已保存分配与receipt必须保留，省略更新仍可保留旧快照。升级Phase20的0003后还须同时满足下一节的迁移兼容条件，不能直接使用原Phase18／19构建。若平台也回退到不认识memberLookups的旧严格解析器，需先移除新配置字段并同步旧版本配置；旧成员边界须另行审阅，不把删字段视作保持新默认拒绝规则。

<a id="reminder-recovery"></a>
## 独立提醒、持久意图与恢复

`POST /work-orders-api/tickets/{id}/remind` 只接受 `application/json` 对象 `{idempotencyKey}`，禁止未知／重复字段和query，正文最多64 KiB；键为1–128个ASCII `[A-Za-z0-9._:-]` 字符。这是独立提醒事件键，与创建请求头 `Idempotency-Key`、requestId和工单version分别保存。提醒不重复保存工单，不修改工单、版本或创建receipt。

一次当前内省／context必须同时提供 `ticket:read`、`ticket/read`、`notification:publish` 和 `notification/publish`。服务先按当前tenant、未删除工单、read grant及组织硬上限读取，再检查接收组织与用户是否在独立publish grant范围内；SELF发布仅允许当前用户和当前组织，其他scope仍与硬上限取交集。每次恢复都重新检查这些条件，历史意图或发布成功不能绕过当前撤权、软删除或范围变化。管理配置与普通用户权限见 [提醒授权](data-grant-management.md#notification-reminders)。

本地短事务用scope内 `FOR SHARE` 取得工单快照，收件人为当前处理人，无处理人时为创建人，目标组织为工单组织。提醒正文固定为 `target:{kind:"USER",userId,organizationId}`、`type:"INFO"`、`title:"工单提醒 <编号>"`、`content:"请查看工单 <编号> 的详情并处理。"` 和 `link:"/micro/work-orders/tickets/<UUID>"`，并包含原 `idempotencyKey`；标题、状态和version编辑不改变该正文。

0003的意图唯一域与平台一致：tenant／发布者当前organization／可信application UUID／发布user／key。它保存ticket、目标组织与收件人、完整类型化平台正文及SHA-256，禁止更新、删除、TTL清理或随工单级联删除，不保存token、成功回执或发送队列。同域同键只允许同一工单及相同有效目标／完整正文；即使最初平台POST从未提交，已确认持久化的意图也会使换工单或换收件人的请求返回409 `IDEMPOTENCY_CONFLICT`。将处理人改为原owner且有效目标未变，不凭字段变化制造冲突。

意图事务确认COMMIT并释放连接和行锁后，每个传入请求最多向 `POST /api/apps/work-orders/notifications` 发出一次POST，使用当前普通应用Bearer和持久意图原文。服务不自动补发、不换键、不直接写平台通知表，也不另查成员阻断平台历史回执恢复。平台的新发布才检查收件资格；已有相同publication按当前发布者授权和目标范围恢复原历史回执，不重新枚举当前收件人。

| 情况 | 当前结果与恢复动作 |
| --- | --- |
| 意图COMMIT未确认，或请求总期限在平台POST尝试前耗尽 | 503 `REMINDER_PREPARATION_UNKNOWN`；本请求零次平台POST，保留原键。下一次若有意图必须遵循它；若确无已持久化意图，可按当前快照首次准备，未提交的准备不能永久锁定旧目标 |
| 已确认意图并尝试POST后网络、超时、5xx、无效200或期限过期 | 503 `REMINDER_RESULT_UNKNOWN`；可能已发布，保留同键和已固定的完整正文，用当前有效凭据显式恢复，不自动换键重发 |
| 平台明确400／401／403／404／409 | 对应400 `VALIDATION_ERROR`、401 `INVALID_APP_TOKEN`、403 `DATA_DENIED`、404 `RECIPIENT_NOT_FOUND` 或409 `IDEMPOTENCY_CONFLICT`；修复身份／授权或核对事件，不把拒绝当成成功 |
| 意图结构、类型化正文或hash损坏 | 503 `DATABASE_UNAVAILABLE`；停止平台调用，保留数据供受控核对 |

成功200仅投影已校验的五个字段：UUID `publicationId`、`recipientCount:1`、`deliveryCount:1`、布尔 `duplicate` 和RFC3339 `createdAt`。调用者以接收者真实平台session在其当前组织查询 `GET /api/notifications`，按publicationId核对持久收件箱。Socket.IO服务未启动、用户离线或未出现Toast，不会使已提交HTTP发布变成持久失败；删除收件箱消息后重放原publication不会补建被删除的delivery。

业务库意图与平台通知不是跨库原子事务：本地COMMIT后到平台提交之间工单仍可变化或删除。平台200已通过验证后，不重新读工单并因其变化把成功改成失败。恢复仍先检查当前工单read／publish授权；不可见或已删除工单不能凭历史事件绕过检查。

0003不修改0001／0002或增加Cargo依赖。原Phase19严格迁移基线不认识新表和历史，不能直接回退旧二进制；回退必须保留不可变意图与平台publication，先对账在途结果，再使用接受0003和相同事件语义的兼容构建。旧Node迁移runner会执行0003，但旧提醒handler不使用该意图，不能据迁移成功宣称有Phase20的目标固定和准备恢复保证。禁止删意图／receipt、修改checksum或清除迁移历史绕过校验。Phase20不实施前端恢复；当前Phase21消费者的内存边界见下一节，不迁移实时服务或执行部署切流。

<a id="frontend-recovery"></a>
## Vue／Vite消费者的未知结果恢复

Phase21前端以登记SDK1.0.0消费本服务，标准Vue模块／Vite相对入口和动态chunk由静态Nginx提供；业务调用保持opaque iframe的Origin:null、当前应用Bearer、UUID requestId及 `credentials:omit`，不增加allow-same-origin、下载sandbox权限或SDK能力。公开构建配置仅含应用版本、精确宿主origin与API地址，不含client secret、平台session或业务DB URL。独立安装与产物说明见 [前端README](../business-apps/work-orders/frontend/README.md)。

`IntentStore`仅在同一仍存活iframe的模块内存中，按appCode／tenantId／organizationId／userId隔离；不保存token或整份props，不使用opaque localStorage。创建固定一个未完成事件的键与完整DTO，提醒按工单各自固定事件键。路由往返、同业务身份授权续期及切到其他组织再返回，可恢复本域保留的合法草稿／事件；换域不会复用其他域的键或显示其服务器内容。刷新、关闭页面、销毁iframe或切换前端发布实例不保证保留这些内存数据，结果未知时应保持页面直至核对完成。这与业务数据库0003的持久intent是不同层次，不能把浏览器内存描述为持久存储。

| UI遇到的结果 | 当前恢复流程 |
| --- | --- |
| 创建网络／协议／5xx、畸形2xx或CREATE_RESULT_UNKNOWN | 保留并锁定首次键与正文，提示可能已提交；用户显式“恢复原事件”发送同键同正文，预期201及完整合法工单DTO校验后才结清；不自动重复创建或用编辑后的正文换键 |
| 提醒REMINDER_PREPARATION_UNKNOWN | 明确提示本次尚未向平台POST，保留原事件并显式重试；后端是否已持久intent及后续首次准备边界仍按前述契约 |
| 提醒网络／协议／5xx、畸形2xx或REMINDER_RESULT_UNKNOWN | 提示可能已发布，用户显式“重试提醒”沿用同事件；只有预期200及合法五字段回执才结清，409先刷新并确认当前目标，再由用户选择独立新提醒 |
| PATCH／状态／DELETE的WRITE_RESULT_UNKNOWN | 不自动重发mutation；先读取当前详情／状态或核对记录可见性，编辑保留输入并明确采纳当前version后再决定保存 |
| 版本409 | 保留输入，读取最新内容，用户明确选择最新修订后再提交；不静默覆盖并发修改 |

授权替换、到期和切组织立即取消旧请求并隐藏列表、详情、成员及服务器快照；同身份合法草稿与未知事件可留内存，当前权限不成立时不能继续提交。401停用视图，read的403／404隐藏相应记录，写403暂停操作；普通网络故障与409不冒充认证失效。纯主题更新不额外请求列表或取消写入，路由离开只取消相应读取，迟到结果不得覆盖新路由或组织。创建路径 `/micro/work-orders/ticket-create` 与详情 `/micro/work-orders/tickets/:id` 各要求精确MENU和独立权限／grant，恢复事件不扩大授权。

采用新前端须使用新的不可变版本并保留旧资源；旧前端不发送创建header，不能作为当前Rust创建／未知恢复契约的等价回退对象。版本指针回滚不会迁移业务数据，也不能恢复已销毁iframe的内存事件；回退前先核对在途创建、版本写入与提醒结果，不能删receipt或intent解决界面兼容。实际验收由 [Phase21](validation/phase-21.md)报告记录，不预记通过。

## 筛选、分页与 CSV

| 参数 | 当前规则 |
| --- | --- |
| `q` | 去除两端空白后最多200个 Unicode 字符；按标题／描述／编号字面包含搜索，`%`、`_`、反斜杠作为普通字符 |
| `status` | 精确 `OPEN`、`IN_PROGRESS`、`RESOLVED` |
| `priority` | 精确 `LOW`、`NORMAL`、`HIGH` |
| `organizationId` | 标准连字符 UUID；与授权范围取交集，不扩大 grant 或硬上限 |
| `page` | 仅列表，默认1，1–1000000的十进制整数 |
| `size` | 仅列表，默认20，1–100；最大 offset 为99999900 |

查询串最多8192字节；未知、重复、非法百分号／UTF-8／NUL、越界或非整数字段均拒绝，不静默修正。详情不接受查询参数；CSV 只接受四种筛选字段，不接受 page／size。排序固定 `created_at DESC,id ASC`。列表 count 与 records 在同一个 `REPEATABLE READ READ ONLY` 快照使用相同授权和筛选，避免并发写入使总数与当前页来自不同快照；CSV 的容量预检和记录读取也共用一致快照。

CSV 最多1000行工单，超过返回400 `EXPORT_LIMIT`；最终编码最多4 MiB，超过返回413 `EXPORT_LIMIT`，均要求缩小筛选范围，不截断。容量包括 UTF-8 BOM、表头、CRLF、所有双引号及转义扩展。所有单元格加双引号、内部引号加倍；带空白／控制字符的公式前缀 `=`、`+`、`-`、`@` 及起始控制字符在引号内加单引号中和。导出列为编号、标题、描述、优先级、状态、组织、创建人、处理人、创建／更新时间；枚举显示中文，文件名固定 `work-orders.csv`，不从请求拼接响应头。

## 验收与恢复

真实PG测试在 `tests/migrations.rs`、`tests/read_contracts.rs`、`tests/write_contracts.rs`、`tests/member_contracts.rs` 和 `tests/reminder_contracts.rs`，默认明确ignored，须由拥有临时PostgreSQL的受管驱动显式执行，不把默认跳过算通过。`TEST_DATABASE_URL` 与 `CI_TEST_POSTGRES_OWNER` 证明资源归属，仅创建／删除自身随机库。写入／成员／提醒套件分别核对事务、receipt、version、scope、TTL、intent与未知结果；实际选择和结果以阶段报告为准。提醒受控HTTP记录不是真实平台收件箱，平台 `backend/src/phase20_work_orders_tests.rs` 单独验证真实发布与接收者session恢复；两者均不代表生产切流。旧Node仅作为兼容测试输入，Rust独立运行另行验证无Node。

结构或历史不匹配时停止启动，先核对版本及迁移，不能改checksum、删除历史、重建旧库或重跑已执行CREATE。升级0003后，回退构建必须接受0001／0002／0003双历史、创建receipt和提醒意图语义。Node兼容版本同样须核对完整迁移序列及提醒handler差异，不能清除SQLx元数据、receipt或intent绕过校验。迁移事务或提交结果未知时，下一次启动重新检查真实记录，不能因报错就认定未提交；本说明不授权生产升级或回退。

本阶段不切换生产流量，不新增生产容器路线。库接管历史见 [Phase16](validation/phase-16.md)，读取／CSV见 [Phase17](validation/phase-17.md)，写入／0002／幂等及未知结果恢复见 [Phase18](validation/phase-18.md)，成员目录／策略／分配及TTL竞态见 [Phase19](validation/phase-19.md)。提醒、0003意图、当前授权、未知结果恢复及真实平台收件箱的实际HTTP／SQL／联合与CI结果由 [Phase20](validation/phase-20.md)报告记录；本文不预记本轮门禁通过或阶段完成。
