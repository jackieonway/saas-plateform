# Project Task Ledger — Dante Cloud SaaS 与微前端改造

> 台账版本：v1.5｜建立日期：2026-09-30｜状态来源：本文件
> 需求基线：[SaaS 与微前端改造需求分析](./saas-microfrontend-requirements.md)
> 初始代码基线：`main` 的 `edb9e34`（首次提交）；执行时应先读取实际工作区状态。
> 本台账从 Phase 1 开始记录实施与验收；完成状态以本文件中的证据为准。

## 1. Project Overview

### Project Name

Dante Cloud 多租户 SaaS 与独立微前端改造。

### Project Goal

保留现有 Rust/Axum 平台后端、PostgreSQL 和 React 宿主，形成可承载真实业务的 SaaS 控制层。平台统一管理身份、租户、组织、应用注册、授权上下文、审计和通知；可信微应用可独立部署，业务后端凭应用专用令牌校验当前用户的租户、组织、权限和数据范围。

### Background

仓库已有租户与组织树、成员、角色、分层菜单授权、OAuth、通知、HTML/ESM 微应用加载及 Vue/jQuery 示例。现存关键差距：会话撤销和组织状态校验不完整；应用令牌签发未重新检查应用授权；对外菜单/角色接口不是当前用户有效权限视图；微应用 props 缺完整上下文且组织切换可能保留旧令牌；独立发布与生产运行尚无闭环。详见需求基线第 2 节。

### Success Criteria

1. 两租户、多组织的交叉访问测试覆盖列表、详情、写入、导出及业务数据，普通用户不能跨租户/越权访问。
2. 切换组织、撤销成员或授权后，宿主、微应用和业务 API 在约定时限内同步使用新上下文，旧 token 失效。
3. 独立微应用能在不重建宿主的情况下发布/回滚；缺失资源返回 404，授权记录不受部署版本影响。
4. 至少一个真实业务试点使用统一接入契约和服务端权限校验；持久通知、审计、监控、备份和回滚通过端到端验收。

---

## 2. Global Constraints

### Technical Stack

- 平台后端：Rust 2021、Axum、SQLx；数据库：PostgreSQL；宿主：React、TypeScript、Vite；现有网关：Caddy。通知示例使用 Node.js、Socket.IO 和 PostgreSQL LISTEN/NOTIFY。
- 现有核心入口：`backend/src/main.rs`、`backend/src/routes/`、`backend/src/state.rs`、`backend/migrations/`、`frontend/src/App.tsx`、`frontend/src/components/micro/`、`mini-services/`。
- 平台 API 与业务应用 API 可以独立部署；业务应用必须遵循统一身份和授权上下文协议。用户已确认本机隔离 Linux 原生准生产为初版交付验收目标，实际现网部署另行确认，不预设 Kubernetes 或云厂商。

### Architecture Constraints

- 平台是身份、租户、组织、应用注册和授权的权威来源；业务数据由对应应用拥有。平台全局用户可有多个租户/组织成员身份。
- `Application` 定义、租户/组织/角色授权、部署版本和租户个性化配置应分离。更新微应用版本不得重写授权链。
- 可信 HTML 微应用可以在宿主页面执行；当前 Proxy 沙箱仅用于减少误伤和资源回收。未知或不可信代码必须走独立源 iframe/外链。
- 应用令牌与平台 session 分离。微应用前端得到的菜单/权限只用于展示；业务 API 必须自行验证令牌、资源归属和数据范围。
- 已执行的 SQLx 迁移不能原地修改；所有结构和数据修复通过新迁移实现，并提供回滚/兼容说明。

### Business Constraints

- 已确认仅直接成员可进入组织，祖先角色可继承但不扩大数据范围；停用或到期在下一请求拒绝。正式保留期等尚未确定的规则列于第 7 节，不得默认为产品事实。
- 已确认数据范围为 `SELF | ORG | ORG_SUBTREE | TENANT`，默认拒绝；角色只决定功能权限，数据范围按当前直接成员、应用、资源和操作独立显式授权。SELF 归属字段由具体业务声明。
- 当前“快速接入”只生成路径、静态入口和首页菜单，不自动授予租户/组织/角色权限，也不自动建立 OAuth 客户端。
- Athena 模拟数据、OAuth playground、Vue/jQuery 示例不是正式业务系统；试点阶段须使用真实租户归属的业务数据。

### Security / Compliance Constraints

- 普通请求的租户和组织上下文由服务端令牌及成员关系确定，客户端传来的 ID 不得扩大作用域。
- 不向独立应用分享平台 `JWT_SECRET`；应用后端使用受保护内省或公钥验证，浏览器不得持有客户端密钥。
- 生产禁用默认凭据/演示密钥，限制 CORS/CSP，所有敏感接口走 HTTPS，日志和审计不记录明文 token、密码或密钥。
- 受保护 API、导出、通知投递和异步任务均需租户边界；安全性以负向测试证明，不能只依据前端隐藏菜单。

### Code Style / Engineering Rules

- 修改前读取相应路由、迁移和现有契约；以代码为当前行为依据，以[需求文档](./saas-microfrontend-requirements.md)为目标依据。若两者冲突，记录并在台账中说明取舍。
- 每阶段仅完成该阶段范围；添加必要的迁移、接口/契约测试、跨租户负向测试和文档。测试必须验证业务不变量，不只镜像实现细节。
- 保留兼容期内的现有用户、授权和数据；涉及存量记录的迁移先检测异常，再执行可审计修复。
- 完成阶段后同步更新本台账的状态、验收证据、当前执行指针和变更日志；任何阶段不因代码已写而自动视为完成。

### Forbidden Actions

- 不在台账创建任务中开始 Phase 1 实施；后续执行也不得跳过未满足的依赖。
- 不修改已经部署过的历史迁移文件，不把默认管理员密码、JWT 密钥或客户端密钥写入生产配置。
- 不把 HTML 沙箱描述为安全隔离，不把 `/api/external/menus` 当前全量结果当成用户有效权限。
- 不在未确认组织继承/数据范围规则时自行扩张用户权限；不自动给新应用授予租户、组织或角色权限。
- 不把演示页面、模拟数据或单机 Compose 启动成功作为生产验收完成的证据。

---

## 3. Phase Status Summary

| Phase | Name | Status | Depends On | Output Summary |
| --- | --- | --- | --- | --- |
| 1 | 冻结授权规则与隔离基线 | COMPLETED | - | 规则矩阵、存量风险清单、两租户测试基线 |
| 2 | 加固数据库租户/组织关系 | COMPLETED | 1 | 0013 迁移、隔离 PostgreSQL 新库和合法存量库验证通过；六类非法关系被拒绝 |
| 3 | 收敛平台管理 API 的租户作用域 | COMPLETED | 1, 2 | 两租户管理 CRUD/导出范围修复、真实 PostgreSQL 进程内 API 回归通过 |
| 4 | 实现会话有效性与撤销 | COMPLETED | 1, 2 | 0014～0015 会话迁移、立即失效规则、双设备和撤销回归通过 |
| 5 | 加固应用专用令牌签发 | COMPLETED | 3, 4 | 0016～0019 迁移、5 分钟应用受众令牌、逐请求内省与撤销、双租户双应用回归通过 |
| 6 | 提供统一有效授权上下文 | COMPLETED | 1, 3, 5 | 0020 成员数据授权、统一菜单/权限计算、受众限定上下文及双实例回归通过 |
| 7 | 改造对外 API 与业务后端校验组件 | COMPLETED | 5, 6 | 旧路径收敛为当前用户视图、服务端校验组件和资源归属负向测试通过 |
| 8 | 让宿主与示例微应用消费完整上下文 | COMPLETED | 5, 6 | MicroProps 2.0、续期/取消/串行生命周期，31 项 Node 和 11 个真实浏览器场景通过 |
| 9 | 建立独立发布和资源接入链路 | COMPLETED | 8 | 0021 不可变版本、实际预检/回滚与默认 iframe；8 项真实 PG/HTTP、21 个浏览器及两网关 HTTP 验收通过 |
| 10 | 实现持久通知的应用接入 | COMPLETED | 5, 7 | 0022 归属/幂等、受权发布与收件箱、实时服务/宿主/两网关；7 项真实 PG 与 9 项真实平台联合验收通过 |
| 11 | 完成真实业务应用试点 | COMPLETED | 7, 8, 9, 10 | 独立 Vue/Node/业务库与完整实际平台联合验收 16/16 通过，含四范围、撤销、通知、CSV、独立发布/回滚及宿主哈希保持 |
| 12 | 完成生产运行与交付验收 | NOT_STARTED | 3–11 | CI/CD、监控、备份恢复、安全和发布验收记录 |

Allowed status values: `NOT_STARTED`, `IN_PROGRESS`, `BLOCKED`, `COMPLETED`.

---

## 4. Phase Details

### Phase 1: 冻结授权规则与隔离基线

#### Status
COMPLETED

#### Goal
为后续实现提供可执行的租户、组织、角色继承和数据范围规则，并建立能暴露当前越权风险的基线。

#### Scope
阅读现有关系与路由；整理平台管理员、租户管理员、组织管理员的边界；设计两租户、多层组织、跨租户成员的测试夹具；扫描存量数据库约束与异常关系。对应 TEN-01～04、AUTH-01～02、NFR-02。

#### Out of Scope
不修改授权算法、生产数据或现有迁移；不开始会话、令牌和微应用代码改造。

#### Inputs
`docs/saas-microfrontend-requirements.md` 第 4、8、11 节；`backend/migrations/0001_init.sql`；`backend/src/state.rs`；`backend/src/routes/{auth,tenants,organizations,departments,users,roles}.rs`。

#### Execution Requirements
写出角色继承、成员身份、组织树访问和 `SELF/ORG/ORG_SUBTREE/TENANT` 的行为矩阵；对第 7 节 Q1/Q2 等未决项提供具体备选和影响；建立自动化负向测试骨架。若需产品确认，准备可审阅的决策结果，不擅自扩大权限。

#### Expected Outputs
`docs/authorization-policy.md`（或等价规则文件）、存量数据审计报告、跨租户测试矩阵及可运行夹具；本台账决策日志与开放问题更新。

#### Acceptance Criteria
- 规则矩阵覆盖直接成员、祖先角色、子组织、平台管理员、组织停用和租户到期。
- 两租户至少各含两层组织、共享用户与不同授权，测试能展示预期允许/拒绝结果。
- 数据库中潜在跨租户父子组织、部门和角色关系有检测 SQL 与处理建议。
- 需要业务确认的规则均有明确问题、推荐值和影响范围；Phase 2 所需约束已确认或被标为阻塞。

#### Risks
现有种子数据只覆盖演示场景；真实数据可能存在跨组织关系或不同继承预期。

#### Dependencies
无。

#### Notes
已交付 `docs/authorization-policy.md`、`docs/tenant-isolation-baseline.md`、`docs/sql/tenant-relationship-audit.sql`、`tests/tenant-isolation-policy-fixture.sql` 和 `tests/tenant-isolation-api.mjs`。PGlite 中 7/7 个目标矩阵断言通过，其中 2 项显示现有权限 SQL 的风险；空库关系审计可执行。实际数据库/API 未运行，作为 Phase 2/3 门禁保留。Q1/Q2 的推荐规则尚未获业务批准，不得据此扩大权限；Phase 2 的同租户/同组织结构约束不依赖这两项授权语义，可先继续。

---

### Phase 2: 加固数据库租户/组织关系

#### Status
COMPLETED

#### Goal
让组织、部门、成员和角色关系在数据库层不能跨错误租户或组织关联。

#### Scope
检测并处理存量异常；新增 SQLx 迁移，约束组织父节点与租户一致、部门父节点与组织一致、成员部门与成员组织一致、角色的租户/组织一致；补迁移验证。对应 TEN-03、NFR-02。

#### Out of Scope
不引入所有租户独立数据库，不修改历史 `0001`～`0012` 迁移，不重写业务 API。

#### Inputs
Phase 1 规则矩阵与数据审计；`backend/migrations/`；`backend/src/routes/{organizations,departments,users,roles}.rs`。

#### Execution Requirements
采用新迁移；迁移前先输出异常记录，明确是阻止迁移还是按已批准规则修复。为正常和非法关联分别建立数据库级验证。

#### Expected Outputs
新迁移文件、存量修复/回滚说明、数据库约束测试与数据库设计更新。

#### Acceptance Criteria
- 新库迁移成功，已有库经审计和修复后迁移成功。
- SQL 直接插入跨租户组织父节点、跨组织部门父节点及不匹配角色均失败。
- 合法多层组织与跨租户身份成员关系仍可使用。
- 历史迁移校验和未变化，数据修复有记录。

#### Risks
新增复合约束可能与现有种子或存量数据冲突。

#### Dependencies
Phase 1 的关系规则和异常处理决策。

#### Notes
已交付 `backend/migrations/0013_tenant_relationship_integrity.sql`、`tests/tenant-relationship-constraints.sql`、`docs/tenant-relationship-migration.md` 及 `docs/db-schema.md` 更新。PGlite 中 `0001`～`0013` 全链迁移通过；六类非法关系被拒绝，模拟合法存量可迁移，异常存量会被预检安全拒绝。隔离 PostgreSQL 17.6 中后端自动建库并通过 13 条 SQLx 迁移，种子数据只读审计为 0 条异常，六类约束测试通过；另一个临时库验证了合法两租户存量数据迁移与跨租户身份成员关系。历史迁移文件未改，生产存量仍须独立审计。本地配置位置为 Git 忽略的 `.env.test`，模板为 `.env.test.example`。业务应用的数据表隔离在 Phase 7/11 中验证；本阶段处理平台核心关系。

---

### Phase 3: 收敛平台管理 API 的租户作用域

#### Status
COMPLETED

#### Goal
让普通租户在所有平台管理读写入口只能操作其允许的租户、组织和用户资源。

#### Scope
审查并修复 `tenants`、`organizations`、`departments`、`users`、`roles`，以及相关授权、审计、监控与导出查询中的作用域；统一资源归属校验与不可见/无权错误语义。对应 TEN-01～03、AUTH-02、AUD-01。

#### Out of Scope
不改造应用令牌签发、对外 OAuth 资源 API 和微应用宿主。

#### Inputs
Phase 1 权限矩阵、Phase 2 数据约束；`backend/src/routes/`；`backend/src/extractors.rs`；`backend/src/state.rs`。

#### Execution Requirements
逐类覆盖列表、详情、创建、修改、删除、状态变更、批量操作和 CSV 导出；服务端从上下文确定租户边界。对平台级操作保留显式权限与审计，不通过客户端过滤器扩大范围。

#### Expected Outputs
平台管理 API 修复、统一范围校验辅助层、跨租户/跨组织集成测试、接口契约更新。

#### Acceptance Criteria
- 两租户负向矩阵覆盖上述操作类型，非平台用户无法通过修改 ID 或过滤参数越界。
- 平台管理员跨租户操作仍可用，且审计记录目标租户、主体和操作。
- 未授权资源的错误口径一致，不泄露其他租户的详情或存在性信息。
- 原有同租户合法流程通过回归测试。

#### Risks
当前路由中作用域逻辑分散；修复一个写路径可能影响列表或审计口径。

#### Dependencies
Phase 1、Phase 2。

#### Notes
已加固 `tenants`、`organizations`、`departments`、`users`、`roles`、`audit`、`monitor`、`search` 以及全局应用/菜单管理接口。统一的 `AppState::ensure_tenant_scope` 使跨租户资源 ID 与不存在 ID 同为 404；普通租户的用户成员、角色、角色数和搜索组织名按租户过滤，替换成员/角色只修改本租户数据；共享全局账号仅平台可修改，且全局写入在锁定用户行后校验归属。全局监控及应用/菜单目录限平台上下文，租户洞察在 SQL 中限制租户并按令牌实际租户计数。平台跨租户操作按目标租户审计。

`backend/src/scope_tests.rs` 在隔离 PostgreSQL 17.6 上通过双向两租户进程内 Axum API 矩阵，覆盖列表、详情、创建、修改、删除、状态变更、批量、审计和监控 CSV 导出、平台跨租户及同租户合法路径；测试夹具已清理为 0 个剩余租户/用户。Windows 本机端口访问探针仍超时，已用真实数据库加进程内路由请求完成本阶段 API 语义验收；网络监听问题留给 Phase 12 运行环境排查。通知发送的租户化留在 Phase 10，应用令牌/OAuth 管理留在 Phase 5/7。

---

### Phase 4: 实现会话有效性与撤销

#### Status
COMPLETED

#### Goal
使平台 session 在登出、踢出、密码重置、成员移除和组织/租户失效后按定义及时失效。

#### Scope
会话记录或用户/上下文版本机制；`CurrentUser` 的成员与状态复核；登录、切换组织、登出、密码重置、会话管理和审计。对应 TEN-01、IAM-01、IAM-03、IAM-06。

#### Out of Scope
不实现应用专用 token、业务后端适配器或微应用 UI 变更。

#### Inputs
Phase 1 状态失效规则、Phase 2 关系约束；`backend/src/routes/auth.rs`、`backend/src/extractors.rs`、`backend/src/util/jwt.rs`、`backend/migrations/`。

#### Execution Requirements
定义每请求校验与缓存策略、撤销时限和并发切换语义；迁移存量会话时避免误把旧令牌当成永久有效。记录必要的审计而不记录明文令牌。

#### Expected Outputs
会话持久化/版本迁移、后端会话校验与吊销实现、前端登出/切换适配、失效测试和运维说明。

#### Acceptance Criteria
- 登出和管理员踢出后的旧 session 被拒绝；密码重置后的失效规则通过测试。
- 成员移除、组织/租户停用或到期后旧 session 在已批准时限内失效。
- 切换组织只能选择有效成员组织；旧组织并发响应不能覆盖新会话。
- 合法多设备会话与会话列表行为有明确测试。

#### Risks
JWT 无状态与即时撤销之间需要持久状态或版本查询，可能影响请求延迟。

#### Dependencies
Phase 1、Phase 2。

#### Notes
用户已确认登出、管理员踢出、密码重置立即失效，成员移除和租户/组织失效在下一次请求拒绝，多设备独立管理。新增 `0014_platform_sessions.sql`、`0015_platform_session_invalidation.sql`、`backend/src/session.rs` 和 `docs/platform-session-lifecycle.md`；`CurrentUser` 每请求查库复核持久会话、用户/租户/组织祖先状态及直接成员。旧 JWT 缺 `sid/ver` 会被拒绝并要求重新登录。组织切换原子撤销旧会话，前端串行切换并清理旧缓存；改密成功后退出，设备会话可列出并由本人撤销。

隔离 PostgreSQL 17.6 上 `platform_session_lifecycle` 与 Phase 3 双租户回归共 2 项通过；覆盖双设备、登出、本人撤销、组织切换、旧票据、改密/重置、管理员踢出、成员删除及重加、组织祖先和租户停用/到期及恢复、用户状态与旧版 JWT。空库自动建库后 `0001`～`0015` 共 15 条迁移及种子成功，临时库已删除；测试夹具剩余 0 个租户/用户。`cargo test -q` 和前端 Node 24 构建通过。应用 token 的撤销语义留在 Phase 5。

---

### Phase 5: 加固应用专用令牌签发

#### Status
COMPLETED

#### Goal
只为当前用户确实获授权的应用签发用途受限、可撤销的应用 access token。

#### Scope
`/api/oauth/session-token` 的租户/组织/应用授权检查、应用受众、唯一令牌标识、TTL、内省与撤销；关联 OAuth 客户端的状态与配置校验。对应 IAM-01～03、IAM-05。

#### Out of Scope
不在浏览器中放客户端密钥，不实现业务 API 的所有操作权限，不改变 OAuth playground 的演示目标。

#### Inputs
Phase 3 API 作用域、Phase 4 session；`backend/src/routes/oauth2.rs`、`oauth_admin.rs`、`auth.rs`、`backend/src/util/jwt.rs`、`backend/migrations/`。

#### Execution Requirements
签发前复核成员、租户/组织/应用授权与状态；明确 `aud/appCode`、`sub`、租户、组织、`jti` 和过期声明。对现有令牌格式提供兼容/迁移方案；客户端密钥仅用于服务端。

#### Expected Outputs
令牌签发与验证改造、必要迁移、协议契约、正负向测试和配置说明。

#### Acceptance Criteria
- 未授权、已禁用、跨租户或无成员关系的应用请求不能得到 token。
- A 应用 token 在 B 应用校验处被拒绝；过期和撤销 token 内省为无效。
- 令牌响应和日志不泄漏客户端密钥；已有合法应用流程有兼容测试。
- 每个 token 的用户、租户、组织、应用受众可追踪且与签发时授权一致。

#### Risks
现有 `oauth_clients` 可能由多个用途共享；受众和撤销设计需兼顾旧客户端。

#### Dependencies
Phase 3、Phase 4。

#### Notes
Q4 已确认正式采用服务端逐请求内省；如未来增加公钥验证机制，必须保留即时撤销语义。

用户确认每应用独立受众、应用令牌最多 5 分钟，业务后端以机密客户端凭据逐请求内省，撤销立即生效。`0016`～`0019` 迁移建立令牌绑定、撤销与旧令牌升级规则；应用前端只接收短期令牌，不接收客户端密钥。隔离 PostgreSQL 17.6 上双租户双应用正负向测试、撤销/过期/配置变化、原有 `client_credentials` 兼容均通过；3 项真实数据库测试串行通过，`cargo test -q` 和 `cargo check -q` 通过。详情见[应用令牌接入契约](./application-token-contract.md)。

---

### Phase 6: 提供统一有效授权上下文

#### Status
COMPLETED

#### Goal
用一个服务端计算结果向宿主和业务后端提供当前应用的菜单、角色、权限码和数据范围。

#### Scope
统一计算器、上下文 API、契约版本/上下文版本、权限变更失效策略；消除菜单显示与 API 权限计算的口径分叉。对应 TEN-04、AUTH-01、AUTH-04、CTX-01～02。

#### Out of Scope
不把完整管理配置暴露给普通应用令牌，不在本阶段改造全部外部接口或独立应用前端。

#### Inputs
Phase 1 规则矩阵、Phase 3 管理作用域、Phase 5 应用 token；`backend/src/state.rs`、`backend/src/routes/auth.rs`、`backend/src/routes/external.rs`、需求文档第 5.5 节。

#### Execution Requirements
分别计算菜单/按钮/角色继承与独立数据授权，业务 API 同时校验两者；API 只接受目标应用受众 token。多实例缓存通过版本或事件失效；契约有版本与失效时间。

#### Expected Outputs
授权计算服务、应用上下文接口、接口文档、缓存失效实现、跨租户和组织继承测试。

#### Acceptance Criteria
- 同一 `(user, tenant, org, app)` 在宿主、上下文接口和后端计算得到相同有效菜单/权限。
- 错误受众、失效授权、其他租户或组织的 token 返回拒绝结果，无额外数据。
- 角色或菜单授权变更在已批准时限内跨实例反映。
- 上下文包含版本、过期时间和经过确认的数据范围语义。

#### Risks
现有 30 秒进程内缓存与跨实例失效、角色继承规则可能互相影响。

#### Dependencies
Phase 1、Phase 3、Phase 5。

#### Notes
建议以需求文档的上下文 JSON 为草案，正式字段在实施时冻结并版本化。

2026-10-02：用户明确要求角色与数据权限分离，原“多角色数据范围合并”问题作废。当前没有独立数据授权模型；已请求确认策略绑定粒度。`SELF` 由业务资源声明可验证的用户归属字段，在 Phase 7/11 的接入组件和试点中落实，不能从客户端参数推断。现有权限缓存为 30 秒进程内缓存，跨实例失效将作为本阶段实现项。

用户确认首版按当前组织直接成员关系、应用、业务资源和操作显式授予四级数据范围。`0020` 建立独立授权表；`GET /api/apps/{appCode}/context` 返回版本化菜单/权限与 `dataGrants`，不从角色推导数据权限。宿主菜单、平台操作权限和上下文菜单共用查询；移除 30 秒进程内正向权限缓存，两个实例下一次请求均读取最新变更。隔离 PostgreSQL 17.6 上 3 项串行回归通过，覆盖双租户双应用、错误受众、撤销、祖先角色继承、数据范围不继承、范围撤销、上下文版本及双实例一致性。`SELF` 资源字段的实际映射留在 Phase 7/11。契约见[应用授权上下文](./app-authorization-context.md)。

---

### Phase 7: 改造对外 API 与业务后端校验组件

#### Status
COMPLETED

#### Goal
让独立业务后端能安全消费平台上下文，并使普通应用 token 只看到当前用户的授权视图。

#### Scope
区分管理配置 API 与当前用户 API；修复 `/api/external/menus`、`/roles` 的语义/鉴权或提供替代接口；提供 Rust 或语言无关的后端校验示例，覆盖令牌、受众、权限与资源归属。对应 AUTH-02～03、CTX-03～04、IAM-05。

#### Out of Scope
不建设完整业务产品，也不允许业务后端直接信任前端 props。

#### Inputs
Phase 5 token 契约、Phase 6 上下文 API；`backend/src/routes/external.rs`、`oauth2.rs`、`docs/api-contract.md`、`examples/`。

#### Execution Requirements
保持管理 API 的显式平台权限；新增或调整用户视图需按 token 中的用户、租户、组织和受众过滤。适配器的密钥与缓存策略只在服务端运行；示例查询必须带租户/数据范围条件。

#### Expected Outputs
对外 API 改造及迁移说明、业务后端接入组件/参考实现、接口契约、跨租户与资源 ID 篡改测试。

#### Acceptance Criteria
- 普通应用 token 无法取得其他组织角色、全量应用菜单或另一应用数据。
- 参考业务 API 拒绝过期、撤销、错误受众、缺权限和跨租户资源 ID。
- 有效 token 的同租户、同范围业务请求成功；客户端密钥不出现在浏览器资源中。
- 旧对外接口的保留、废弃或更名路径明确记录。

#### Risks
现有 Vue/jQuery 示例依赖全量 `/api/external/menus` 和 `/roles`，需要同步迁移或明确保留管理端点。

#### Dependencies
Phase 5、Phase 6。

#### Notes
本阶段提供可复用后端路径；真实业务数据试点在 Phase 11。

`/api/external/menus`、`/roles` 保留路径但只接受有效应用专用令牌，分别返回当前用户菜单和当前组织角色；旧 `client_credentials` 不再读取这两个用户视图，仍可用于原 OAuth 服务流程。`examples/app-backend/authorize.mjs` 逐请求内省并获取上下文，校验受众、操作权限、独立数据授权以及资源的租户/组织/用户归属，查询示例在 SQL 中同时绑定资源 ID 和租户 ID。隔离 PostgreSQL 17.6 上 3 项串行回归通过，覆盖未授权菜单、另一组织角色、另一应用受众和撤销令牌；Node 参考组件 5 项测试通过，覆盖同范围合法请求及过期/撤销/错误受众/缺权限/跨租户资源 ID。迁移说明见[对外 API 用户视图迁移](./external-api-migration.md)。

---

### Phase 8: 让宿主与示例微应用消费完整上下文

#### Status
COMPLETED

#### Goal
让宿主为微应用安全提供当前组织的完整展示上下文和新 token，避免上下文切换时旧数据回流。

#### Scope
版本化 `MicroProps`、租户/组织/角色/权限/菜单传参、token 续期、组织切换、异步请求取消、`mount/update/unmount` 契约测试；Vue/jQuery 示例验证。对应 APP-03～04、IAM-04、NFR-09。

#### Out of Scope
不建立独立发布注册，不把示例改为真实业务系统，不在沙箱内执行不可信代码。

#### Inputs
Phase 5/6 的 token 与上下文；`frontend/src/components/micro/`、`frontend/src/hooks/use-auth-data.ts`、`frontend/src/stores/auth.ts`、`mini-services/micro-{vue,jquery}/`。

#### Execution Requirements
应用/组织变更时先停止旧工作，再获取新上下文；使用单调版本或请求序号拒绝迟到响应。`update` 的触发条件覆盖 token、租户、组织、权限、菜单、主题和路由；无 `update` 的子应用安全重挂。

#### Expected Outputs
宿主上下文管线、版本化接入类型/示例 SDK、Vue/jQuery 契约更新、切换与卸载自动化测试、接入文档更新。

#### Acceptance Criteria
- 连续快速切换组织/应用时，不展示旧组织数据，也不使用旧 token 发新请求。
- token 续期后示例微应用收到新值；权限/菜单/主题/路由变化触发正确更新。
- 反复挂载/卸载后无持续计时器、document/window 监听器或样式泄漏。
- 当前授权应用不可用时宿主退出其路由并显示明确状态。

#### Risks
当前挂载 effect 与更新签名未覆盖完整上下文，异步顺序和 React StrictMode 可能产生竞态。

#### Dependencies
Phase 5、Phase 6。

#### Notes
当前 Proxy 沙箱只能减少资源污染；接入信任边界由 Q5 和 Phase 9 控制。

宿主取得有效应用 token 和完整上下文后才挂载；组织切换先撤下旧实例，查询固定 token/epoch/signal，迟到结果不回流。MicroProps 2.0 覆盖租户、组织、角色、操作权限、独立 dataGrants、菜单、版本与到期时间。上下文至多约 20 秒刷新，token 到期前续签；完整内容变化推送 update，未实现 update 时串行卸载后重挂。HTML loader 失败/取消缓存同步换代并在每段脚本前检查取消；沙箱回收计时器、RAF、window/document 同步监听和动态节点。Vue/jQuery 发布镜像与 ESM 示例同步升级。31 项 Node 回归、11 个真实 Chromium 场景、TypeScript 和生产构建通过。详见[Phase 8 验收记录](./phase8-validation.md)和[MicroProps 2.0 契约](./micro-app-context-props.md)。

---

### Phase 9: 建立独立发布和资源接入链路

#### Status
COMPLETED

#### Goal
让可信微应用独立部署和回滚，并由平台稳定配置其入口与版本。

#### Scope
应用版本/环境入口注册、健康/资源预检、同源网关路径、静态文件 404 与 MIME、缓存策略、发布切换/回滚、可信/iframe 模式标记。对应 APP-01～02、APP-05～07、NFR-04、NFR-06。

#### Out of Scope
不替所有应用选定同一家云厂商；不在本阶段实现完整自动审批或复杂灰度。

#### Inputs
Phase 8 契约；`frontend/src/components/micro/loader.ts`、`backend/src/routes/applications.rs`、`deploy/Caddyfile`、`docker-compose.yml`、`docs/microfrontend-integration.md`。

#### Execution Requirements
把应用定义和部署版本分离；为独立服务提供同源前缀代理，入口和脚本/样式路径按应用版本解析。缺失资源必须 404；HTML 入口与带哈希资源采用不同缓存策略。明确不可信 iframe 的消息来源和令牌传递边界。

#### Expected Outputs
注册/版本迁移及管理界面、网关示例配置、发布预检、回滚流程、独立部署接入手册与发布测试。

#### Acceptance Criteria
- 独立 Vue 微应用发布新资源后无需重新构建宿主，授权关系保持不变。
- 无效入口、错误 MIME、缺失 JS/CSS、健康失败阻止启用或给出可诊断失败；缺失文件返回 404。
- 可切回上一可用版本，已登录用户的应用入口与资源保持一致。
- 未列为可信的外部应用不能在宿主页面上下文执行代码。

#### Risks
现有种子入口含 `/sub/vue/`、`/sub/jquery/` 和 Vite 源码路径；迁移需要兼容存量应用设置。

#### Dependencies
Phase 8。

#### Notes
可保留 `scripts/sync-microapp.mjs` 作为随主站发布的简单模式；独立发布不应依赖它。
已交付 0021、发布/预检/激活/回滚 API 与管理界面、默认 opaque iframe 和经典 SDK、不可变打包工具、可配置只读挂载及 Caddy/Nginx 版本网关。8/8 真实 PostgreSQL/HTTP gate、4/4 Rust 单元、21/21 宿主及管理页面浏览器、16/16 iframe、31/31 上下文/示例、29/29 Linux 打包、Caddy/Nginx 各 17/17 实际网关测试通过，TypeScript、生产构建和 Compose 配置通过。验收与边界见 `docs/phase9-validation.md`；实际 Docker/生产部署保留给 Phase 12。

---

### Phase 10: 实现持久通知的应用接入

#### Status
COMPLETED

#### Goal
让业务应用在授权范围内发布可追踪站内通知，同时保留 `onNotify` 作为纯界面 Toast。

#### Scope
应用身份与租户范围验证、受权发布 API、用户/组织目标校验、去重和重试；通知 Socket.IO 服务的配置、部署、健康、重连与审计。对应 MSG-01～03、AUD-01。

#### Out of Scope
不把任意 Toast 写入数据库，不建设短信、邮件等外部渠道。

#### Inputs
Phase 5 应用 token、Phase 7 后端校验组件；`backend/src/routes/notifications.rs`、`mini-services/notification-ws/`、`frontend/src/hooks/use-notification-socket.tsx`、`docker-compose.yml`。

#### Execution Requirements
发布端只接受明确授权的应用或用户上下文；目标用户必须在允许租户/组织范围内。连接中断时数据库记录是权威来源；配置不再使用开发机绝对路径或写死数据库地址。

#### Expected Outputs
通知发布契约、后端接口、服务配置与编排、去重/跨租户/断线恢复测试、运维说明。

#### Acceptance Criteria
- `onNotify` 只弹 Toast；持久发布接口写库、可标记已读、可审计。
- 另一租户目标被拒绝；同一去重键的重试结果符合已批准策略。
- 通知服务在生产配置下可健康启动，断线重连后未读状态与数据库一致。
- 客户端无法使用普通显示回调绕过服务端发布权限。

#### Risks
现有服务固定端口、数据库 URL 和备用密钥文件路径；Socket.IO 路径与网关配置耦合。

#### Dependencies
Phase 5、Phase 7。

#### Notes
本阶段的投递成功与即时在线推送分开衡量；持久记录先保证正确性。
用户已确认：发布同时要求 `notification:publish` 操作权限与 `notification/publish` 独立数据授权；目标必须是当前租户的有效成员且可访问该应用，组织目标默认只取直接成员。去重按租户、组织、应用、发布用户及键隔离；同键同内容返回原结果，同键不同内容返回 409。

迁移实现采用保留并隔离旧无归属通知的策略，不推断其租户后混入新租户收件箱。
已交付 0022、统一事务发布与五维幂等、严格收件归属和动态 APP 授权、SYSTEM 同业务事务通知、内部 session 校验、Node 24 实时服务、宿主恢复与统一 Socket.IO 网关。7/7 真实通知 PG、8/8 发布回归、9/9 最新 main/PG/SocketIO 联合、71 服务协议、5 LISTEN、9 前端 scope、10 浏览器、9 Vite 和两网关各 23 项通过，编译/TypeScript/生产构建/Compose 配置通过。最终契约、运维和证据见 `docs/notification-integration.md`、`docs/notification-operations.md`、`docs/phase10-validation.md`。Docker 镜像与生产 TLS 演练留 Phase 12。

---

### Phase 11: 完成真实业务应用试点

#### Status
COMPLETED

#### Goal
以一个真实业务用例验证平台控制层、独立微应用和业务 API 的完整授权链。

#### Scope
选择一个有真实 CRUD/列表/导出需求的试点（业务领域由用户确认）；独立前端与业务后端、含 `tenant_id`/组织范围的数据模型、平台上下文、通知及独立发布；执行需求文档第 10 节端到端场景。对应 CTX-03、AUTH-02、APP-02、MSG-02。

#### Out of Scope
不把 Athena 模拟数据包装成真实业务；不一次迁移所有旧系统。

#### Inputs
Phase 7/8/9/10 的契约与运行能力；需求文档第 10 节；试点业务领域和样本数据由业务方提供或批准。

#### Execution Requirements
试点数据与服务端查询带强制租户/组织过滤；正常、跨租户、跨组织、权限撤销、组织切换、令牌失效、版本回滚和通知均做端到端验证。若无现成真实项目，可用经用户确认的最小业务模块，但不得把模拟静态页面算作完成。

#### Expected Outputs
可独立部署的试点应用、数据库迁移、业务后端接入、部署配置、端到端测试与验收记录。

#### Acceptance Criteria
- 同一试点由两租户使用，双方数据互不可见；组织数据范围与 Phase 1 规则一致。
- 隐藏按钮对应 API 直接调用、篡改资源 ID、旧组织 token 均被服务端拒绝。
- 应用独立发布/回滚不要求重新构建宿主；通知在授权范围内入库并展示。
- 业务方确认试点操作流程、字段和数据保留口径。

#### Risks
工单试点领域、流程和数据口径已获用户确认；实际现网业务数据导入及正式保留期在对应部署时另行评审。

#### Dependencies
Phase 7、Phase 8、Phase 9、Phase 10；Q8 的试点选择。

#### Notes
若 Q8 长期未答复，应完成可独立验证的准备工作；达到明确阻塞条件后按执行规则更新状态，不用静态演示替代真实试点。
Q8 已解决：用户确认 `docs/business-pilot-proposal.md` 的工单试点（work-orders、Vue iframe、Node 24 独立业务 API、真实 CRUD/CSV、三种状态与重开、SELF 按创建人、软删除保留且不自动清理、手动发送提醒）。按该提案实施，不用原示例模拟数据替代业务数据。

**用户暂停检查点（2026-10-02）：** 用户要求停止任务，等待通知再继续。Phase 11 保持 `IN_PROGRESS`，不得标记完成；本阶段改动尚未提交。业务 API 48/48 真实 HTTP/PostgreSQL、平台成员接口 2 组真实 PostgreSQL、iframe 协议 21/21、前端 22/22、bootstrap 31/31、两网关各 30 项以及 TypeScript/生产构建已通过；这些门禁的实际服务与 fixture 覆盖范围须分别记录。首次完整联合运行有 14 项真实 API 场景通过，浏览器场景发现宿主循环分包和原生 fetch receiver 两项缺陷，均已修复；后续真实浏览器调试确认宿主和 Vue 可以挂载，最终正常全链路验收仍未完成。恢复后先用已修复的宿主与工单构建完整验证浏览器 CRUD、CSV 下载、通知、组织切换／撤销及 v1→v2→回滚期间宿主文件哈希不变，再完成 Phase 11 验收文档、台账和提交。Phase 12 的原生准生产方案已获用户确认，尚未实施；须待 Phase 11 完成后推进。不得提交 `.env.test`，不得修改无关 `outputs/`。

**恢复完成记录（2026-10-07）：** 最终正常全量联合运行 16/16 PASS（包含总测试及四个嵌套范围测试），0 failure／cancelled／skip，总耗时 596.846 秒。实际 PostgreSQL 17.6 平台库完成 22 条迁移，独立业务库迁移校验和匹配；实际 Linux Node 24.3、Rust main、Nginx 1.28、通知、生产宿主、Vue 与 Chromium 151 完成四级范围、跨租户、写入／导出／提醒、授权撤销、组织切换和不可变版本发布回滚。宿主 100 个文件逐项哈希、页面实例、会话和宿主脚本请求计数保持不变；敏感日志检查通过。已修复生产分包循环、原生 fetch receiver、拒绝后的界面缓存及 iframe／弹窗视口布局；验收脚本也修正撤销后旧写入令牌与提醒／主题控件同步。独立清理审计确认自有数据库、数据库连接、服务／浏览器进程和临时目录均为 0。审查截图与非秘密运行记录保留于 Git 忽略目录，截图删除曾被自动审批拒绝，不计为活跃资源。完整证据和测试／fixture 边界见 `docs/phase11-validation.md`。

---

### Phase 12: 完成生产运行与交付验收

#### Status
NOT_STARTED

#### Goal
将已验证的控制层和试点应用交付为可重复部署、可观测、可恢复的运行系统。

#### Scope
CI/CD 与迁移顺序、生产密钥与默认凭据清理、HTTPS/CORS/CSP、健康与指标、请求 ID、审计查询、数据库备份/恢复、发布回滚演练及最终验收。对应 NFR-01～08、AUD-01、TEN-05。

#### Out of Scope
不承诺未经批准的 SLA、RPO/RTO；不顺带建设计费、支付、多区域部署或全部租户独立数据库。

#### Inputs
Phase 3～11 的交付物；`backend/Dockerfile`、`frontend/Dockerfile`、`docker-compose.yml`、`deploy/Caddyfile`、需求文档第 6、10 节。

#### Execution Requirements
使用用户确认的本机隔离 Linux 原生准生产环境执行实际构建、迁移、部署、HTTPS、故障和恢复演练；从 production 配置中移除演示密钥与账号；建立可定位租户/应用但不泄漏敏感信息的日志与指标。RPO 24 小时、RTO 1 小时为演练目标，不承诺商业 SLA；数据不自动清理。实际 Docker 构建／容器启动待 runner，不计为本次原生目标的完成证据。详见 `docs/delivery-acceptance-proposal.md`。

#### Expected Outputs
CI/CD 配置、生产部署与回滚手册、监控/告警配置、备份恢复记录、安全检查报告、最终端到端验收报告。

#### Acceptance Criteria
- 从干净环境可重复构建和部署；迁移失败有安全停止/恢复路径。
- 默认凭据、明文密钥、宽松跨域和未授权脚本来源已按批准方案处理。
- 平台 API、网关、微应用、通知与数据库具备健康检查和可定位指标；故障告警演练通过。
- 备份数据可恢复；版本回滚和跨租户/撤销/通知终验通过，结果记录在台账。

#### Risks
生产主机、域名、密钥管理、监控系统和正式 SLO 尚未确认。

#### Dependencies
Phase 3～11 通过验收；Q6/Q7 的运维决策。

#### Notes
这是交付门禁，不替代各阶段自己的测试。只有实际演练证据完整后才能标为 `COMPLETED`。
用户已明确采用本机准生产提案，作为初始台账“生产环境”的本次验收目标调整。原生部署、专用 CA 的真实 TLS 验证、故障告警及加密备份恢复必须实测；不直接发布现网。Docker／CI 配置仍交付，未执行的容器部署在报告中明确。

---

## 5. Current Execution Pointer

Current Phase: Phase 12

Current Objective:
完成已确认的本机隔离 Linux 原生准生产交付：生产安全配置、可重复构建部署、实际 HTTPS、可定位健康／指标及故障告警、加密备份和新库恢复、最终授权与发布回滚验收。RPO 24 小时／RTO 1 小时为演练目标，数据不自动清理，Docker 实跑待 runner。

用户于 2026-10-07 通知继续，Phase 11 最终完整验收已通过。按用户执行所有阶段的授权，在 Phase 11 文档和提交收尾后推进 Phase 12。

Phase 3～11 依赖均已通过。初版原生准生产目标已获用户确认；正式现网主机／域名、商业 SLA 和正式保留期在对应部署时另行确认，不阻塞已批准的初版工程验收。

Current Restrictions:
Q1 的直接成员/祖先角色规则、Q2 的四级数据范围与默认拒绝、角色和数据授权分离及按成员关系显式授权、Phase 4 的立即失效与多设备独立策略，以及 Phase 5 的每应用独立受众、5 分钟令牌和逐请求内省均已获用户确认。SELF 归属字段由业务资源接入时声明并在服务端验证，不得自行扩大。不得把测试用 `.env.test` 提交到 Git。

---

## 6. Decision Log

| Date | Decision | Reason | Impact |
| --- | --- | --- | --- |
| 2026-09-30 | 建立以 `docs/saas-microfrontend-requirements.md` 为需求基线的 12 阶段台账。 | 现有需求覆盖身份、隔离、微应用和生产运行，需按依赖拆成可验收单元。 | 后续执行从 Phase 1 开始，并在本文件记录进度。 |
| 2026-09-30 | 暂定保留 Rust/Axum + PostgreSQL 控制层及 React 宿主，业务应用允许独立部署。 | 与既有代码和需求范围一致；属于规划约束，非业务规则批准。 | 避免在早期阶段进行无目标的整体重写。 |
| 2026-09-30 | 组织继承、数据范围、信任边界和运维目标保持 OPEN。 | 需求文档明确列为待评审。 | Phase 1 先产出决策材料；相关后续阶段不得自行扩大权限。 |
| 2026-09-30 | Phase 1 完成静态基线；同租户/同组织结构一致性可先于 Q1/Q2 授权语义实施。 | 两租户夹具与只读审计 SQL 已可运行，结构关系不变量不扩大用户权限。 | 指针推进 Phase 2；真实库审计与 Q1/Q2 继续记录为后续门禁。 |
| 2026-09-30 | Phase 2 保留目标环境验收门禁，离线通过不等于阶段完成。 | 当前机器没有可连接的 PostgreSQL；用户将填写 `.env.test` 中的隔离连接。 | Phase 2 标记 `BLOCKED`，Phase 3 暂不启动。 |
| 2026-09-30 | `.env.test` 连接诊断发现目标测试库不存在。 | PostgreSQL 17.6 实例及维护库可连接，目标库不存在；当前账号具备建库权限，但实例隔离性尚未确认。 | 等待隔离环境确认或连接信息修正；不在未确认的实例上创建数据库。 |
| 2026-09-30 | 用户确认允许建库，并要求后端支持自动建库。 | 隔离实例授权明确；首次部署应能在目标库不存在时初始化。 | 加入默认关闭的 `AUTO_CREATE_DATABASE` 开关，仅在数据库不存在时建库，然后执行原有迁移和种子。 |
| 2026-09-30 | Phase 2 在隔离 PostgreSQL 17.6 达成验收。 | 自动建库及 13 条 SQLx 迁移成功，种子审计 0 异常，六类非法关系被拒绝，合法存量数据迁移通过。 | Phase 2 标记 `COMPLETED`，指针推进 Phase 3；生产存量需另行审计。 |
| 2026-09-30 | 用户确认 Q1/Q2 基础规则。 | 仅直接成员可进入组织、祖先角色可继承但不扩大数据范围；采用四级范围并默认拒绝。 | Phase 3 依据已确认的组织边界实施；Q2 的多角色合并及 SELF 字段留待业务资源定义。 |
| 2026-09-30 | Phase 3 在隔离 PostgreSQL 17.6 达成 API 语义验收。 | 双向两租户负向矩阵及平台管理员审计、同租户回归通过；跨租户信息泄漏和全量替换误删得到修复。 | Phase 3 标记 `COMPLETED`，指针推进 Phase 4；Windows 本机端口探针问题在 Phase 12 处理。 |
| 2026-09-30 | 用户确认 Phase 4 会话策略。 | 登出、踢出、改密立即撤销；成员/组织/租户失效在下一次请求拒绝；多设备独立。 | 使用持久会话和每请求数据库校验，不使用正向授权缓存。 |
| 2026-09-30 | Phase 4 在隔离 PostgreSQL 17.6 达成验收。 | 会话迁移、真实数据库生命周期测试、空库 15 条迁移及前端构建通过。 | Phase 4 标记 `COMPLETED`，指针推进 Phase 5；应用 token 另行设计。 |
| 2026-09-30 | 用户确认 Phase 5 应用令牌契约。 | 每个应用使用独立受众、令牌最多 5 分钟；业务后端用机密客户端凭据逐请求内省，撤销立即生效。 | 应用令牌绑定当前平台会话、用户、租户、组织与应用；客户端密钥只在服务端保存。 |
| 2026-09-30 | Phase 5 在隔离 PostgreSQL 17.6 达成验收。 | `0016`～`0019` 迁移、真实数据库双租户双应用回归 3 项及 Rust 普通测试/检查通过；配置恢复不复活旧令牌。 | Phase 5 标记 `COMPLETED`，指针推进 Phase 6。 |
| 2026-09-30 | Phase 6 需要数据范围业务规则。 | 多角色合并与 SELF 归属主体尚未确认，现有表也没有数据范围字段；已向用户请求确认。 | Phase 6 标记 `BLOCKED`，待规则确认后继续。 |
| 2026-10-02 | 用户调整 Phase 6 授权边界。 | 角色与数据权限应分开；角色仅决定菜单和操作，数据范围独立授权。 | 废弃“多角色合并数据范围”设计；Phase 6 待独立策略绑定粒度确认后实施。 |
| 2026-10-02 | 用户确认独立数据授权绑定粒度。 | 按当前组织直接成员、应用、业务资源和操作显式授权；没有记录默认拒绝。 | Phase 6 建立独立授权表和上下文契约，角色变更不改写数据范围。 |
| 2026-10-02 | Phase 6 在隔离 PostgreSQL 17.6 达成验收。 | 上下文 API、统一菜单计算、双实例即时可见和双租户组织继承回归通过。 | Phase 6 标记 `COMPLETED`，指针推进 Phase 7。 |
| 2026-10-02 | Phase 7 完成用户视图与业务后端校验路径。 | 旧对外路径收敛当前用户、参考组件 Node 5 项及真实 PostgreSQL 3 项回归通过。 | Phase 7 标记 `COMPLETED`，指针推进 Phase 8；示例微应用页面迁移在 Phase 8。 |
| 2026-10-02 | Phase 8 完成完整上下文与生命周期管线。 | 31 项 Node 回归、11 个真实浏览器场景、TypeScript 和生产构建通过；真实 Vue/jQuery vendor 接入验证成功。 | Phase 8 标记 `COMPLETED`，指针推进 Phase 9；生产入口及信任协议在下一阶段。 |
| 2026-10-02 | 默认 IFRAME，平台管理员显式标记可信；首版人工预检、启用和回滚，提供两种网关配置。 | 用户确认应用信任与首版发布规则；存量代码不自动获信任。 | Phase 9 按此实现，旧 app token 保留最多 5 分钟兼容窗，显式运行模式变更立即撤销对应应用 token。 |
| 2026-10-02 | Phase 9 完成独立发布和隔离接入。 | 8 个真实 PG/HTTP gate、21 个浏览器场景与两网关 HTTP、打包器等门禁通过；WSL 传输经局部 TCP 分段对照验证。 | 标为 `COMPLETED`，指针推进 Phase 10；实际生产容器、域名与网络留待交付阶段。 |
| 2026-10-02 | 用户确认持久通知发布权限、目标和幂等规则。 | notification:publish 加独立 notification/publish 数据授权；当前租户有效且可访问应用的目标；五项命名空间同键同内容复用、不同内容 409。 | Phase 10 按确认规则实施；旧无归属通知采用保留隔离的迁移策略。 |
| 2026-10-02 | Phase 10 完成持久通知全链路。 | 7 项真实 PG、9 项实际 main/PG/SocketIO 联合与客户端/服务/网关验收通过；全部自有测试资源已清理。 | 标为 `COMPLETED`，指针推进 Phase 11。 |
| 2026-10-02 | 用户确认工单试点提案。 | 明确字段、状态重开、SELF 创建人、软删除保留、手动提醒以及独立 Vue iframe 和业务 API。 | Q8 解决，Phase 11 按 `docs/business-pilot-proposal.md` 实施。 |
| 2026-10-02 | 用户确认本机隔离原生准生产工程交付验收。 | 按提案实际 HTTPS、故障／告警、加密备份与新库恢复；RPO 24h／RTO 1h 为演练目标，无商业 SLA，不自动清理数据。 | Phase 12 使用已确认目标；实际 Docker 运行留待 runner，并在交付报告明确。 |
| 2026-10-07 | 用户通知继续，Phase 11 完成最终正常全量验收。 | 实际平台、独立业务库、Vue、通知、CSV 与发布回滚 16/16 通过；宿主 100 文件哈希和页面／会话保持，自有活跃测试资源清理为 0。 | Phase 11 标记 `COMPLETED`，指针推进已确认目标的 Phase 12。 |

---

## 7. Open Questions

| ID | Question | Impact | Owner | Status |
| --- | --- | --- | --- | --- |
| Q1 | 已确认：仅直接成员可进入组织；进入后继承祖先角色，继承不扩大数据范围；首版不提供子组织拒绝覆盖。 | Phase 1、2、3、6、11 的授权矩阵和约束 | 用户 | RESOLVED |
| Q2 | 已确认：首版采用 `SELF/ORG/ORG_SUBTREE/TENANT` 四级且默认拒绝；角色只决定菜单和操作权限，数据策略按当前组织成员、应用、资源及操作独立显式授权。`SELF` 归属字段按业务资源声明并由服务端验证。 | Phase 1、6、7、11 的数据范围模型 | 用户/业务方 | RESOLVED |
| Q3 | 首版平台库和业务库分别部署、租户按既有逻辑隔离；工单软删除和通知／审计数据不自动清理，用户已确认该初版交付口径。正式保留期和特定租户物理独立库仍待后续产品／运维决策。 | 后续数据迁移与运行策略 | 产品/运维 | PARTIALLY_RESOLVED |
| Q4 | 已确认：应用后端通过 HTTPS 访问内省接口；每应用独立受众，最多 5 分钟，逐请求内省且立即撤销。客户端密钥只保存在业务后端的服务端配置或密钥管理中。 | Phase 4～7 的令牌设计 | 用户 | RESOLVED |
| Q5 | 已确认：新应用和存量升级默认 IFRAME；只有平台管理员明确标记的自有可信应用可用 HTML/ESM 在宿主页执行。 | Phase 8、9 的信任和发布边界 | 用户 | RESOLVED |
| Q6 | 已确认首版人工预检、启用与回滚，提供 Caddy 和 Nginx 接入配置；实际生产网关及复杂审批/灰度需求仍待运维确认。 | Phase 9、12 的部署与回滚 | 产品/运维 | PARTIALLY_RESOLVED |
| Q7 | 已确认本机隔离 Linux 原生准生产环境，专用测试 CA 的实际 HTTPS、健康/指标和故障告警；RPO24h/RTO1h演练、不承诺商业SLA。现网域名和监控接入留实际现网部署时确认。 | Phase 12 已确认工程验收；后续现网接入 | 用户/运维 | RESOLVED_FOR_INITIAL_DELIVERY |
| Q8 | 已确认采用文档中的最小真实工单试点，流程／字段／SELF 和软删除及手动提醒按 `docs/business-pilot-proposal.md`；正式保留期仍待后续运维确认。 | Phase 11 的范围与验收 | 用户 | RESOLVED |
| Q9 | 本机 Windows Rust/.NET/Nginx回环受限，Chromium IPv6与Caddy可用；WSL较大TCP分段丢失已由8KB SQL对照确认，测试进程TCP_MAXSEG=1000后完整gate通过。实际生产网络仍需单独验收。 | Phase 12 的网络与部署验收 | 技术负责人/运维 | PARTIALLY_RESOLVED |
| Q10 | 初版不自动清理会话／IP/UA／审计记录，用户已确认该交付口径；提供运维配置和说明，正式保留期与清理批次另行确认后实施。 | 后续合规与容量运行 | 用户/运维 | RESOLVED_FOR_INITIAL_DELIVERY |

---

## 8. Change Log

| Version | Date | Change |
| --- | --- | --- |
| v0.1 | 2026-09-30 | 基于需求分析和现有代码创建 12 阶段任务台账，所有阶段为 `NOT_STARTED`，指针指向 Phase 1。 |
| v0.2 | 2026-09-30 | Phase 1 完成：规则草案、风险清单、只读审计、两租户夹具与 API 探针；指针移至 Phase 2。 |
| v0.3 | 2026-09-30 | Phase 2 迁移、约束测试和说明完成离线验证；因隔离 PostgreSQL 未就绪标为 `BLOCKED`。增加 `.env.test` 配置模板并记录待运行门禁。 |
| v0.4 | 2026-09-30 | 用户授权隔离库建库；实现后端可选自动建库，完成 PostgreSQL 17.6 新库和合法存量迁移验收；Phase 2 标记 `COMPLETED`，指针推进 Phase 3。 |
| v0.5 | 2026-09-30 | Phase 3 修复管理 API 租户范围、共享账号写入和跨租户审计；双租户真实数据库 API 矩阵通过，指针推进 Phase 4。 |
| v0.6 | 2026-09-30 | Phase 4 完成持久平台会话、即时撤销、前端适配和两设备/状态失效测试；指针推进 Phase 5。 |
| v0.7 | 2026-09-30 | Phase 5 完成应用令牌受众、绑定、逐请求内省、撤销迁移与真实数据库回归；指针推进 Phase 6。 |
| v0.8 | 2026-09-30 | Phase 6 等待多角色数据范围合并与 SELF 归属语义确认，状态标为 `BLOCKED`。 |
| v0.9 | 2026-10-02 | 根据用户反馈将角色权限与数据授权分离，修正 Phase 6 阻塞点和需求契约草案。 |
| v1.0 | 2026-10-02 | 用户确认按成员关系独立授权；Phase 6 完成迁移、上下文 API、统一菜单权限计算和双实例回归，指针推进 Phase 7。 |
| v1.1 | 2026-10-02 | Phase 7 收敛旧对外菜单/角色接口，增加服务端校验示例及跨租户资源 ID 测试，指针推进 Phase 8。 |
| v1.2 | 2026-10-02 | Phase 8 完成 MicroProps 2.0、取消与续期、串行生命周期及真实示例接入；31 项 Node、11 个浏览器场景通过，指针推进 Phase 9。 |
| v1.3 | 2026-10-02 | Phase 9 完成不可变独立发布、真实预检、回滚、默认 iframe 和两网关实测，指针推进 Phase 10；记录用户已确认通知规则与独立实时服务子项并行进展。 |
| v1.4 | 2026-10-02 | Phase 10 完成受权持久通知及实际平台联合验收；记录用户确认的工单提案，指针推进 Phase 11。 |
| v1.5 | 2026-10-07 | 从用户暂停恢复，Phase 11 独立工单与最终实际联合验收 16/16 通过；同步已确认的全局规则，指针推进 Phase 12。 |
