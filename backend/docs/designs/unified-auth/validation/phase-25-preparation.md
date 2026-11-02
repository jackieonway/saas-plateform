# Phase 25 验收准备（尚未执行）

本文件只整理依赖、当前可执行入口和资源边界。Phase 24 退役完成后才冻结发布候选并执行本阶段；下表没有一项表示最终版本已经通过。历史阶段报告用于定位测试，不能代替候选版本的新执行结果。

## 固定版本与证据

最终候选包含身份服务、平台宿主/后端、客户门户、门店、工单、通知、SDK 和独立标准 OIDC 示例。每个独立仓库保留已有未提交工作；记录实际生产文件、迁移、角色 SQL、配置模板、测试脚本、前端产物、锁文件和二进制的 SHA-256。不能只给身份服务重建后的摘要并声称所有消费者重建。

现有 `tests/unified-auth-https/snapshot-build.py` 仅闭合身份、平台和可选门户输入。最终使用前还须纳入门店、工单、通知、标准 RP 及其必要依赖/静态产物；不能把这些服务的旧 `.local-run/bin` 当作当前候选。各服务须使用与其 migrator 相同的 canonical `0001` 和角色 SQL，不改写 checksum，不重用旧阶段库。

正式报告逐项保存：候选摘要、命令、执行时间、通过/失败数量、证据层级、明确的 fixture 边界、失败尝试和修正、清理结果。旧报告只作溯源，引用的新报告必须由本次候选重新生成。发生生产文件修改后按受影响范围重建/重跑并更新组合来源证明。

## 方案第 18 节的执行映射

| 验收类别 | 当前可复用入口及最终需要补齐的证据 |
| --- | --- |
| 统一登记 | canonical schema/真实 LOGIN readiness、统一客户端 API/浏览器测试；Phase 24 退役扫描证明无旧登记/签发入口 |
| 普通外部登录 | Phase 21 的独立标准库 RP；使用无成员账号、公开 discovery/协议、真实 TLS，运行进程不配置身份或平台数据库 |
| OIDC 一致性 | `util::audit_tests::auth_protocol` 和 Phase 21 标准库；核对 iss/aud/sub/nonce/auth_time、UserInfo，一并记录错误 issuer/nonce/client 的拒绝 |
| subject 隔离 | `phase06_pairwise_subjects_are_sector_scoped_stable_and_retired_with_accounts`；从当前空库重跑稳定、跨 sector、重命名和注销 tombstone |
| code/PKCE | `phase08_authorization_code_pkce_consent_pairwise_id_token_and_userinfo_use_one_registry`，标准库 RP 的 state/nonce/S256；包括并发兑换、重放和回调拒绝 |
| refresh | `phase10_refresh_rotation_reuse_concurrency_limits_and_offline_source_are_enforced` 及 consumed-token 两项回归；实际 BFF/标准库轮换、scope 收缩与离线边界 |
| 客户端认证 | canonical client-admin/credential-age/MACHINE 套件；公开与机密客户端正反向、轮换/紧急撤销；默认第三方政策不得被测试配置扩大 |
| 资源隔离 | identity resource proof、平台 `identity_deployment_tests::phase15*`、门店/工单真实库和实际 HTTPS；ID Token、错误 audience/scope、MACHINE/USER/员工/客户边界 |
| 同意 | canonical consent PG 与授权管理浏览器；新 scope、group version、跨组、显式共享；实际浏览器首次确认与持久复用 |
| 撤销与删除 | `phase07_revoke_delete_reauthorize_and_logout_keep_generation_fences`；实际授权页撤销/删除，旧 token 不复活，陈旧删除不能影响重新授权 |
| 上下文 | canonical Phase 12 PG、真实员工 `browser.mjs` 和客户 `customer-browser.mjs`；零/一/多候选、搜索、员工客户隔离与 ACCOUNT_ONLY 无业务接口 |
| 业务权限 | canonical proof 与独立业务库矩阵；通过当前实际服务验证组织/成员/准入缩减，外部 scope 不能突破 permission/dataGrant |
| 跨库与标签页 | `customer-browser.mjs` 原结果恢复、host notification browser、Phase 18 新竞态回归；最终需在当前实际身份/平台/业务/通知 HTTPS 环境跑完整切换链，明确不以 HTTP responder 替代 |
| 管理安全 | `phase19-browser.mjs`、邀请/审核 PG、平台管理幂等恢复和权限浏览器；真实 TOTP、申请人不可自批、CSRF、强制审计失败、版本/来源冲突 |
| 登出 | `phase20-browser.mjs`、实际 BFF receiver、Phase 21 RP；CURRENT_APP/GLOBAL/consent 撤销、jti/签名/aud、任务重试/永久失败与短会话合同 |
| 浏览器 | 当前客户端/授权管理/邀请/宿主桌面和移动浏览器矩阵；键盘焦点、弹窗滚动、Cookie 限制、XSS/开放重定向拒绝；保留无秘密截图 |
| 故障 | Phase 22 当前运行/压力工具与故障矩阵；池/队列/超时/身份/平台/数据库失联、明确拒绝语义及受控时钟边界 |
| 运维 | Phase 22 指标告警/实测容量与 Phase 23 当前恢复演练；当前日志按运行秘密和实际投递材料内存比对；候选资源完整清理 |

每个大类还须展开为方案中所有逗号分隔的具体场景，再链接本次新结果；不能只给一个大类 PASS 而漏掉子项。单元、HTTP fixture、真实 PostgreSQL、跨服务 HTTPS、浏览器、外部标准库分别标注。任何一种证据都不能独自替代完整链路。

## 环境和清理边界

优先复用任务专属原生 PostgreSQL 和共享 Cargo cache；当前任务端口 `55494`，集群标识 `7701548153213487841`，与既有本机 `5432` 数据库分离。Phase 21/22/23 的新进程和数据库需有不同归属标识及协调的端口 lease。最终候选开始前关闭前置阶段的专属运行实例，以免把旧进程当候选。

本阶段计划创建/复用 Docker 容器为 0。若改用 Docker，先按工作区 AGENTS 检查实际 context、记录基线和项目预算，不得隐式扩大。所有原生临时数据库、角色、namespace、gateway/proxy、collector、测试应用进程、私钥、投递材料、日志和备份均由 finally/lease 清理；结束后独立核查。共享 PostgreSQL/cache 是阶段间保留资源，整个目标结束前仍须清理，不可在此遗漏。正式源码和报告保留。

当前各仓库测试、安装 lifecycle、CI、Compose 与 Docker CLI 的静态资源检查见 [Phase 25 资源预查](phase-25-resource-preflight.md)。普通宿主镜像策略测试使用 fakeDocker；真实 history 导出、deployment-agent 执行和门店本地网关等入口会创建容器，不能不经预算直接纳入零容器矩阵。安装依赖使用锁文件及 `--ignore-scripts`，再按实际需要执行明确的构建命令。

## 真实发布目标

用户已确认本机以及 `auth.saas.test`、`console.saas.test`。只读发现的既有数据库与 owner 见 [本机目标盘点](local-target-inventory.json)；它不构成重置或切换授权。Windows 既有 `127.0.0.1:443` PID 5044、WSL 既有通知 PID 24971，以及现有业务数据库继续保留。

Phase 25 应先形成具体可审阅候选：建议采用独立新库、明确新 owner/runtime 角色和服务/端口、准确 issuer/CA/回调、受保护秘密注入、备份/整体回退、停止旧签发权威的切换顺序和冒烟清单。准备完成后才向用户请求针对该清单的最终授权，并核实真实审核/发布/值班责任、保留政策和容量目标；不得把 fixture reviewer、临时接收方或实测单机数字当成真实运营事实。
