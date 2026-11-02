# Phase 25 当前组件逐断言缺口审计

日期：2026-10-29。本记录是主矩阵回填前的只读断言核对，不改变111项准备矩阵的状态。身份执行报告为[当前142项组件](phase-25-identity-components.json)，其测试到源码匹配见[执行映射](phase-25-component-execution-map.json)。P/S/U/X/B/R/A含义沿用[独立层级审查](phase-25-evidence-layer-audit.md)。

## 已有当前P断言，不应重复建设新fixture

| 子例 | 已执行测试及直接断言 | 使用边界 |
| --- | --- | --- |
| C01-S01/S02 | `phase04_unified_client_api_enforces_policy_paging_and_credential_lifecycle`：[auth_admin_tests.rs](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9)经统一私有handler登记FIRST_PARTY、THIRD_PARTY、同应用第二client，并检验返回归属与分页 | 当前真实PG的小schema；不代替公开网关唯一入口X |
| C04-S01 | `phase06_pairwise_subjects_are_sector_scoped_stable_and_retired_with_accounts`：[auth_subjects.rs](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L206)同应用同环境两个不同client得到相同subject | SMALL_SCHEMA_PG持久映射；不是两个真实客户端浏览器登录 |
| C04-S02 | 同测试直接比较同应用不同environment、不同partner application的subject不同 | 测试中批准投影是明确owner SQL fixture，不是实际合作方发布 |
| C04-S03 | 同测试[更名与签名轮换](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L275)后sub保持、profile更新 | 当前PG正向断言，不需重新实现更名fixture |
| C04-S04 | 同测试[RETIRE命令](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L326)封存已发3个link、旧账号resolve拒绝，新账号sub不继承 | 真实账号命令及PG持久事实；不单独代替实际恢复R |
| C07-S01的P | `phase08_authorization_code_pkce_consent_pairwise_id_token_and_userinfo_use_one_registry`：[PUBLIC sibling](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1642)仅client_id兑换成功，credentials count=0；管理测试[PUBLIC轮换拒绝](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L434) | 前端不显示/发送secret是独立U事实；PUBLIC不是当前额外真实TLS fixture |
| C07-S02 | 同phase08测试[错误CONFIDENTIAL secret](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1498)返回401；phase11机器与resource credentials有独立负例 | 准确列出USER/MACHINE/内省各自断言，不用一个401覆盖所有用途 |
| C07-S03/S04的P | 管理测试[凭据重叠和紧急撤销](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L400)校验一个current加一个temporary、90天截止、撤销两个凭据并推进policyVersion；phase11实际token/内省按credential version复验 | 重启后拒绝属于独立R，须当前密钥/恢复真实运行证据 |
| C06-S04/S05/S08 | `phase10_refresh_rotation_reuse_concurrency_limits_and_offline_source_are_enforced`：[期限](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2890)校验离线7天/24小时、自然SSO到期不撤离线；将准确family idle/absolute截止置为过去后refresh返回invalid_grant；GLOBAL logout/账号停用另行拒绝 | 真实PG受控截止，不冒充长期真实等待或主机时钟修改 |
| C10-S01/S03/S04的版本事实 | `phase07_revoke_delete_reauthorize_and_logout_keep_generation_fences`：[真实DELETE/撤销/重授权](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L318)检查active删除409、原grant/refresh/access失效、display删除不改墓碑、重授权后旧expectedVersion删除409 | 旧删除与新授权按明确先后执行；不能表述为额外启动了OS并发进程 |

上述测试均在当前142项报告中有精确`OK`记录及源码摘要绑定。没有因准备矩阵写了EXTEND就重复创建数据或扩大真实环境。

## 真正仍需新证据或重跑的断言

| 子例 | 直接核对结果 | 当前闭合方式 |
| --- | --- | --- |
| C03-S05真实人员auth_time | phase08有`max_age=0`不复用旧SSO的负例，但未直接比较签名ID Token的auth_time与实际primary认证事实；MACHINE的NULL不构成人员正例 | 新[协议组件](phase-25-protocol-components.json)验证SDK与auth_time窗口；新protocol真实TLS runner必须记录本轮实际primary请求前后窗口并比较实际签名声明 |
| C03-S10错误ID Token aud | 原RP back-channel错aud测试属于logout_token；原Rust手工ID Token verifier和正确aud正例不能推断实际独立标准验证器拒绝 | 已补[当前独立RP4项签名负例/对照](phase-25-rp-audience-components.json)：同fixture签名密钥，仅aud或iss不同，经实际RP callback400且不建会话；正确字段对照303建会话。明确替换点为SDK测试port，不冒充实际identity响应 |
| C05-S02/S03 | 当前142源码只见S256正向/变换及错误verifier；未见实际authorize端点plain、缺失、短/长/非法challenge负例 | 新protocol浏览器组逐个发实际请求，必须精确400且不产生callback；尚未执行时保持PENDING |
| C05-S06/C06-S02 | canonical `phase22_tests`有真实PG锁屏障及独立AppState，但不等于两个OS进程 | 架构只要求并发。若主矩阵保留额外OS进程文字，需透明标明这项额外要求；新protocol提供两独立child的code竞争，refresh仍由当前PG证据或另行真实组证明 |
| C11-S02–S07等U | 现有[context-selection-ui.mjs](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)确实用实际Chromium DOM和明确HTTP fixtures验证单/多租户、自动继续、搜索不自动选择、XSS与移动尺寸；frontend886不包含identity该脚本，identity142也是Rust | 已新增[本轮13项DOM结果](phase-25-identity-dom.json)：11输入与apps-r2前后逐字节匹配，13/13通过。临时目录清理被自动审批启动前拒绝，17文件382558逻辑字节与1依赖junction仍保留；U通过不推进完整清理A |
| C17-S07时钟偏移 | 存在先前准备盘点漏掉的有效生产validator测试：[bff_tests.rs](../../../../src/bff_tests.rs#L87)使用RS256签名调用backchannel validator，直接断言future iat+6秒拒绝、+5秒接受及过期边界 | 等当前平台组件执行报告绑定精确case后可记back-channel领域的P证据；这不声称ID Token验证器采用同一容差 |

## 主矩阵回填约束

`sourceRef`匹配到测试执行只证明该测试运行，不能替代上述逐断言核对。142/432/886等总数均不能直接换算为111项通过。P/S/U事实可先独立存证，实际X/B/R/A结果到齐后再综合判定。保留准备原版、失败运行与所有限制，缺少实际结果的项继续PENDING。

本审计没有修改生产代码、启动原生服务、操作数据库或Docker。新协议组件的本地HTTPS fixture、浏览器HTTP helper和其清理已在各自组件报告单独记录；它们不作为当前完整环境的执行结论。
