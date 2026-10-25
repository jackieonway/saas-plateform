# Phase 5：管理 UI 与缓存会话隔离验收

日期：2026-10-25。状态：**COMPLETED**。后端格式／all-targets 检查、Clippy、3 项新增 PostgreSQL HTTP 专项、31 项相关 PostgreSQL 回归、185 项普通测试及文档检查已通过。前端保留首轮 **443 项，434 PASS／9 FAIL**，修复后 **443／443 PASS**；新增用例后的 **456 项首轮 455 PASS／1 FAIL** 也已修复。生产 StrictMode 顺序修正后，最新完整 **456／456 PASS，9.70 秒**，lint 再次为 **0 errors／46 warnings**，最终生产构建及 dist 交付检查已通过。成员管理浏览器 **14／14 PASS**、策略浏览器 **5／5 PASS**、额外 micro setup **10／10 PASS**，原九个浏览器入口均有逐组实际通过记录；本阶段五项验收标准均为 **PASS**。保留失败、修复和分组重跑过程，不宣称单次九入口全部通过，也不代表正式环境已开放创建或部署。

范围见[主台账 Phase 5](../task-ledger.md)，API 与会话合同见[控制台成员证明与会话隔离](../phase-05-console-identity.md)。本文只记录本阶段实际结果，不把之前阶段的全部安全回归或工作区既有改动归于本轮。正式环境创建开关继续关闭，没有发布 Phase 6 收缩迁移、客户最小 API、客户门户或 OIDC。

## 本阶段实现与验收边界

- [控制台认证路由](../../../src/routes/auth.rs)在 context／profile 顶层返回服务器证明的具体 `membershipId`、`memberType`、十进制字符串 `identityVersion`／`accountVersion`。选择项返回真实账号、成员、类型、状态和代际；客户与停用员工可以展示，但不会进入可签发员工 context 的 ticket 快照。
- [生命周期路由](../../../src/membership_lifecycle/routes.rs)新增只读 `GET /api/memberships/capabilities`，要求员工 session 和 `system:user`，精确报告三个服务器 config 开关。读取不开放写入，客户不能借该接口取得管理资格。
- 管理端按员工／客户／全局账号分别呈现；新增第二身份与替换采用组合生命周期接口，并显式处理源、目标、目标版本、停用目标的启用确认和客户账号绑定依据。普通登录选择提交具体 `membershipId`，不写成员类型或创建关系。
- 前端共享会话保存完整服务器证明，缓存、请求 fence、通知上下文、最近页面／搜索及侧栏菜单折叠命名空间纳入具体成员及代际。旧缺证明记录要求重新登录；晚返回结果不能补入新身份。应用／客户端策略成功保存后，在捕获的会话 fence 下同步取消当前请求、清理 profile／当前应用并提升 epoch，触发重新挂载及授权重读，不通过伪刷新 token 完成失效。此处不宣称所有标签页都立即收到策略变更广播。具体实现及输入规则见合同文档，运行证据分项记录如下。

客户控制台登录仍关闭。真实客户 context 的独立入口及其最小本人资料合同在后续阶段实施。本阶段浏览器可以检查同账号同组织的两条成员关系及禁选原因，不能把受控 fixture 注入的成功响应描述成生产客户准入。

## 已完成的后端门禁

以下由 root 在统一 Linux runner 执行，未由并行 agent 启动第二套 Cargo 构建。数字为本轮实际报告：

| 门禁 | 结果 | 时间与范围 |
| --- | --- | --- |
| 初始格式检查＋all-targets check | PASS | 合计 **58.10 秒**。 |
| all-targets Clippy | PASS | **68 秒**。 |
| 新增 Phase 5 PostgreSQL HTTP 专项 | **3／3 PASS** | 测试 **5.07 秒**；各自有数据库全部清理。 |
| 普通测试 | **185 PASS** | library 9、主 binary 163、其他 target 13；主 binary 测试 **6.81 秒**。 |
| 文档检查 | PASS | **19.38 秒**。 |

新增专项位于 [identity_ui_tests.rs](../../../src/identity_ui_tests.rs)，只读取显式 `IDENTITY_PHASE05_TEST_DATABASE_URL`，要求选中 `postgres` 管理库，只创建、迁移及清理本轮 `identity_phase05_*` 自有数据库。迁移截止 0015；双身份正向用例仅在该自有库移除旧二元唯一键，不修改真实部署迁移。测试过滤入口为：

```sh
cargo test --locked --bin saas-platform-backend identity_ui_tests -- --ignored --test-threads=1
```

三个专项分别证明：

| 用例 | 实际检查 |
| --- | --- |
| `phase05_login_and_profile_show_identity_metadata_without_expanding_ticket_authority` | 真密码登录、同账号多类型／多组织／停用员工选择项的实际成员 ID、账号 ID、状态与版本；签名 ticket 只含启用员工；context、profile 和服务器 session 的完整证明一致。 |
| `phase05_context_selection_is_exact_and_switching_does_not_modify_memberships` | 客户 ID 不能回退同组织员工、账号／组织不匹配、客户专属组织、停用员工、UUID v4／未知字段及缺选择拒绝；旧组织兼容选择只取有效员工；切换前后成员快照相同，旧 session 已撤、新 session 绑定目标；旧代际 ticket 被拒。 |
| `phase05_capabilities_reflect_server_gates_and_customer_console_access_stays_closed` | 实际非超级管理员 `system:user` 角色；四组 config 值精确返回；无权限员工与有效客户 session 被拒；全关闭时写入及数据库变化被阻止；客户专属账号的旧控制台登录、context、profile 仍关闭。 |

`userId` 在 profile 选择项中统一由真实 `memberships.user_id` 返回，新增断言同时核对该字段。此一致性修正已经在上述编译及 3 项专项运行前写入，相关断言已通过。

本阶段相关的五组既有后端专项已全部通过，保持各组实际运行记录：

| 专项组 | 结果 | 测试时间 |
| --- | --- | --- |
| identity binding | **6／6 PASS** | **5.48 秒**。 |
| membership lifecycle | **4／4 PASS** | **13.45 秒**。 |
| platform session refresh | **7／7 PASS** | **3.70 秒**。 |
| login protection | **5／5 PASS** | **50.01 秒**。 |
| employee isolation | **9／9 PASS** | **13.35 秒**。 |

五组共 **31／31 PASS**，为成员组合、员工 bulk／双向角色隔离、账号／成员／来源证明、刷新及登录限制提供直接后端回归。它们与新增 3 项 HTTP 专项分别记录，不把普通测试的 ignored 项算作数据库通过。专用 PostgreSQL runner 的最终清理检查中，非模板数据库仅剩 `postgres`，通知测试角色计数为 **0**。

本阶段采用已有后端安全链路，没有用浏览器 fixture 重新定义其授权含义。[Phase 1](phase-01.md)记录具体成员组合、停用目标显式启用及原关系保留；[Phase 2](phase-02.md)记录账号／成员／来源／应用版本和策略撤销；[Phase 3](phase-03.md)记录员工 bulk、双向角色和永久退出后的授权清理。这些链路中本阶段相关五组已按上表重跑。[Phase 4](phase-04.md)的真实 PostgreSQL／两个服务 Socket.IO 用例提供通知收件与在线局部撤权的既有证据，本阶段新增通知前端场景验证消费端作用域；不把旧阶段的两服务运行描述成本阶段重新部署或重新运行。

## 前端已运行结果与首轮失败

活动 SDK pinned 检查和 qiankun 源码检查已报告 PASS。固定 SDK 与 Context 1、MicroProps 2、Notification 1 四类发行合同／历史副本保持原义；本阶段新增 proof 属于独立管理端 DTO，没有改写固定协议字段来容纳 UI 身份状态。最终构建后 SDK pinned 再次通过，固定发行物检查与本阶段源码／dist 门禁已完成。

首轮完整 Node 运行保留为 **443 项：434 PASS／9 FAIL**，不把随后的子集通过合并成一次完整通过：

| 首轮失败组 | 数量 | 已识别原因与处理状态 |
| --- | --- | --- |
| 导航 fixture | 3 | 缺少新的完整服务器成员 proof；已补齐受控响应。 |
| 用户 fixture | 3 | 新组件抽取后，旧相对组件 loader 不覆盖新加载路径；已修复测试加载边界。 |
| data-grant fixture | 1 | 裸 token 设置缺少具体成员证明；已改为显式 fixture identity。 |
| micro fixture | 2 | 缺少新的 `contextKey`；已补齐完整命名空间。 |

上述修复后的完整重跑为 **443／443 PASS，8.71 秒**。随后加入本阶段用例，完整集合扩为 456 项；首轮为 **455 PASS／1 FAIL**，唯一失败是旧 PUBLIC client fixture 缺少 Phase 2 已要求的用途／身份集合元数据。修正 fixture 为真实 UUID v7、`USER`、`EMPLOYEE`、策略版本 1，保留原 PKCE 兼容断言；相关定点 **12／12 PASS** 后再次运行完整集合，最终 **456／456 PASS，9.36 秒**。没有删除 PKCE 兼容检查来绕过失败。

已报告的定点通过结果如下；这些子集可能相互重叠，**不累加成额外完整用例总数**：

| 定点运行 | 结果 | 报告来源与范围 |
| --- | --- | --- |
| context identity＋recent visits | **6 PASS** | root 定点运行。 |
| membership model | **18 PASS** | root 定点运行；验证 DTO、能力门禁、绑定依据、源／目标与版本等模型规则。 |
| auth state＋micro routing | **7 PASS** | root 定点运行。 |
| session integration | **21 PASS** | root 定点运行。 |
| 三套 session 专项 | **70 PASS** | 会话实施 agent 报告；完整运行另以上述 456 项结果为准。 |
| query＋notification 专项 | **33 PASS** | 会话实施 agent 报告；完整证明、旧代请求与通知作用域检查，完整运行另以上述 456 项结果为准。 |

最终前端静态及产物门禁：

| 门禁 | 结果 | 范围 |
| --- | --- | --- |
| TypeScript typecheck | **0 error** | 10 秒内完成；未取得精确用时，不另推算。 |
| lint | **0 errors／46 warnings** | 以现有警告为主；生产 StrictMode 修正后再次运行，结果相同。 |
| `npm run build` | PASS | 生产 StrictMode 修正后退出 0，含 TypeScript 构建及 Vite 产物；最后 Vite **18.44 秒**。此前通过的 Vite **20.94 秒**记录保留。 |
| dist qiankun 交付检查 | PASS | 最后构建 **401 files**，SDK **3.0.0**，`buildChecked=true`。 |

追加两个显式 npm 浏览器脚本与报告环境变量说明后，`test-entrypoints` 定点 **1／1 PASS**，SDK pinned 再次 PASS，tracked 文件 `git diff --check` 通过。调用方式及报告位置见[浏览器 README](../../../../frontend/tests/browser/README.md)：`npm run test:identity:management:browser` 与 `npm run test:admission:policy:browser`；报告目录由各自显式环境变量指定，不写入临时凭据路径。

本阶段没有改动固定 wire／SDK，因此没有据此新增一次跨工作区七仓产物全量发布或验证结论。

## 成员与策略浏览器及既有场景逐组验收

成员管理专用浏览器已完成 **14／14 PASS**，实际环境为 **Edge 142.0.3595.94／Playwright 1.64.1**。它运行真实 React/Vite `UsersPage`，HTTP 为隔离 fixture，没有 PostgreSQL 断言。结果见[成员浏览器 JSON](../../../../frontend/target/identity-sso-phase05/members/identity-management-browser.json)。

场景覆盖桌面三视图与客户员工字段隔离、移动列表／弹窗、三类 config 门禁、能力依赖不可用、当前具体员工的自保护、员工／客户独立启停删除、停用目标的显式复用和替换源停用、旧版本拒绝、精确已有账号及绑定依据、未知目标依赖阻止组合，以及旧员工 bulk 排除客户／保留停用状态。首轮成员浏览器因 `@username` 选择器不符合实际显示失败；修正选择器后完成上述 14 项，没有以放宽业务断言代替交互。

共保存 7 张实际截图：

- [桌面员工](../../../../frontend/target/identity-sso-phase05/members/desktop-employees.png)、[桌面客户](../../../../frontend/target/identity-sso-phase05/members/desktop-customers.png)。
- [移动员工](../../../../frontend/target/identity-sso-phase05/members/mobile-employees.png)、[移动客户](../../../../frontend/target/identity-sso-phase05/members/mobile-customers.png)、[移动操作](../../../../frontend/target/identity-sso-phase05/members/mobile-member-action.png)。
- [第二身份确认](../../../../frontend/target/identity-sso-phase05/members/desktop-second-identity-confirmation.png)、[客户账号绑定](../../../../frontend/target/identity-sso-phase05/members/desktop-customer-account-binding.png)。

root 已实际查看桌面客户、移动客户及移动操作截图，当前布局检查合格。

策略专用浏览器已完成 **5／5 PASS**，使用真实 `main`、session gate、缓存及 React/Vite 管理页面，HTTP 权威仍为隔离 fixture。结果见[策略浏览器 JSON](../../../../frontend/target/identity-sso-phase05/admission/admission-policy-browser.json)。五个场景分别检查应用多选保存后立即退役旧 profile／菜单子树／活动请求、客户端 409 后重新读取并要求新原因和确认、USER 默认与 PUBLIC／CONFIDENTIAL／MACHINE 分离、固定控制台来源不可改客户，以及移动弹窗和影响确认。

策略首轮失败实际暴露生产 StrictMode effect 顺序问题：`useQuery` 在 StrictMode replay 中先于 `mounted` effect 的 setup 重放，守卫判为未挂载而抛 `AbortError`，HTTP 请求计数为 0。最小修复将 mounted effect 放在 `useQuery` 之前，保留原守卫和 StrictMode；没有删除取消保护或改成宽松 mock。修复后以上五场景通过，并再次运行完整 Node **456／456 PASS，9.70 秒**。

策略验收共 6 张成功截图：[应用准入](../../../../frontend/target/identity-sso-phase05/admission/desktop-application-admission.png)、[授权投影退役](../../../../frontend/target/identity-sso-phase05/admission/desktop-projection-retired.png)、[客户端 409](../../../../frontend/target/identity-sso-phase05/admission/desktop-client-policy-conflict.png)、[机器用途](../../../../frontend/target/identity-sso-phase05/admission/desktop-machine-policy.png)、[新 USER 默认](../../../../frontend/target/identity-sso-phase05/admission/desktop-new-user-default.png)、[移动准入](../../../../frontend/target/identity-sso-phase05/admission/mobile-client-admission.png)。历史失败截图另行保留，不计入这 6 张成功截图。root 已实际查看桌面 409 和移动截图，布局检查合格。

首轮完整九个浏览器入口运行中，notification recovery 场景通过后，清理诊断引用未定义的 `queryClient`，遮蔽了原始失败信息。随后修正 async predicate、安全诊断与按 pathname 匹配的 query-safe route；notification events 单场景重跑 **1／1 PASS**。

第二轮原九入口集合先完成前六组，失败 fixture 修复后另两组补过；当前逐组记录为：

| 浏览器入口 | 结果 |
| --- | --- |
| notifications | **11／11 PASS** |
| platform settings | **11／11 PASS** |
| navigation | **16／16 PASS** |
| login | **2／2 PASS** |
| regions | **14／14 PASS** |
| entities | **27／27 PASS** |
| departments（fixture 修复后） | **11／11 PASS** |
| route permissions | **64 PASS**，含成员／身份代际／账号代际／应用菜单折叠隔离 |
| micro qiankun | **4／4 PASS**，最后分组进程退出 0 |

接着 department fixture 的组织 `code` 拼接完整 36 字符 UUID，超过真实字段 32 字符限制，导致该入口失败。修正 fixture 生成合法短组织 code，原 UUID 选择断言保留；department、route 和最后 micro qiankun 重跑均已通过，后三组串行进程退出 0。已通过且源码无变化的前六组没有重复执行；上表按前六组与后三组的实际覆盖记录，**不宣称单次九入口全部通过**。

额外 `micro-setup-browser` 首轮 **9 PASS／1 FAIL**，失败来自移动布局在动画 resize 瞬间取得的 bounds。测试改为利用 `requestAnimationFrame` 和 `getAnimations()` 等待真实几何稳定，未修改 x、width、overflow 断言阈值；移动定点 **1／1 PASS** 后，完整 **10／10 PASS**、进程退出 0。root 已实际查看移动布局截图。

至此，原九个浏览器入口、新成员／策略入口及额外 micro setup 均有明确通过结果。源代码与构建门禁、数据库清理及分组失败重跑记录已在上文分别列出，没有把待执行项目或未运行的跨仓发布计入通过范围。

## 浏览器与 PostgreSQL 证据分别证明什么

真实源码 UI 的受控 HTTP／Socket.IO 浏览器 fixture 用于检查成员分视图、具体身份选择、禁用按钮、显式确认、缓存／菜单／通知清理、多标签页协调和旧响应晚返回。它可以证明实际组件使用何种请求和何种服务器响应，不能证明模拟 handler 已完成真实数据库事务、权限隔离、版本撤销或收缩迁移。

PostgreSQL HTTP 专项直接验证真实 SQL、服务器 session、签名 ticket、授权与回滚；生命周期、员工角色、数据授权和实时通知还应引用相应已有专项的本轮重跑。Rust HTTP 测试不能代替浏览器点击、弹窗约束或多标签页的运行结果。

同组织 EMPLOYEE／CUSTOMER 共存展示、客户禁选和完整命名空间应作为 Phase 5 浏览器检查；可用客户业务 context 的签发及客户入口访问不属于本阶段成功条件，不能为浏览器演示放宽旧控制台后端门禁。

## 五项验收标准

五项标准均已完成，按界面证据与真实后端边界分别给出结论：

| 台账标准 | 当前结论 | 实际依据与边界 |
| --- | --- | --- |
| 员工 bulk 只操作员工；客户刷新后保留且不能获员工角色 | **PASS** | 成员浏览器检查员工 bulk 请求、客户隔离及刷新；本轮 employee isolation 9 项与既有 Phase 3 PostgreSQL 证据核验客户原成员保留和双向角色拒绝。 |
| 双身份、多组织、多标签页、替换和晚返回不混用菜单／数据／通知；普通切换不改成员 | **PASS** | 新增 HTTP 专项核验普通切换前后成员快照及旧 session 撤销；完整 Node 和九浏览器入口检查具体成员／代际作用域、旧响应及页面隔离；通知服务授权语义引用 Phase 4 真实双服务证据，客户控制台仍禁选。 |
| 第二身份保留源；替换明确源／目标，不恢复停用目标旧权；独立启停 | **PASS** | 模型与成员浏览器核验源／目标、版本、复用／启用确认和独立操作；本轮 lifecycle／employee PostgreSQL 专项及 Phase 1～3 证据核验真实状态、授权和会话清理，fixture 仅在自有 schema 模拟双行。 |
| 关闭创建时 UI/API 均拒绝；固定发行物不变 | **PASS** | 新增 PostgreSQL 专项验证真实 config／写入拒绝；成员浏览器覆盖关闭及权威不可用时按钮禁用；SDK pinned、源码／dist 检查与最终构建均通过，正式环境三开关仍关闭。 |
| 应用／客户端集合安全默认并可保存；收紧后旧页面／凭据不保留撤准入 | **PASS** | 策略浏览器验证实际多选保存、409 重读、USER／MACHINE 及客户端类型分离、固定员工来源与当前页投影退役；本轮 binding 与 Phase 2 真实策略／凭据证据验证旧链路撤准入；最后构建通过，不宣称所有标签页即时策略广播。 |

Phase 5 在上述范围完成。下一阶段按主台账进入 Phase 6 的客户最小 API、独立入口准入与收缩迁移验收；本阶段没有实施这些后续能力，不更改正式创建开关，也不把隔离 fixture 的双身份测试当作生产开放或部署完成。
