# Phase 4 通知的成员绑定与实时权威证明

本文描述当前实现；执行状态与真实测试结果以[台账](task-ledger.md)及阶段验收为准。客户、第二身份及新增生命周期入口的真实启用仍等待 Phase 6，不执行历史通知搬迁。

## 持久化和发送

增量迁移 `0015_notification_member_binding.sql` 为通知增加 `recipient_membership_id`、`recipient_member_type`、`recipient_identity_version`，为 publication 增加对应的 publisher 三列。证明必须全部为空或全部完整，完整证明使用 UUID v7、已知成员类型和正整数身份版本；创建后不得改绑。

成员证明采用保留原值的软引用。删除成员不会把旧消息挂到新成员，也不会清空其原始成员归属。现有账号、组织和应用的整体删除级联语义维持原有合同。APP 投递唯一键为 publication、收件成员和身份版本，发布幂等命名空间同时包含发布成员与代际。`recipientCount` 继续按不同账号计数，`deliveryCount` 按具体成员投递计数；同账号在多个组织的合法员工关系可以分别接收。

APP 发布者必须是当前有效员工，具有应用操作权限和独立数据范围，应用也必须明确允许员工。控制台发布和 app token 发布使用相同规则。客户用途未登记时不能发送客户通知。缺省组织／租户广播只展开有效员工，同组织客户行不参与展开，不产生重复投递。旧 SYSTEM 三元上下文接口仅解析有效员工并去重，不会将客户关系当成员工资格。

所有旧无证明行保持原值，既不根据当前成员回填，也不自动归属于当前员工或客户。普通收件箱不展示这些行；有平台审计权限的员工通过 `/api/notifications/legacy` 查看归档。新库没有实际历史回填工作，旧结构行为由隔离 fixture 验证。

## HTTP 收件箱与偏好

通知接口使用专用 `NotificationContext`，只接受可由服务器记录证明的有效员工或客户 session。通用员工管理的 `CurrentUser` 仍保持员工门禁。请求中的账号、组织、成员或版本字段不能覆盖 session 绑定。

列表、详情、未读数、已读、批量删除和 socket 可见性共用具体成员及身份版本条件。APP 消息还要求当前应用启用、租户／组织授权有效，并允许当前成员类型。非本身份的详情与删除返回 404，标记已读返回未修改；批量操作只作用当前身份可见的记录。

修改接口在事务内完成操作，所有通知行锁等待结束后，再以独立语句复验具体 session 和受影响 APP 的当前可见性，失败整体回滚。读取在返回前复验 session。权威状态不可确认返回 503，不将其描述为密码错误或失效凭证。

`notification_member_prefs` 以 `(membership_id, identity_version)` 为主键。默认免打扰关闭、时间为 `22:00–07:00`，静音类型为空。不同组织、同组织双身份及成员重新启用后的新版本各自使用独立偏好；旧 `notification_prefs` 留存，不复制到新身份。账号全局版本用于凭据撤销，不作为持久收件箱的分代键。

## 固定公共协议与内部证明

已发布 Notification 1、Context 1 和 MicroProps 2 的生成副本不变。公开认证输入仍为 `{token}`，Socket.IO `authenticated` 仍为五字段，`notification` 仍为十二字段；原 `/api/notifications/socket-context` 仍返回六字段的平台 socket context。

通知服务改用内部 `GET /api/internal/notifications/context-proof`，请求仍携带当前 session bearer，可选 `notificationId`。返回精确十一字段：

```text
proofVersion="1", contractVersion="1.0",
userId, tenantId, organizationId, sessionId, expiresAt,
membershipId, memberType, identityVersion, accountVersion
```

UUID 必须为规范 UUID v7；`identityVersion` 为大于等于 1 的 JSON i64 整数，`accountVersion` 为大于等于 0 的 JSON i64 整数。到期时间同时受数据库 session 和 JWT 上限约束。指定通知必须对该具体身份可见，否则返回 404。无效或已撤销的 session 返回 401。

数据库私有 `new_notification` 提示也明确版本，精确八字段：

```text
hintVersion="1", id, userId, tenantId, organizationId,
membershipId, memberType, identityVersion
```

旧四字段或畸形提示被丢弃。提示没有持久重放保证，断线恢复仍从 HTTP 补拉；不通过重新发布业务通知补偿提示丢失。

实时队列按账号、租户、组织、成员、类型和身份版本分区。每条事件从持久行重新取得收件证明，并再次调用平台验证可见性。连接固定绑定完整成员证明、session 和账号版本，不允许改绑或延长首次期限。首次与周期复验失败终止连接；逐条投递权威故障拒绝该条投递，不缓存正向授权。成员撤销只影响该成员；账号版本提升覆盖该账号全部连接。

通知服务仍不持有平台签名密钥，不写收件箱、不迁移平台库、不直接读取账号或成员表。专用数据库角色仅增加三个通知证明列的 SELECT 权限。真实平台权威测试、受控 authority 的通知消费者测试以及两个真实服务的联调证据分别记录，不能互相冒充。
