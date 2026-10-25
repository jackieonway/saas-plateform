# Phase 6 写入器、实例与活动消费者就绪审查

审查日期：2026-10-25。范围为本身份功能的新库及当前工作区源码，依据 [Phase 6 台账](task-ledger.md)、[Phase 1 路径清单](phase-01-membership-path-audit.md) 和 Phase 1～5 已有验收。本文记录代码路径、实际执行的只读检查及尚须主流程验收的条件，不以源码搜索或 SDK 测试代替真实收缩迁移、后端安全回归和环境启用决议。

## 结论与状态边界

- 当前活动成员写入全部收敛到平台：账号创建、员工批量维护、联系人管理员、seed 与显式生命周期。未发现生产成员写入仍依赖二元 `ON CONFLICT(user_id,organization_id)`；历史迁移、隔离 fixture 和独立 UUID 恢复流程分别列出，不改写为当前生产写入器。
- 普通员工入口以显式 `EMPLOYEE` 唯一类别或精确 `membership_id` 定位；授权、凭据与通知的完整成员／版本证明由平台权威核验。按 user/org 查询不等于有歧义，须核对是否同时限定唯一成员类型、只做集合存在性／统计，或属于明确的账号级操作。
- 本轮发现组织树 `userCount` 按成员行数计数的遗漏，已改为关联账号去重；审计 `memberCount` 继续计算关系行数。两套现有组织范围 PostgreSQL fixture 已补同账号同组织双行回归，等待根统一运行，不能仅因格式检查通过标记数据库验收完成。
- SDK 3.0.0 的 20 个生成产物检查及 30 项测试实际通过。两个业务仓库、通知与 examples 的本轮工作为源码审查，没有自行重跑 Cargo、业务服务、浏览器或数据库。
- 本任务没有登记或部署身份新库的生产平台／业务实例，也没有需迁出的历史身份库或旧凭据。本任务的旧 binary 清退登记为不适用；可见的 UUID 预览／恢复实例属于另一专项，不能据其存在宣称本身份新库已有生产部署，更不能操作它们完成本台账。
- 创建开关保持关闭。代码审查记录不是开启授权；独立收缩迁移、Phase 1～6 安全门禁、活动契约和显式环境就绪条件全部通过后，才能按实际目标环境开启。正式结论与实际命令由 `validation/phase-06.md` 汇总。

## 扫描方法

从 `backend/` 追踪以下实际源码搜索，再逐个读调用、参数绑定、事务末端复核及活动入口；没有只凭匹配行判定安全：

```powershell
rg -n -i 'INSERT\s+INTO\s+memberships|UPDATE\s+memberships|DELETE\s+FROM\s+memberships|ON\s+CONFLICT\s*\(\s*user_id\s*,\s*organization_id\s*\)' src tools ops -g '*.rs' -g '*.sql' -g '*.mjs' -g '!**/target/**' -g '!**/*tests*' -g '!**/release_tests/**'
rg -l 'memberships' src tools ops -g '*.rs' -g '*.sql' -g '!**/target/**' -g '!**/*tests*' -g '!**/release_tests/**'
rg -n 'REFERENCES memberships|memberships_user_id_organization_id_key|trg_membership|revoke_sessions_on_membership' migrations -g '*.sql'
```

跨仓扫描覆盖 stores、work-orders、notification、SDK 与五个 examples。排除生成／固定归档与构建目录后审查活动源，再沿真实入口复核必要的固定合同来源；没有把 vendor 的历史副本误作本阶段可改源码。`fetch_one`、`LIMIT 1`、`first()` 和二元业务幂等键均作语义分类。

## 平台写入器与批任务

下表的“代码就绪”指当前源码具备双身份边界；它不证明任何未知部署已安装该版本。平台源码及迁移 owner 仍为当前平台进程，具体的认证存储 owner 拆分属于 Phase 7。

| 写入器／入口 | 代码证据与选择语义 | 收缩前后处理 | 本轮判断 |
|---|---|---|---|
| `POST /api/users` | [users.rs](../../src/routes/users.rs) 显式插入 `EMPLOYEE`、应用生成 v7；创建新账号后加入明确组织，不接受把普通 DTO 改为客户创建 | 新账号不存在同账号另一关系；组织和管理范围仍由服务端核验 | 代码就绪；客户创建继续走独立生命周期 |
| `PUT /api/users/{id}/memberships` | 同文件读取、DELETE、UPSERT 全部限定 `member_type='EMPLOYEE'`；冲突目标为 `(user_id,organization_id,member_type)`；保留已存在 ID／状态，重复组织输入返回 409 | 客户和管理范围外关系不参加员工替换集合；删除当前员工身份采用精确 ID 的最后一步；角色支持集合只认剩余有效员工 | 代码就绪；双行 bulk／局部撤销由阶段安全回归验证 |
| 联系人管理员初始化 | [onboarding.rs](../../src/routes/onboarding.rs) 的 `provision_contact_administrator` 使用显式员工三元冲突键；读取有效员工资格后授角色，停用员工返回冲突 | `ensure_employee_creation_allowed_in_tx` 在第二身份关闭时拒绝仅有客户的账号产生员工第二行；不会把客户行当员工复用，也不会隐式启用旧员工 | 代码就绪；无原地类型修改 |
| 开发 seed | [seed.rs](../../src/seed.rs) 的 `run` 与 `insert_membership` 只插显式员工、v7；没有成员 UPSERT 或客户 seed | 只用于既有 seed 合同，不成为客户／第二身份创建旁路 | 代码就绪；本轮未执行 seed |
| 空生产库引导 | `seed::initialize` 在锁内确认账号数为零后使用同一个员工插入函数；已有账号时转只读凭据检查 | 不向既有成员集合猜测性补行；不是旧实例身份迁移器 | 代码就绪；不宣称已运行本任务生产初始化 |
| 显式创建、状态、删除、第二身份与替换 | [membership_lifecycle/service.rs](../../src/membership_lifecycle/service.rs) 精确 `id`、类型和并发版本；类型不可原地修改；账号锁与成员锁后再验 actor、管理范围、期限和权限 | 新关系完整记录账号绑定确认／来源与 reason；目标已停用时必须明确启用并清理旧授权，替换才停用源；账号和真实组织必须存在 | 代码就绪；三个创建开关仍受配置和收缩门禁约束 |
| 用户密码、状态与全账号删除 | [users.rs](../../src/routes/users.rs) 锁全局用户行，检查全部归属范围；[session.rs](../../src/session.rs) 提升账号版本并全账号撤销 | 这些是明确的全账号操作，允许覆盖该账号两种身份；不得用作单成员退出接口 | 代码就绪；与成员局部撤销分开 |
| 租户／组织删除和部门维护 | [tenants.rs](../../src/routes/tenants.rs)、[organizations.rs](../../src/routes/organizations.rs)、[departments.rs](../../src/routes/departments.rs) | 租户／组织级联删除覆盖辖内所有关系是原合同；部门外键调整资料，不修改成员类型；审计 `memberCount` 保持关系数 | 代码就绪；全实体操作不伪装为客户局部删除 |
| `platform-admin` CLI | [grants.rs](../../tools/platform-admin/src/grants.rs) 使用 manifest 的明确 `membershipId` 核对员工专用成员端点，数据授权 target 含完整五个实体 ID；API 对每次写入重新授权 | CLI 没有成员创建、删除、批量导入、任意 SQL 或 Docker 控制命令；bootstrap／access-policy 只使用管理 API，机器客户端与员工应用用途分别核对 | 没有独立成员 writer；不是绕过生命周期的入口 |
| artifact／deployment agent、监控和后台发布任务 | [tools](../../tools) 与 [ops/monitoring](../../ops/monitoring) 中未发现平台成员表写入 | 发布、制品、部署任务不迁移身份表，不为应用授权创建成员 | 无成员 writer |
| 成员 CSV／业务导入器 | 当前 routes、工具与服务未发现此类活动入口；审计 CSV 是导出 | 将来新增入口必须登记并复用生命周期、开关与账号绑定依据 | 当前不适用，不预设未来导入安全 |

### 独立 UUID importer 的范围

[ops/recovery/src/uuid_rekey/import.rs](../../ops/recovery/src/uuid_rekey/import.rs) 及其 COPY planner 能处理固定注册表中的成员数据，但只接受固定 U04 三目标、私有证据包、owner／catalog／schema 摘要和独占模式。它属于独立预览恢复任务，不是身份新库的活动导入器。本轮没有运行 COPY、读取包或凭据、改变它的固定目标、部署或 schema；不得拿它给身份新库导入旧成员或绕过本阶段开关。

## 二元读取、删除、触发器与广播分类

| 匹配路径 | 实际边界 | 判断 |
|---|---|---|
| `routes/auth.rs` 的登录列表、选择与数组首项 | 登录首个可选员工仅用于登录审计的组织字段，不建立 context；选择票据快照具体 member／generation。context 只有候选恰好一项才读取 `[0]`；普通控制台固定员工用途 | 没有取任意首成员签发凭据 |
| `routes/oauth2.rs` password grant | 显式 `membership_id` 或唯一有效候选；候选受 client 允许类型过滤，数量不为 1 即拒绝；签发再次捕获精确 binding | 没有最早成员／最早客户端兜底 |
| `session.rs`、`credential_binding.rs`、`oauth_issuance.rs`、`app_token.rs` | 持久载体包含具体成员、账号／成员／client／app 策略版本；权威状态和用途交集实时核验，缺失证明不补绑 | 不依赖旧二元唯一键 |
| `authorization.rs`、`app_authorization/store.rs` | 员工投影先拒绝非员工；完整 session/member 与 generation 相等；角色祖先资格只由有效员工支持 | 同组织客户不借员工角色，也不使有效员工被误拒 |
| `data_grants/store.rs`／`discovery.rs` | target 为具体 member，actor 为实时员工证明，候选为有效员工 | 不按账号合并两种身份的数据授权 |
| `routes/users.rs` 的账号范围 EXISTS | 账户全局资料与管理范围查询允许看到该账号的全部关系，以拒绝越范围全局修改；不是员工授权证明 | 按账号操作的明确集合语义，不是首行身份定位 |
| `routes/roles.rs`、`routes/app_members.rs`、onboarding 的员工资格 | 目标查询限定有效 `EMPLOYEE`，用户角色与角色用户双向均核验资格，员工目录目标组织受策略限定 | 不把客户当员工候选 |
| `routes/search.rs` 的 `LIMIT 1` | 员工管理员搜索的账号摘要只展示管理范围内一个组织名；不建立会话、owner 或角色资格 | 展示字段，不作为授权依据 |
| tenant／monitor／organization 的 `userCount` | 关联账号人数去重；`memberCount` 为身份关系行数；员工目录／角色人数另有员工资格门禁 | 本轮修复组织树的重复人数遗漏，未把客户计为员工资格 |
| 成员 DELETE／status 触发器 | 追加 [0014](../../migrations/0014_credential_identity_binding.sql) 将删除定位为 `OLD.id`，身份版本／状态变更撤销该具体成员派生载体；账号版本变更仍全账号撤销 | 同组织另一身份不被局部变更误撤 |
| `notifications.rs` 发布、SYSTEM 与提示 | 真实生产发布受众和 SYSTEM lookup 只取有效员工；通知收件 proof 不回填。`system_in_tx` 对传入 `(user,tenant,org)` 去重后再次选择唯一有效员工，并写具体 member／generation | 上游集合查询即使遇到双行，不产生客户消息或重复投递；不能只看上游查询无类型就误判最终广播 |
| 关系外键与自然键 | 原 migrations 中成员被引用的外键为 `membership_data_grants.membership_id → memberships.id`；部门组合外键引用 departments 的组合键 | 源码未发现引用旧 `(user_id,organization_id)` 唯一键的 FK；真实 catalog 依赖检查和收缩执行仍须独立迁移门禁证明 |
| 历史 `0001` 二元键／旧触发器 | 为升级链保留的历史字节；当前函数被后续追加迁移替换 | 不改历史迁移，也不把这些匹配行称为活动旧 binary |

### 通知兼容传输基础的明确边界

Phase 4 已验收合法 platform session 的类型中立 inbox／偏好／Socket.IO 完整成员隔离，包括合成客户收件行下的双向局部撤销。该基础保持不变；它不意味着首版开放客户发送用途。生产 APP 发布与 SYSTEM helper 仍只产生员工收件行，默认广播不发送客户，客户不能读员工收件行或旧代消息。

Phase 6 新公开客户路径只接受真实 OAuth access 的明确 client／成员绑定，不能把它变成控制台 platform session，也不能直接喂给 `NotificationContext`。本阶段只开放登记的最小客户 profile／context／应用目录用途，不创建客户通知业务。现有 OAuth userinfo／introspection 是既有协议身份信息，客户角色投影为空，不因此放开员工业务 API。通知内部 11 字段 proof、8 字段提示与固定公开 Notification 1 是不同合同，不能通过改 SDK 固定 DTO 来混同。

当前新增源码为 [customer_identity](../../src/customer_identity)，实际只读入口为 `GET /api/customer/context`、`GET /api/customer/profile`、`GET /api/customer/applications`，合同见 [客户 profile 说明](phase-06-customer-profile.md)。登记用途固定为 `CUSTOMER_PROFILE`，入口为 `/api/customer/profile`，成对配置的 client／application 主键精确定位唯一 USER client 和 application；缺登记返回 403，不选择首 client 或默认 app。只接现有 `/oauth/token` 真实签发、`token_use=oauth` 的 access，逐次核验 profile scope、当前客户成员、来源 client 与 app 的类型交集和版本、租户组织授权。任意非空 query 不允许代读，profile 只含本人最小资料，应用目录只返回该登记 app。该模块与六项真实 PostgreSQL／HTTP 测试已落源码，本文编写时仍等待根统一 Cargo 验收。

## 跨仓活动消费者

| 仓库／入口 | 当前合同与就绪证据 | 本轮执行边界 |
|---|---|---|
| stores Rust API | [http.rs](../../../../saas-stores/backend/src/http.rs) 逐请求平台授权；[directory.rs](../../../../saas-stores/backend/src/directory.rs) 只登记 store/create、store/update、store-staff/read。创建与更新最终非空联系人均取短效员工 witness，包含显式相同 ID；锁后及提交前复核 | 无本地平台 memberships；旧 store_staff 业务关联表已冻结写入，旧 staff 写入口授权后 410；schema `--import` 不导入成员数据。未跑 Rust／PG |
| work-orders Rust API | [http.rs](../../../../saas-work-orders/backend/src/http.rs)、[writes.rs](../../../../saas-work-orders/backend/src/writes.rs)、[directory.rs](../../../../saas-work-orders/backend/src/directory.rs)；显式相同处理人 ID 仍取有效员工 witness，锁后核对 preflight，提交前后复核期限 | 本地 user/org 冲突键为工单／提醒幂等，不定位平台成员；迁移 CLI 不导入成员；Node `index.mjs` 明确退休。未跑 Rust／PG |
| 两业务服务的 legacy／combined 授权 | 平台 app_authorization 两次拒绝客户，load query 精确 session/member；app_members list／one 都是有效员工。Context 1／授权 1.0 未增私有成员字段是保留员工旧合同 | stores PG fixture 的 combined authority 与工单 PG fixture 的 legacy authority 是受控 HTTP fixture；不冒充真实平台 PG 授权证明。Phase 6 主门禁仍需真实平台权威 |
| notification Rust 服务 | [protocol.rs](../../../../saas-notification/src/protocol.rs)、[store.rs](../../../../saas-notification/src/store.rs)、[server.rs](../../../../saas-notification/src/server.rs)；完整 proof 固定连接、持久行和六维路由分区，逐条再次问平台 | 不写平台成员或收件箱、不持签名 key；只读 recipient proof 列＋LISTEN。当前合法客户空连接基础如上一节，不计新增客户业务；未重跑服务合同 |
| 平台 React 宿主 | Phase 5 的完整 server proof、contextKey／epoch／请求与偏好隔离；[use-notification-socket.tsx](../../../frontend/src/hooks/use-notification-socket.tsx) 仍发送 `{token}` 并消费公开 5／12 字段 | 不是成员写入器；新客户先用独立 DTO，不能解 JWT 猜 member 或拓宽现有员工应用 |
| SDK 当前生成器 | [release.json](../../../../saas-micro-app-sdk/release.json)、[build.mjs](../../../../saas-micro-app-sdk/tools/build.mjs)、[generate.mjs](../../../../saas-micro-app-sdk/tools/generate.mjs)；3.0.0，MicroProps 2／Context 1／Notification 1 | 只消费宿主 context／token／epoch／signal；无平台 DB、成员选择、refresh 或客户端选择。生成检查在临时 workspace，不写固定历史发行字节 |
| Vue／jQuery／报表中心 | 各真实 index／Vite entry 使用宿主 props 和 SDK；报表外部角色请求使用应用 bearer，省略 Cookie，context 变化取消请求 | 无成员 writer、成员首项猜选或客户注册旁路；不把服务器返回的展示 clientId 当授权选择 |
| WebSocket example | [src/main.tsx](../../../../saas-examples/websocket/src/main.tsx) 实际导入根 `frontend.tsx`；匿名聊天后端只按 socket.id 管理进程内 ChatRoom | 根 frontend.tsx 是活动源；ChatRoom membership 不等于平台 membership；没有 DB writer 或把宿主 context 当聊天身份的路径 |
| OAuth playground | 四个流程分别配置明确 CLIENT_ID；`clients[flow.index()]` 不存在就拒绝。授权、交换、refresh、内省和吊销沿同一个 Client，浏览器只选枚举 flow | Cookie／token 状态只在本服务有界内存，不转发平台 bearer，不选“最早 client”。真实 USER／MACHINE 和成员用途必须由平台登记与核验；本轮未运行服务 |

examples 的 [dependencies.lock.json](../../../../saas-examples/dependencies.lock.json) 与当前活动来源均记录 SDK 3.0.0。本轮仅将 [oauth-playground/README.md](../../../../saas-examples/oauth-playground/README.md) 的旧 2.0.0 描述校准为 3.0.0，MicroProps 2.0 不变；没有重新发行 SDK、改归档摘要或固定副本。

## 实例清单与旧 binary 清退

本轮只读执行 WSL Ubuntu 的 `docker ps -a`，输出仅容器 ID、名称、镜像、状态与端口；随后仅检查选定容器的 labels，没有读取 Env、连接 URL、密码、进程参数或业务数据库。

| 可见资产 | 只读事实 | 本身份新库决议 |
|---|---|---|
| `identity-sso-01a16e22-pg` | `postgres:17.6-bookworm`；仅 `127.0.0.1:55438→5432`；labels 为空。本任务已有 Phase 4 证据明确它是独占测试集群 | 测试基础设施，不是生产平台／业务 binary，不借空 labels 编造生产 owner |
| `qiankun-uuid-u06-recovery-*` 的 gateway、platform、stores、work-orders、notification、examples-chat／oauth | 审查时为运行状态；选定 platform／notification labels 为 `saas.uuid-stage=U06`、`saas.uuid-generation=recovery`、`saas.phase04.owner=ci-toolchain-bootstrap` | 独立 UUID 专项恢复资产，不属于本身份新库，不启动／停止／替换／迁移这些服务 |
| `qiankun-uuid-u06-final-*`、`qiankun-uuid-u05-*`、早期 preview/local 容器 | 审查时为已退出状态；名称和用途与独立预览／UUID 任务一致 | 不能作为本任务已部署旧实例或已完成生产清退的证据 |
| `saas-uuid-u02-pg17-20261024` | 独立 UUID 测试 PostgreSQL，`127.0.0.1:55437→5432` | 非本任务的当前身份测试集群，不据此推断创建开关或部署就绪 |

台账明确本身份功能为新库，无历史凭据／用户迁移。本任务尚未提供生产环境、生产部署清单或外部进程 owner，因此本阶段**本范围旧 binary 清退为不适用**；这不是对不可见远程环境不存在旧版本的证明，也不等于当前源码已部署到上述其他任务实例。未来出现实际部署目标时，必须登记 owner、准确 binary／镜像版本、迁移与配置，以及所有活动 writer 的安装状态，再执行启用检查，不能直接复用本表冒领。

## 本轮真实验证与待汇总门禁

实际使用 Windows `D:\Software\Scoop\apps\nodejs\current\node.exe`，版本 **24.22.1**。SDK 根目录执行：

| 实际命令 | 结果与边界 |
|---|---|
| `node tools/build.mjs --check` | exit 0；`SDK 3.0.0: verified 20 generated artifacts`；没有 `--write`，没有写真实归档／vendor |
| `node --test tests/*.test.mjs` | exit 0；**30/30 PASS**；约 **5.22 秒**。含 archive 不变、确定性生成和独立消费者类型检查 |
| 限定组织统计源码与回归文件的 `rustfmt`／`rustfmt --check` | exit 0；本代理未运行 Cargo |
| 限定变更文件的 `git diff --check` | 通过；Windows 原有 CRLF 提示不代表检查失败 |

组织统计回归在 `src/main.rs` 注册，主流程的定点命令应为：

```sh
cargo test --locked --bin saas-platform-backend organization_subtree_reporting_tests -- --ignored --nocapture --test-threads=1
```

该命令在本审查代理中**没有执行**；数据库 URL 必须由根显式选择已核实测试集群，不能使用 `.env`、预览或默认实例。本轮测试增量只在自有随机库构造目标 schema 的双关系 fixture，不替代独立收缩 migration 的真实执行证据。根的全 binary ignored 门禁可同时覆盖两项。

### 主流程最终汇总

以上“本代理未运行 Cargo”“等待根统一验收”等说明保留初次审查时点，不能据此把后续主流程的真实结果遗漏，也不能改写为本代理执行了这些命令。主流程现已完成本阶段安全门禁，以及新建 `development`／`TASK_OWNED` 目标的真实收缩、receipt 启动和 HTTP 验收；完整命令、失败及修复重跑过程以 [Phase 6 验证记录](validation/phase-06.md)为准。

| 主流程项目 | 最终实际结果与安全证据 |
|---|---|
| 活动 SDK 与仓库合同 | SDK 3.0.0 [20 个生成产物](../../../local-identity-sso-phase06-qa/sdk-check.log)及 [30／30 Node 测试](../../../local-identity-sso-phase06-qa/sdk-tests.log)通过；[repository contracts 10／10](../../../local-identity-sso-phase06-qa/repository-contracts.log)通过。MicroProps 2／Context 1／Notification 1 保持既有合同，没有重写固定发行字节。 |
| 组织人数双关系回归 | 普通租户和平台租户的两项 `organization_subtree_reporting_tests` 在 [main ignored 原始日志](../../../local-identity-sso-phase06-qa/backend-full-baseline.log)均真实 PASS，同账号同组织员工＋客户仍为三账号四关系，组织树 `userCount=3`，具体成员关系保留。该轮另两项准入 fixture 失败没有被抹去，后续定向复验另记。 |
| 后端与客户安全门禁 | [最终门禁](../../../local-identity-sso-phase06-qa/rollout-final-gates.log)记录格式／check／Clippy、193 项普通测试、三项 rollout PostgreSQL 专项、最终 bins 构建及文档通过；客户真实 OAuth／PostgreSQL／HTTP **6／6** 通过。main 的 178 项 ignored 按首轮 **176 PASS／2 FAIL** 与 [准入修复后定向 2／2 PASS](../../../local-identity-sso-phase06-qa/policy-final-gates.log)取得逐项分组通过证据，没有宣称修复后重跑全部 178 项。 |
| 通知服务与平台权威 | 本阶段实际重跑 [notification 36 项普通＋3 项 PostgreSQL](../../../local-identity-sso-phase06-qa/notification-gates.log)并通过构建／文档；平台 [9 项通知 PostgreSQL](../../../local-identity-sso-phase06-qa/backend-full-baseline.log)全部 PASS，含真实平台与外部 Socket.IO 服务、成员与代际隔离及局部撤权。初审表中的“未重跑”只描述审查代理自身，本次不因此声称另外两个业务仓重新执行了 Rust 测试。 |
| 前端门禁来源 | 本阶段前端源码未变，[456／456 测试](../../../local-identity-sso-phase06-qa/frontend-tests.log)、[构建](../../../local-identity-sso-phase06-qa/frontend-build.log)与 [lint](../../../local-identity-sso-phase06-qa/frontend-lint.log)明确继承 Phase 5 的真实证据；没有包装成 Phase 6 新运行。 |
| 真实资产与十项 gate | [已审查 inventory](../../../local-identity-sso-phase06-qa/inventory-review.json)和 [真实 manifest](../../../local-identity-sso-phase06-qa/development-final-evidence/manifest.json)包含 **7 项资产**，覆盖五种 kind：一个实际 backend 文件资产、真实 migrate／rollout 两项 CLI 为 READY，未在本目标部署的 platform-admin、importer、外部 batch、offline job 各有 NOT_APPLICABLE 理由，没有编造退休实例。[十项 gate 汇总](../../../local-identity-sso-phase06-qa/gate-results.json)的 **34 个原始 artifact 引用**已逐个核对磁盘 SHA；`phase_06` 指源代码与安全测试通过，最终操作证据另列，不用 receipt 自证。 |
| 独立收缩与启动负例 | 新库 `identity_phase06_development_01a16ef0f9cd7025aef9ccb4b6e8a2bd` 的实际 OID 为 `1158337`、cluster system identifier 为 `7700230159816609830`；[目标事实](../../../local-identity-sso-phase06-qa/development-final-evidence/target.json)、[真实 apply](../../../local-identity-sso-phase06-qa/development-final-evidence/results/apply-report.json)与 manifest 绑定最终后端 SHA，取得 receipt `01a16ef5-6465-7473-9446-34a64a61dfd6`。缺 receipt、错误 manifest SHA、错误 canonical receipt 三项真实启动均非零拒绝，原始文件保存在 [首次操作证据](../../../local-identity-sso-phase06-qa/development-first-attempt/results)。 |
| 首次失败与同目标恢复 | [首次正确 proof 启动](../../../local-identity-sso-phase06-qa/development-first-attempt/results/development-start.json)已观察到 `MEMBERSHIP_READINESS_VERIFIED`，随后地区 `COPY regions` 被默认 5 秒 statement timeout 取消，退出 1、health 未达；该失败保留。仅对自有 development 服务显式采用 60 秒 statement／30 秒 lock 初始化预算，保持原 binary、manifest 和 receipt，通过新的 [只读恢复预检](../../../local-identity-sso-phase06-qa/development-final-evidence/attempts/resume-1416df0c213f79be/results/resume-preflight.json)后执行 `resume-start`，**没有再次 apply**。[新启动结果](../../../local-identity-sso-phase06-qa/development-final-evidence/attempts/resume-1416df0c213f79be/results/development-start.json)记录 health 200／UP、readiness 已验证、正常关闭和退出 0。 |
| 收缩后的真实 HTTP 与关闭后核验 | [30 项 HTTP 检查](../../../local-identity-sso-phase06-qa/development-final-evidence/attempts/resume-1416df0c213f79be/results/http-smoke.json)全部 PASS：同账号同组织独立员工／客户、同类型重复 409、并发 200／409、绑定确认／未知组织拒绝、员工三元 UPSERT 保 ID、bulk 只删员工保客户，以及双向局部状态与旧员工 bearer 不复活。[关闭后只读核验](../../../local-identity-sso-phase06-qa/development-post-checks.json)确认地区 **665923** 行且已 imported、receipt **1** 行、双身份对 **1**、旧二元键不存在、三元唯一键保留、剩余服务连接 **0**。 |

上述真实环境验收仅覆盖本任务新建的 `development`／`TASK_OWNED` 目标，三个创建开关只在该受控进程中显式开启，验收后进程已退出；仓库默认值和生产环境并未据此启用。本轮没有修改两个业务 Rust 仓库、接管独立预览／UUID 实例或落实未知生产 owner。manifest 所引用的写入审查是当时归档的原始快照；本段追加主流程最终结果，不改变该快照、manifest、receipt 或原始失败证据的字节。后续实际环境仍须逐一满足以下条件。

## 按环境启用前的必须条件

1. 记录明确身份目标环境及实例／writer owner，确认不指向独立预览或 UUID 专项目标。
2. 所有活动平台、CLI、seed、批任务使用本清单的安全代码；若该环境确有旧 binary，则真实清退并验证；本新库无旧 binary 的不适用范围保持明确。
3. 通过实时 catalog 外键／依赖检查，再执行独立收缩增量迁移；不得通过启动 auto migrate 提前删除旧约束。
4. Phase 1～6 后端安全、固定契约、通知和收缩验证全部通过，创建／绑定校验真实组织、租户／管理范围、精确账号与绑定来源。
5. 管理员显式按环境开启相应创建开关，记录生效版本；未满足条件维持 false。开启后只能退回已支持三元唯一和具体成员撤销的安全版本，不恢复二元唯一假设。

本文件不执行启用，不声称生产发布或实际环境的运维 owner 已落实。
