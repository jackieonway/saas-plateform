# 平台文档导航

本文档集根据工作区实现维护。[qiankun 实施与验证](qiankun-audit.md)记录 2026-10-23 的迁移验收快照，区分本地候选和正式上线；后续门店组织模型的现行契约以本文导航中的授权与组织目录说明为准，历史候选不代表后续源码已重新发布。

## 开发与应用接入

| 文档 | 内容 |
| --- | --- |
| [架构](architecture.md) | 仓库边界、宿主加载、身份与业务数据流 |
| [开发指南](development.md) | 环境、启动、代理、SDK 和日常检查 |
| [微应用接入](micro-application-onboarding.md) | qiankun 产物、登记、URL/版本入口、访问范围 |
| [按环境配置 URL](environment-url-entries.md) | 可变入口、revision 和版本覆盖规则 |
| [静态代理](micro-url-proxy.md) | `/micro-proxy/` 的资源解析和转发边界 |
| [独立业务交付](independent-business-delivery.md) | 平台登记、业务后端与前端的部署分工 |
| [工程验证](engineering-baseline.md) | CI、数据库和浏览器测试的实际门禁 |

## 身份、权限与通知

| 文档 | 内容 |
| --- | --- |
| [应用授权契约](application-authorization-contract.md) | 应用 token、combined/legacy 服务端授权与数据范围 |
| [应用组织目录](application-organization-directory.md) | STORE 门店身份、总部范围与新鲜组织资料 |
| [OAuth](oauth-issuance.md) | 授权码、PKCE、设备码、内省和撤销 |
| [数据授权管理](data-grant-management.md) | 服务端策略、差分预览/提交、并发与审计 |
| [数据授权操作](data-grant-user-guide.md) | 管理界面流程和范围解释 |
| [实时通知](realtime-protocol.md) | HTTP 持久化、Socket.IO 提示和补拉 |
| [管理审计](management-audit.md) | 事务、变更摘要、查询与导出 |
| [登录保护](login-protection.md) | 限流、密码校验预算和配置 |
| [成员生命周期 API](identity-sso/phase-01-api.md) | Phase 1 已实现的成员／准入扩展、显式管理接口、旧员工写入兼容与默认关闭的启用门禁 |
| [成员写路径审查](identity-sso/phase-01-membership-path-audit.md) | 创建／批量／联系人／seed、触发器与二元读取清单，锁顺序及后续阶段边界 |
| [身份 Phase 1 验收](identity-sso/validation/phase-01.md) | 编译、隔离数据库、锁等待、SDK／消费者及完整回归的真实结果与限制 |
| [凭据身份绑定](identity-sso/phase-02-credential-binding.md) | Phase 2 session／refresh／OAuth／app 的具体成员、账号、来源与策略证明，旧凭据拒绝及撤销边界 |
| [准入策略管理](identity-sso/phase-02-policy-api.md) | 应用／客户端允许集合、策略版本、并发保存和 CLI 合同 |
| [身份 Phase 2 验收](identity-sso/validation/phase-02.md) | 185 普通、151 数据库、58 CLI 与固定 SDK 合同的实际结果和分组重跑 |
| [身份 Phase 3 验收](identity-sso/validation/phase-03.md) | 精确员工门禁、角色／目录／bulk、160 数据库逐项通过及两业务仓候选合同 |
| [通知成员绑定](identity-sso/phase-04-notification-binding.md) | 具体收件成员／代际、HTTP 与偏好、Notification 1 固定载荷和内部 authority |
| [身份 Phase 4 验收](identity-sso/validation/phase-04.md) | 真实平台／通知服务联调、收件隔离、锁等待撤权、119 UUID 与数据库角色权限 |
| [控制台成员证明](identity-sso/phase-05-console-identity.md) | 服务器 proof、具体成员选择、能力开关、会话和缓存 namespace |
| [身份 Phase 5 验收](identity-sso/validation/phase-05.md) | 管理 UI／准入编辑、456 Node、真实 PG、浏览器逐组、构建与 StrictMode 修复 |
| [客户最小资料](identity-sso/phase-06-customer-profile.md) | 独立 context／profile／applications、精确客户用途及 client／app／成员实时交集 |
| [独立收缩与环境启用](identity-sso/phase-06-rollout.md) | 不改默认迁移链的收缩工具、真实资产／门禁 manifest 和二进制绑定 receipt |
| [身份 Phase 6 验收](identity-sso/validation/phase-06.md) | 193 普通、178 main ignored 逐项分组、3 rollout、真实开发库收缩和 30 HTTP，保留超时恢复 |
| [同进程账号认证边界](identity-sso/phase-07-authentication-boundary.md) | Phase 7 port 与同库事务适配、账号字段／会话／混合 OAuth owner 及未来拆分边界 |
| [身份 Phase 7 验收](identity-sso/validation/phase-07.md) | 194 普通、178 既有＋4 新 PG／HTTP、启动单项及静态／文档门禁；保留失败修复记录 |
| [SSO 全局账号会话](identity-sso/phase-08-global-session.md) | Phase 8 独立账号认证、Cookie／CSRF、全局撤销与 legacy 改密提交前再验；已验收 |
| [身份 Phase 8 验证](identity-sso/validation/phase-08.md) | 194 普通、21 新增、182 既有、3 rollout、16 配置、真实 Edge15 PASS 与原始失败记录 |
| [标准 OIDC 与独立密钥](identity-sso/phase-09-oidc-provider.md) | code＋S256／ID Token／发现／JWKS、客户端用途与策略、固定 issuer 及 RS256 密钥生命周期 |
| [身份 Phase 9 验收](identity-sso/validation/phase-09.md) | 194 普通、9 新 PG、203 既有、3 rollout、37 配置、两轮各 117 标准 RP／Edge 与 6 图 PASS |
| [平台 BFF 与兼容入口](identity-sso/phase-10-platform-bff.md) | 服务端 OIDC／平台 refresh、精确员工 context、内存短 Bearer、显式恢复与独立注销 |
| [身份 Phase 10 验收](identity-sso/validation/phase-10.md) | 195 普通、14 BFF PG、212 既有、3 rollout、66 配置、505 Node、实际 Edge 98 条检查及 9 图 PASS |
| [独立客户门户](identity-sso/phase-11-customer-portal.md) | CUSTOMER RP／BFF、精确客户成员选择、独立会话与本人资料 |
| [身份 Phase 11 验收](identity-sso/validation/phase-11.md) | 独立门户、0019 客户资源 port、平台 197 普通／7 新 PG／226 既有、门户 12 普通／5 PG、36 Node、真实双 RP 196 条检查与 8 图 PASS |
| [独立认证部署](identity-sso/phase-12-identity-deployment.md) | 独立 identity、网络 port、共享库迁移 owner 与最小运行角色 |
| [身份 Phase 12 验收](identity-sso/validation/phase-12.md) | 独立拓扑、数据库权限、协议／浏览器和共享库恢复的实际证据及限制 |
| [灰度与身份感知恢复](identity-sso/phase-13-rollout-recovery.md) | Phase 13 当前执行范围：灰度回退、持续负载、双库恢复和撤销 |
| [活动消费者与双身份审查](identity-sso/phase13-consumer-audit.md) | 八仓消费者、仍支持的 legacy 路径、逐命中 SQL 分类及当前合同验证 |
| [身份 Phase 13 验收](identity-sso/validation/phase-13.md) | 当前 IN_PROGRESS；真实运行、失败修复、容量／恢复／通知与封账状态 |
| [身份新库 UUID v7 基线](identity-sso/validation/fresh-v7-baseline.md) | 空库正式迁移、真实 seed、引用／约束及活动消费者验证，不含旧库搬迁 |

## 部署与运维

| 文档 | 内容 |
| --- | --- |
| [部署](deployment.md) | 平台 Compose、独立网络、网关和配置 |
| [接入已有 Nginx](existing-nginx.md) | 宿主机 Nginx 的路由、TLS、CSP 和业务代理 |
| [ZIP 分发](zip-distribution-runbook.md) | 制品存储、代理、节点分发和激活 |
| [升级与保留](upgrade-and-retention.md) | 数据库迁移、版本指针、回滚和清理 |
| [网关工具](../nginx/README.md) | 配置生成、来源策略和静态资源路径 |
| [管理 CLI](../tools/platform-admin/README.md) | bootstrap、release、grants、artifact 和 retention |
| [镜像部署代理](../tools/deployment-agent/README.md) | 镜像操作边界和节点部署 |
| [制品代理](../tools/artifact-agent/README.md) | 固定版本静态资源落盘 |
| [运维工具](../ops/README.md) | 监控与恢复能力及当前限制 |

## 规划与历史

[员工、客户身份与统一认证中心方案](identity-sso/README.md)及[实施台账](identity-sso/task-ledger.md)按阶段推进。Phase 1～13 的成员生命周期、完整凭据证明、准入管理、员工授权／目录／双向角色、通知／管理 UI 隔离、客户最小能力、独立收缩、同进程认证边界、全局 SSO 会话、标准 OIDC、平台 BFF、独立客户门户与独立认证部署已通过验收；Phase 13 的灰度、分轮容量、身份感知恢复和活动消费者验收已完成，详见[最终验收](identity-sso/validation/phase-13.md)；已通过项按用户要求复用，仅补失败项，各轮结果及源码版本单独记录。正式状态和实际验证以台账及阶段证据为准。成员写入、SSO／OIDC／BFF 默认关闭，任务自有环境的显式启用及 receipt 只证明相应测试范围，生产尚未开放。

[UUID v7 审查与保数据重建](identity-sso/uuid-v7-audit.md)、[只读预览库清点](identity-sso/uuid-v7-inventory.json)及[专项台账](identity-sso/uuid-v7-task-ledger.md)管理独立预览三库的存量重编号。其状态、转换及切流证据仅由专项台账维护；身份新功能使用空库，从自身 v7 基线验收推进，不以历史搬迁为前置，也不接管预览库或冒记专项完成。

[支付设计](pay/README.md)和[支付实施台账](pay/task-ledger.md)是未实施规划，不是现有支付服务的操作手册。SDK 的公共 API 与生成发布方式由独立 SDK 仓库维护；本平台说明只描述消费和集成。

已删除旧迁移分析、阶段实施记录、旧日期审计、拆仓验收和重复项目总览。迁移验收结论记录在 `qiankun-audit.md`；现行行为以对应功能文档和源码为准。数据库旧迁移、固定 SDK 1 发布归档和历史制品仍可作为历史证据，不能据其目录存在推断当前运行方式。
