# 成员与访问控制

账号由身份服务拥有；租户、组织、成员与授权由平台拥有。一个成员用独立 UUIDv7 标识，绑定账号、组织与 `EMPLOYEE` 或 `CUSTOMER` 类型；数据库从首次初始化即使用三元唯一键。

控制台只接受当前有效员工成员。客户门户只接受登记 RP 允许且当前有效的客户成员。成员列表可以展示不可选项及明确原因，客户端不能通过组织 ID 推断、补建或扩大成员身份。

会话证明绑定 user、tenant、organization、membership、memberType、identityVersion、accountVersion 和原始认证来源。账号或成员代际变化、组织停用、来源 RP 策略变化、会话撤销和上下文切换都会使旧证明失效。

平台菜单和功能权限按具体成员、角色及应用求交集，业务资源权限还要核对动作和 SELF/ORG/ORG_AND_CHILDREN/TENANT 范围。前端过滤用于交互；服务端每次请求重新检查权威状态。

成员写入与客户能力的显式开关按实际部署配置，不再依赖旧阶段 rollout receipt。并发修改遵循账号、应用/来源、成员和会话的既定锁顺序；事务提交前复核原始证明，不用新代际修补旧请求。

客户端用途由当前登记约束，人员授权与机器 `client_credentials` 分开。人员登录仅在 SSO 完成，平台没有 password grant 或本地登录 ticket。
