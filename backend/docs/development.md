# 本地开发

先按[全新安装](fresh-installation.md)准备平台共享库、门店库、工单库和客户门户库。平台唯一结构文件为 `0001_init.sql`，不运行旧升级链。数据库结构初始化与账号/RP/业务授权登记分开处理。

平台与身份服务使用准确且不同的 HTTPS origin，由本机网关与可信 CA 提供 TLS。身份服务负责账号认证与 OIDC；平台和客户门户分别登记 WEB_BFF / USER / PLATFORM_CONTEXT RP，成员类型分别限定 EMPLOYEE 和 CUSTOMER。准备精确 callback、独立 secret、RS256 keyset 和 BFF AEAD keyring，不能复用凭据。

平台需配置 `DATABASE_URL`、`JWT_SECRET`、`PUBLIC_APP_URL` 及[SSO 必需配置](sso-only.md)。独立身份部署还需身份服务 URL、CA、ACCOUNT_COMMAND、AUTH_ADMIN、APPLICATION_PROJECTION、KEY_ADMIN 及上下文来源等按用途隔离的服务登记，并在启动 runtime 前完成 owner/ACL 交接。按各仓库 README 设置身份、业务 API、客户门户与通知服务配置；平台不会代为启动它们。

在平台 `backend` 运行 `cargo run --locked`，在 `frontend` 运行 `npm ci`、`npm run dev`。后端默认 3001，宿主默认 3000，身份服务默认 3041。开发服务器只是内部端口；浏览器通过配置的 HTTPS 入口访问。

登录流程：访问平台 → SSO → 认证中心选择租户与员工身份 → 平台回调并消费选择结果 → 返回原页面。员工上下文切换同样前往认证中心，绑定原 BFF 会话版本。旧 BFF 模式开关、平台密码接口和公开 refresh 已删除。SSO 不可用时保持失败关闭。

新库不包含旧账号、会话或业务记录。受控初始化可用明确的管理员 bootstrap 配置建立最小平台数据；开发和生产首次 bootstrap 均需显式管理员配置；正常启动不再自动写入旧演示账号、租户或 OAuth 客户端。

系统菜单沿用原始可折叠目录：仪表盘独立，“组织机构”“用户与角色”“平台配置”“开放平台”下挂各功能页。目录由菜单数据的 `DIR` 与 `parent_id` 表达，前端使用现有树形展开/收起和搜索行为，不额外生成静态分类标题。已有库调整菜单需显式修改菜单配置；正常启动不重排管理员已经配置的菜单。

成员、客户能力和双身份写入通过当前显式功能开关配置；三元唯一键已在基线中建立，不使用旧 rollout receipt。

用户管理仅保留“新建员工／新建客户”，电话必填。前端通过 `POST /api/users` 的 `membership` 字段提交目标组织、成员类型、岗位和原因。后台按规范化后的唯一电话匹配当前可管理的已有账号：匹配成功则直接建立成员关系，保留原姓名、用户名、密码和联系方式；不匹配则要求填写姓名、用户名及初始密码，一并创建新账号和成员关系。邮箱可选，填写后须唯一；电话与邮箱指向不同账号、账号停用、已有同类型成员或管理范围不符时拒绝自动关联。此流程不合并两个不同账号，也不覆盖已有账号资料。

员工／客户创建和绑定由 `MEMBERSHIP_LIFECYCLE_ENABLED=true` 控制；客户成员和同组织双身份分别需要 `CUSTOMER_MEMBERSHIPS_ENABLED=true`、`DUAL_MEMBERSHIPS_ENABLED=true`。新账号由认证中心执行账号命令，平台通过同一持久操作记录完成成员关系；结果未知时前端保留原操作编号并查询恢复，不重新创建账号。已存在账号的自动关联使用成员生命周期事务及审计。

当前 `0001_init.sql` 为非空邮箱和电话建立全局唯一索引。邮箱比较忽略首尾空格及大小写，电话比较忽略空白、括号和短横线；停用／退役账号仍保留联系方式占用。唯一约束同时覆盖创建和资料修改，并发重复写入会被数据库拒绝。初始化超管可暂不填写电话，之后新增业务员工／客户及为旧账号创建成员时必须具有有效电话。结构变化使用当前空库基线，不改写旧库 SQLx checksum。

数据授权页的目标应用来自 `DATA_GRANT_POLICY` 中的应用白名单和管理边界，未配置时返回空列表。必须使用当前环境的应用、租户和组织 UUID，并让 capability 与该应用的有效菜单权限相交；参见[数据授权配置](data-grant-management.md)。修改启动策略后需重启平台，菜单管理权限本身不会自动填充目标应用。

修改后运行[工具链与验证](engineering-baseline.md)中的相应门禁。数据库测试要使用任务自有空库，记录实际通过范围，并清理本次运行资源；旧阶段报告不作为当前通过证据。

GitHub Actions 的数据库集成任务使用 runner 自有 PostgreSQL 服务，宿主端口为 `55494`。任务从该实例读取 `pg_control_system().system_identifier`，向当前 BFF 回归测试显式提供 `IDENTITY_PHASE24_TEST_DATABASE_URL` 和 `IDENTITY_PHASE24_TEST_CLUSTER_ID`；身份模型与权限测试分别使用 `IDENTITY_PHASE02_TEST_DATABASE_URL` 和 `IDENTITY_PHASE12_TEST_DATABASE_URL`。这些值只指向本次 CI 实例，各测试建立并清理自己的数据库，不使用开发或生产库。
