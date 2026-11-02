# Phase 20 客户端登记与审核组件验证

日期：2026-10-28。组件状态：COMPLETED。依据[登出与投递合同](../phase-20-logout-delivery-contract.md)；本报告只覆盖客户端登记、独立审核清单与管理 UI，整体 worker 和跨服务 HTTPS 由阶段总报告验收。

## 源码边界与安全约束

- 身份：[客户端登记](../../../../../../saas-identity/src/auth_admin.rs)、[审核投影](../../../../../../saas-identity/src/auth_review.rs)；平台：[审核策略](../../../../src/auth_review/policy.rs)、[私有管理 facade](../../../../src/oidc/remote_admin.rs)；前端：[客户端编辑与详情](../../../../../frontend/src/pages/system/oauth/UnifiedClientsPage.tsx)、[合作方申请](../../../../../frontend/src/pages/system/oauth/PartnerIntakePanel.tsx)及对应 API DTO。迁移、owner ACL/readiness、BFF 接收器、邀请领取和 worker 不属于本组件。
- 客户端输出始终包含 `logoutMode`、`backchannelLogoutUri`、`backchannelTargetId`；省略输入默认短会话与两个空值。新客户端先创建为 `SHORT_SESSION_ONLINE_CHECK`，已有机密 USER 客户端才可绑定 `BACK_CHANNEL`。
- runtime 只读 owner 目标元数据，再调用窄 `identity_delivery_target_active` SECDEF fence，按原 client、精确 URL、目标代际、状态和期限持锁复验。runtime 不对 owner 表直接 `SELECT FOR SHARE`，没有目标 CRUD；任意 UUID 不能自证核验。
- 第三方 `BACK_CHANNEL` 必须与当前应用/环境独立审核的精确 URI 一致。申请 URI 只允许当前审核域名内、无 query/fragment/userinfo 的 HTTPS 地址；申请不携带或创建目标 ID。前后端均校验，身份服务为最终权威。
- 纯 `DISABLED`/`RETIRED` 必须保持三个字段及其他授权配置完全一致，不能夹带目标、URI 或 scope 修改。过期目标不阻止纯停用、退役或凭据权限收缩；重新启用和其他编辑重新验证当前事实。表单切换回短会话会清空 URI/引用，纯停用请求则保留原值。

## 验证结果

| 检查 | 结果与证据层级 |
| --- | --- |
| 身份 `auth_admin::logout_registration_tests` 定向测试，包含 ignored PG 用例 | 2/2 PASS：1 个纯输入校验；1 个真实 canonical PostgreSQL 用例。后者验证任意 UUID 拒绝、原 client/URL 核验、跨 client 拒绝、过期目标纯停用，以及 canonical trigger 的实际 UPDATE。 |
| 平台 `oidc::remote_admin_tests` | 19/19 PASS；严格 DTO、字段组合、创建默认值及纯停用隐藏修改拒绝。 |
| 前端 `npm test` | 450/450 PASS；包含 back-channel 客户端字段与申请 manifest 的合法/非法组合。 |
| 真实 Chromium、生产 React/Vite 页面与独立 HTTP fixture | 客户端 [11/11](phase-20-registration-ui/clients/unified-clients-browser.json)；合作方接入 [17/17](phase-20-registration-ui/intake/phase19-partner-intake-browser.json)。覆盖 390px 编辑、申请域名拒绝、请求字段和退休保留原绑定；既有冲突、未知操作恢复、细权限和敏感字段拒绝继续通过。 |
| 编译与静态检查 | identity/backend `cargo check --locked`、backend 测试编译、TypeScript、变更文件 ESLint、`npm run build`、所属 Rust 文件 `rustfmt --edition 2021 --check` 和 `git diff --check` 全部通过。 |

新增移动端截图已视觉检查：客户端[登记及 owner 引用](phase-20-registration-ui/clients/phase20-backchannel-mobile.png)、合作方[待独立审核地址](phase-20-registration-ui/intake/phase20-review-backchannel-mobile.png)。页面无横向溢出，保存/提交和取消动作可达。完整浏览器截图与 JSON 按原目录结构保留在上述两个证据目录，复制前后 SHA-256 一致。

HTTP fixture 不执行真实 SSO/TOTP、owner 域名核验、实际生产批准或真实 worker HTTPS 投递。PG 用例使用真实 canonical 基线及明确 owner 测试事实，证明登记边界和 SQL 触发器，不等于真实部署责任人完成核验。transport 的实际 TLS、投递租约/重试和 BFF 撤销由其他组件及阶段总报告另列。

## 资源与清理

本组件只使用根任务指定的原生 PostgreSQL `127.0.0.1:55494`，集群标识 `7701548153213487841`；所属 `identity_phase20_core_01a181c14be8729c97ecaf1c70c127b2` 已由 isolated helper 关闭连接并删除，所属临时数据库残留为 0。浏览器两个 runner 均 exit 0，并在 `finally` 关闭 context、Chromium 和 Vite；本组件活动 fixture 残留为 0。正式 JSON/截图归档后，原两份 `frontend/target/phase20-*backchannel-browser` 临时输出定向删除。

创建/复用 Docker 容器均为 0，未创建卷、网络、镜像或专用构建缓存。共享 PostgreSQL 和根任务构建缓存仍供主流程使用；本组件不停止或清除它们。无本组件 Docker 磁盘占用变化，也不声称 Windows/WSL 虚拟磁盘已缩小。
