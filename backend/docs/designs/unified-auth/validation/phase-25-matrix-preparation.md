# Phase 25 当前版本验收矩阵

准备时间：`2026-10-29T10:13:26.010317+00:00`。状态：**PENDING**。本文是验收映射和结果槽位，不宣称 Phase 25 已完成。

按[架构第 18 节](../../unified-oauth-oidc-sso-architecture.md#18-必须通过的验收矩阵)拆为 **18 类、111 个独立子例**。全部 Phase 25 运行结果尚未执行。实际环境、数据库、进程、秘密与清理由单一环境 owner 管理；本矩阵代理只读源码并生成本文和[结构化 JSON](phase-25-matrix.json)，Docker 创建／复用、DB 操作和运行进程启动均为 0。

## 证据使用规则

- 每个子例独立保留结果、时间和 actualEvidence；不能只把十八类各写一个 PASS。JSON 空 evidence 数组不表示通过。
- CURRENT_INPUT_EVIDENCE 仅表示本次逐文件核对旧组件报告输入；不是当前完整版本 HTTPS／浏览器／部署结果。消费者早期执行缺预运行清单的限制继续保留。
- 身份 Phase04/06/07/08/09/10/11/12 多为小 SCHEMA 和 mock intent/SelectionPort；函数名含 canonical 也不提升层级。canonical PG、真实 LOGIN ACL 和实际跨服务 HTTPS 分开。
- Phase22 independent AppState 并发不是两个服务进程；独立 RP 的受控 TLS issuer 测试不是实际 identity 互通。候选 runner 须在当前版本执行。
- 源码定位证明候选入口存在；EXTEND/GAP 说明仍需明确断言，不能从大测试名称推断已覆盖。运行前修复旧集群与输出绑定，保留精确归属校验。
- PASS 必须记录当前源码／binary／config 摘要、命令、selector、脱敏观测、限制与清理；禁止存秘密原值及原始私密日志。

## 当前组件输入核对

| 证据 | 当前核对 | 使用边界 |
| --- | --- | --- |
| [E24IDENTITY](phase-24-identity-final.json) | 100/100 输入匹配；CURRENT_INPUT_EVIDENCE | 141 lib + 1 main: 86 ordinary, 24 small-schema PG, 31 canonical PG. Exact evidence layers retained. |
| [E24PLATFORM](phase-24-platform-final.json) | 382/390 输入匹配；PRIOR_COMPONENT_EVIDENCE_REQUIRES_INPUT_REVIEW | 432 unique cases through 382 unchanged input-equivalent and 50 current affected cases. Historical 424 PASS / 7 FAIL retained, not rewritten as one 432 PASS run. |
| [E24CONSUMERS](phase-24-consumer-components.json) | 1078/1086 输入匹配；PRIOR_COMPONENT_EVIDENCE_REQUIRES_INPUT_REVIEW | Partial explicit source manifest; frontend/stores/work-orders/RP/CLI/Nginx component scopes retain original limitations. Not all early runs had full pre-run manifests. |
| [E24EMPTY](phase-24-empty-database-final.json) | DEPENDENCY_EVIDENCE_ONLY | 103 real empty-PG canonical migrate/provision/verify checks and actual LOGIN-role ACL; no Phase25 service/browser credit. |
| [E24RECOVERY](phase-24-recovery-retirement.json) | DEPENDENCY_EVIDENCE_ONLY | Old generic restore fails before I/O; component gate, not actual restore. |
| [E24INPUTAUDIT](phase-24-platform-final-input-audit.json) | DEPENDENCY_EVIDENCE_ONLY | Historical independent input audit; this matrix independently checks relevant manifests again. |

报告 SHA、输入字段、差异和逐源码 SHA 在 JSON。历史平台 `424 PASS / 7 FAIL` 保留 FAIL；432 唯一用例是 382 输入等价加 50 当前受影响组执行，不是一次 432 PASS。

## 十八类逐项结果

候选运行只表示当前源码入口，精确 selector 与 SHA 在索引及 JSON。全部 PENDING，实际执行后逐项附证据，不能批量置 PASS。

### C01 统一登记

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C01-S01` 自有与第三方同表 | 两种ownership的客户端由同一auth_clients登记、读取与授权处理；无隐藏注册簿 | [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [I19](../../../../../../saas-identity/src/auth_review/tests.rs#L407), [PBASE](../../../../src/identity_deployment_tests.rs#L317)<br>运行：[H19REGISTRY](../../../../tests/unified-auth-https/phase19-browser.mjs#L369) | **PENDING**；补当前新安装记录及同表只读证据 |
| `P25-C01-S02` 一应用多客户端 | 同一application的两个独立client可用，凭据、回调和授权来源仍按client隔离 | [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [I07GROUP](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L526)<br>运行：[H19REGISTRY](../../../../tests/unified-auth-https/phase19-browser.mjs#L369) | **PENDING**；EXTEND:明确两客户端成功与交叉回调拒绝HTTPS子例 |
| `P25-C01-S03` TEST与PRODUCTION隔离 | 另一环境的admission、客户端和scope不能被复用 | [I19](../../../../../../saas-identity/src/auth_review/tests.rs#L407), [PBASE](../../../../src/identity_deployment_tests.rs#L317)<br>运行：[H19PUBLISH](../../../../tests/unified-auth-https/phase19-browser.mjs#L423) | **PENDING**；任务production绑定不推断真实发布账号 |
| `P25-C01-S04` 单一公开权威 | 旧/oidc签发及旧registry全部404，统一/oauth发现与签发为唯一公开入口 | [IRETIRED](../../../../../../saas-identity/src/oidc/protocol.rs#L255), [PRETIRED](../../../../src/protocol_retirement_tests.rs#L10)<br>运行：[H21DISCOVERY](../../../../tests/unified-auth-https/phase21-browser.mjs#L140) | **PENDING**；实际网关重验旧路由、私有端点拒绝和唯一issuer |

### C02 普通外部登录

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C02-S01` 无业务成员账号 | 零业务membership账号使用ACCOUNT_ONLY成功；不强迫选租户/组织 | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [I12](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:断言零membership及没有业务context网络调用 |
| `P25-C02-S02` 仅标准OIDC接入 | 独立RP仅使用discovery/JWKS/token/userinfo，openid-client与jose完成登录 | [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H21DISCOVERY](../../../../tests/unified-auth-https/phase21-browser.mjs#L140), [H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；记录实际标准依赖版本和无内部API/DB/SDK调用证据 |

### C03 OIDC一致性

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C03-S01` 正确issuer | 签名ID Token iss等于准确HTTPS issuer且discovery与回调iss一致 | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；保存脱敏布尔断言与版本，不能保存原token |
| `P25-C03-S02` 正确audience | ID Token aud只指向当前公开client；不能采用资源audience | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:显式声明值比对 |
| `P25-C03-S03` 不透明subject | ID Token sub非内部UUID且绑定当前app/environment的pairwise映射 | [I06](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L126), [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；只报告不透明/相等布尔值，不导出可关联sub |
| `P25-C03-S04` 正确nonce | ID Token nonce绑定原浏览器授权transaction | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；记录标准SDK实际校验；不导出nonce |
| `P25-C03-S05` 正确auth_time | 人员auth_time与实际primary/MFA认证事实一致且符合max_age/step-up约束 | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [PMFA](../../../../src/auth_review/tests.rs#L481)<br>运行：[H19MFA](../../../../tests/unified-auth-https/phase19-browser.mjs#L368), [H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；GAP:人员auth_time精确比较及旧时间拒绝；MACHINE NULL不替代人员正例 |
| `P25-C03-S06` UserInfo subject一致 | 标准SDK核验ID Token与UserInfo sub严格相同 | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [ICLAIMS](../../../../../../saas-identity/src/oidc/subjects.rs#L296), [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；当前实际identity与RP通信重跑 |
| `P25-C03-S07` 错误client拒绝 | 另一client不能兑换code、验证ID Token或取UserInfo原身份 | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:不同client负例及无会话提交 |
| `P25-C03-S08` 错误issuer拒绝 | 错误回调iss及错误签名issuer均由RP拒绝 | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [RPFAIL](../../../../../../saas-examples/oidc-rp/tests/deployment.test.mjs#L45)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:标准SDK调用实际错误issuer响应，不仅helper存在 |
| `P25-C03-S09` 错误nonce拒绝 | 改动nonce的有效签名响应被拒绝且不建立会话 | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:实际SDK负例 |
| `P25-C03-S10` 错误ID Token audience拒绝 | 有效签名但错误aud的ID Token被独立标准RP拒绝 | [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；GAP:组件盘点未见直接错audience客户端验证调用，须显式新增 |

### C04 subject隔离

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C04-S01` 同sector稳定 | 同应用同环境且经批准sector内同一账号跨客户端稳定 | [I06](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L126)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:同sector双client实际登录比对 |
| `P25-C04-S02` 不同合作方隔离 | 不同app/partner或environment不能获得相同可关联sub | [I06](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L126)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:两独立partner及TEST/PRODUCTION比对 |
| `P25-C04-S03` 更名稳定 | 更改displayName/username后原subject映射保持 | [I06](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L126)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:当前新账号经受控管理更名再登录 |
| `P25-C04-S04` 注销不复用 | 账号退休后旧subject封存，新账号不继承原subject | [I06](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L126), [RECOVERY](../../../../ops/identity-recovery/test_reconcile.py#L136)<br>运行：[H23RESTORE](../../../../tests/unified-auth-https/phase23-browser.mjs#L591) | **PENDING**；EXTEND:任务新账号退休及新建比对，不操作已有账号 |

### C05 code与PKCE

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C05-S01` 错误callback拒绝 | 未精确登记/交叉client/带额外参数callback不接收成功code | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [ICALLBACK](../../../../../../saas-identity/src/oidc/protocol.rs#L362), [I12CALLBACK](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2546)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:实际authorize与token两处redirect_uri负例 |
| `P25-C05-S02` plain拒绝 | code_challenge_method=plain被拒绝，不产生可兑换code | [IPKCE](../../../../../../saas-identity/src/oidc/oauth.rs#L4066)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；GAP:现有S256变换单测不等于plain负例，需显式请求 |
| `P25-C05-S03` 非法challenge拒绝 | 过短/过长/非法字符或缺少S256 challenge被拒绝 | [IPKCE](../../../../../../saas-identity/src/oidc/oauth.rs#L4066), [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；GAP:实际challenge语法负例 |
| `P25-C05-S04` 错误verifier拒绝 | 格式正确但不匹配原challenge的verifier无法兑换 | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:实际token端点invalid_grant及code状态 |
| `P25-C05-S05` code重放拒绝 | 已消费code第二次兑换被拒绝且不新增grant | [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [I22CODE](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L261)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:标准client或公共token重放子例 |
| `P25-C05-S06` 并发兑换单赢家 | 两个真实身份进程同时兑换同一code仅一个成功 | [I22CODE](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L261)<br>运行：[H23KEY](../../../../tests/unified-auth-https/phase23-key-cases.mjs#L50) | **PENDING**；EXTEND:实际双进程并发；同进程AppState不能替代 |

### C06 refresh

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C06-S01` 轮换 | 每次成功refresh替换原refresh并保持原sub/sid来源 | [I10](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2724), [PREFRESH](../../../../src/bff_tests/canonical.rs#L839), [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；记录原/新载体不同布尔值，禁止原值 |
| `P25-C06-S02` 并发 | 两个节点争用原refresh只允许规定结果且无双活family | [I22REFRESH](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L293), [PREFRESH](../../../../src/bff_tests/canonical.rs#L839)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:实际双进程及BFF并发 |
| `P25-C06-S03` reuse撤销family | 重用已消费refresh使同family存活后代失效且不越界其他来源 | [I10](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2724), [I22REFRESH](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L293)<br>运行：[H21REVOKE](../../../../tests/unified-auth-https/phase21-browser.mjs#L187) | **PENDING**；EXTEND:在线与离线family当前拒绝 |
| `P25-C06-S04` 绝对期限 | refresh无法延长原family绝对截止或BFF本地截止 | [I10](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2724), [PDEADLINE](../../../../src/bff_tests/canonical.rs#L886)<br>运行：[H21OFFLINE](../../../../tests/unified-auth-https/phase21-browser.mjs#L217) | **PENDING**；实际database时钟及原截止；期限配置标签不能冒充生效值 |
| `P25-C06-S05` 空闲期限 | idle超过已采纳窗口后拒绝，成功操作不突破绝对上限 | [I10](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2724)<br>运行：[H21OFFLINE](../../../../tests/unified-auth-https/phase21-browser.mjs#L217) | **PENDING**；当前受控fixture真实时间边界 |
| `P25-C06-S06` scope收缩 | 请求或权威scope缩小后新access仅保留交集 | [I10](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2724), [I10REPLAY](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2605)<br>运行：[H21SCOPE](../../../../tests/unified-auth-https/phase21-browser.mjs#L171) | **PENDING**；EXTEND:已批准额外scope后实际收缩 |
| `P25-C06-S07` 禁止scope扩权 | refresh添加未获授权resource/scope被拒绝 | [I10](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2724), [I09](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L3167)<br>运行：[H21SCOPE](../../../../tests/unified-auth-https/phase21-browser.mjs#L171) | **PENDING**；EXTEND:实际token负例 |
| `P25-C06-S08` 在线来源与离线分离 | online来源失效即拒绝；明确批准offline按7天绝对合同且可显式撤销 | [I10](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2724)<br>运行：[H21OFFLINE](../../../../tests/unified-auth-https/phase21-browser.mjs#L217) | **PENDING**；自然SSO到期与显式GLOBAL/consent revoke分别验收 |

### C07 客户端认证

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C07-S01` 公开client无secret | PUBLIC注册/运行不存储、不展示、不发送client secret | [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [UCLIENT](../../../../../frontend/tests/unified-clients-browser.mjs#L83)<br>运行：[H19REGISTRY](../../../../tests/unified-auth-https/phase19-browser.mjs#L369) | **PENDING**；EXTEND:实际PUBLIC客户端S256正例和secret字段拒绝 |
| `P25-C07-S02` 机密client错误凭据 | 错误/缺失Basic凭据拒绝，不能退回PUBLIC或其他client | [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [I08](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338), [I11](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2240)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150), [H21MACHINE](../../../../tests/unified-auth-https/phase21-browser.mjs#L192) | **PENDING**；EXTEND:人员与MACHINE各一次负例 |
| `P25-C07-S03` 凭据轮换 | 新secret成功、旧secret失效，保留明确版本与边界 | [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [I11](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2240), [I11AGE](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2184)<br>运行：[H23SERVICES](../../../../tests/unified-auth-https/phase23-service-access-cases.mjs#L43) | **PENDING**；EXTEND:OAuth client secret与service secret轮换分别报告 |
| `P25-C07-S04` 紧急撤销 | emergency revoke使旧credential版本及来源立即失效且不能重启复活 | [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [I11](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2240), [PMACHINE](../../../../src/identity_deployment_tests.rs#L461)<br>运行：[H23RESTORE](../../../../tests/unified-auth-https/phase23-browser.mjs#L591) | **PENDING**；EXTEND:实际client credential撤销与当前MACHINE请求 |

### C08 资源隔离

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C08-S01` ID Token拒绝API | ID Token作为Bearer访问业务API或introspection不会取得访问 | [I09](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L3167), [PRESOURCE](../../../../src/identity_deployment_tests/phase15_http.rs#L165)<br>运行：[H21RESOURCE](../../../../tests/unified-auth-https/phase21-browser.mjs#L180) | **PENDING**；EXTEND:实际stores/work-orders/resource API，不仅userinfo |
| `P25-C08-S02` 错误audience拒绝 | 一个资源access不能调用另一资源API | [I09](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L3167), [PRESOURCE](../../../../src/identity_deployment_tests/phase15_http.rs#L165)<br>运行：[H21RESOURCE](../../../../tests/unified-auth-https/phase21-browser.mjs#L180) | **PENDING**；EXTEND:stores和work-orders交叉audience |
| `P25-C08-S03` 跨资源scope拒绝 | 同名scope也必须绑定resource；不能拼接资源权限 | [I09](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L3167), [I11](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2240), [PRESOURCE](../../../../src/identity_deployment_tests/phase15_http.rs#L165)<br>运行：[H21RESOURCE](../../../../tests/unified-auth-https/phase21-browser.mjs#L180) | **PENDING**；EXTEND:实际两资源scope交叉 |
| `P25-C08-S04` 错误主体拒绝 | 不符合endpoint主体类型的USER/MACHINE及错误member来源均拒绝 | [I11](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2240), [PRESOURCE](../../../../src/identity_deployment_tests/phase15_http.rs#L165), [PBIND](../../../../src/identity_binding_tests.rs#L392)<br>运行：[H21RESOURCE](../../../../tests/unified-auth-https/phase21-browser.mjs#L180), [H21MACHINE](../../../../tests/unified-auth-https/phase21-browser.mjs#L192) | **PENDING**；EXTEND:资源端不能只在issuer测拒绝 |
| `P25-C08-S05` 机器不冒充人员 | MACHINE无人员sub/会员上下文，USER内部能力和客户会话不能由机器取得 | [I11](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2240), [PMACHINE](../../../../src/identity_deployment_tests.rs#L461), [PRESOURCE](../../../../src/identity_deployment_tests/phase15_http.rs#L165)<br>运行：[H21MACHINE](../../../../tests/unified-auth-https/phase21-browser.mjs#L192) | **PENDING**；断言无用户字段及人员endpoint拒绝 |

### C09 同意

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C09-S01` 首次确认 | 首次申请显示应用/用途/resource/scope且未批准不能签发 | [I07CONSENT](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L219), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:真实consent页拒绝及批准各一例 |
| `P25-C09-S02` 后续复用 | 同组版本与权限不变时复用已确认同意 | [I07CONSENT](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L219), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:第二次登录无再次提示且未扩权 |
| `P25-C09-S03` 新增scope再确认 | scope或group成员版本扩大必须再次提示，旧同意不能自动覆盖 | [I07CONSENT](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L219), [I07GROUP](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L526)<br>运行：[H21SCOPE](../../../../tests/unified-auth-https/phase21-browser.mjs#L171) | **PENDING**；EXTEND:实际管理员变更后浏览器再确认 |
| `P25-C09-S04` 跨组不继承 | 同账号在其他group/app/environment必须独立确认 | [I07CONSENT](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L219), [I06](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L126)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:实际两个group登录 |
| `P25-C09-S05` 同组受控共享 | 仅显式同app同environment同责任主体成员共享；新增成员提升group version | [I07GROUP](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L526), [I07CONSENT](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L219)<br>运行：[H19REGISTRY](../../../../tests/unified-auth-https/phase19-browser.mjs#L369) | **PENDING**；EXTEND:实际组成员登记和用户再同意 |

### C10 撤销与删除

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C10-S01` active不可直接删 | 活动consent DELETE失败且grant/receipt不变化 | [I07DELETE](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L318), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H21REVOKE](../../../../tests/unified-auth-https/phase21-browser.mjs#L187) | **PENDING**；EXTEND:实际用户管理API active删除负例 |
| `P25-C10-S02` 撤销来源均失效 | consent revoke结束相关code/access/refresh/BFF/内部capability且不影响无关组 | [I07DELETE](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L318), [PCAP](../../../../src/identity_deployment_tests/phase15.rs#L50)<br>运行：[H20APP](../../../../tests/unified-auth-https/phase20-browser.mjs#L177), [H21REVOKE](../../../../tests/unified-auth-https/phase21-browser.mjs#L187) | **PENDING**；EXTEND:调用consent revoke；CURRENT_APP/RFC7009不能替代组撤销 |
| `P25-C10-S03` 删除不复活 | 已撤销记录隐藏后旧token仍拒绝且墓碑/代际保留 | [I07DELETE](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L318), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33), [RECOVERY](../../../../ops/identity-recovery/test_reconcile.py#L136)<br>运行：[H23RESTORE](../../../../tests/unified-auth-https/phase23-browser.mjs#L591) | **PENDING**；EXTEND:实际删除后旧载体与恢复均拒绝 |
| `P25-C10-S04` 旧删除与重授权冲突 | 重新授权后旧expectedVersion DELETE冲突且新授权保留 | [I07DELETE](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L318)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:实际并发/旧版本API场景 |

### C11 上下文

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C11-S01` 零候选 | PLATFORM_CONTEXT或CUSTOMER_PORTAL零候选不自动批准也不造默认成员 | [I12](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826), [PCONTEXT](../../../../src/context_selection_http/tests.rs#L416), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H13](../../../../tests/unified-auth-https/browser.mjs#L229), [H14TYPE](../../../../tests/unified-auth-https/customer-browser.mjs#L508) | **PENDING**；EXTEND:真实零成员context用途拒绝，区别ACCOUNT_ONLY成功 |
| `P25-C11-S02` 单租户 | 一个租户多组织跳过租户步骤且保留组织选择 | [I12](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299) | **PENDING**；EXTEND:实际页面断言无tenant step |
| `P25-C11-S03` 多租户 | 先选租户再选组织，不按搜索或首条自动选择 | [I12](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299) | **PENDING**；EXTEND:实际多tenant fixture与页面 |
| `P25-C11-S04` 单组织 | 选定租户只一可用成员时精确自动继续 | [I12](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H13](../../../../tests/unified-auth-https/browser.mjs#L229), [H14TYPE](../../../../tests/unified-auth-https/customer-browser.mjs#L508) | **PENDING**；重验单项来源与实际member ID布尔绑定 |
| `P25-C11-S05` 多组织 | 一个租户多个合法成员需显式选择，会员/类型/代际完整绑定 | [I12](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826), [PCONTEXT](../../../../src/context_selection_http/tests.rs#L416), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299) | **PENDING**；当前多候选正例 |
| `P25-C11-S06` 记忆同意单项自动继续 | 同意充足且恰一合法候选时无需额外确认按钮 | [I12](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826), [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H13](../../../../tests/unified-auth-https/browser.mjs#L229) | **PENDING**；EXTEND:当前二次登录实际无点击观察 |
| `P25-C11-S07` 搜索不自动选 | 过滤至一项仍待显式选择，搜索不产生approve请求 | [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299) | **PENDING**；EXTEND:实际HTTPS网络计数与页面断言 |
| `P25-C11-S08` 员工客户隔离 | 同SSO下两独立BFF/cookie/member用途；反向建立会话失败 | [PSELECT](../../../../src/customer_portal_tests/selection.rs#L169), [PRESOURCE](../../../../src/identity_deployment_tests/phase15_http.rs#L165)<br>运行：[H14TYPE](../../../../tests/unified-auth-https/customer-browser.mjs#L508) | **PENDING**；纯员工/纯客户负例和双身份账号 |
| `P25-C11-S09` ACCOUNT_ONLY无context调用 | 账号登录与refresh不请求平台候选/selection/result接口 | [I12](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:记录目标path计数0而不仅页面没显示 |

### C12 业务权限

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C12-S01` 组织收缩 | 停用/撤销组织授权后旧能力立即拒绝且恢复组织不复活旧能力 | [PBIND](../../../../src/identity_binding_tests.rs#L392), [PCAP](../../../../src/identity_deployment_tests/phase15.rs#L50)<br>运行：[H13REVOKE](../../../../tests/unified-auth-https/browser.mjs#L330) | **PENDING**；EXTEND:实际资源请求前后验证 |
| `P25-C12-S02` 成员收缩 | 成员停用/退休/类型或identity_version变化使准确成员能力失效 | [PBIND](../../../../src/identity_binding_tests.rs#L392), [PCAP](../../../../src/identity_deployment_tests/phase15.rs#L50)<br>运行：[H13REVOKE](../../../../tests/unified-auth-https/browser.mjs#L330) | **PENDING**；EXTEND:实际stores/work-orders/notification，其他成员保持有效 |
| `P25-C12-S03` 应用准入收缩 | application/tenant/organization admission缩小在原事务内使准确app能力失效 | [PAPP](../../../../src/identity_binding_tests.rs#L472), [PVERSION](../../../../src/routes/admission_policy_tests.rs#L670), [PCAP](../../../../src/identity_deployment_tests/phase15.rs#L50)<br>运行：[H19PUBLISH](../../../../tests/unified-auth-https/phase19-browser.mjs#L423) | **PENDING**；EXTEND:实际应用撤销后资源端即时拒绝 |
| `P25-C12-S04` scope不越过用户权限 | 第三方获批scope仍需用户当前业务权限，不能绕过成员/应用上限 | [I09](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L3167), [PRESOURCE](../../../../src/identity_deployment_tests/phase15_http.rs#L165)<br>运行：[H21RESOURCE](../../../../tests/unified-auth-https/phase21-browser.mjs#L180) | **PENDING**；EXTEND:批准scope但无业务权限USER实际访问拒绝 |
| `P25-C12-S05` 恢复授权不恢复旧token | 重新启用组织/成员/app后只能新发能力，旧代际保持失效 | [PBIND](../../../../src/identity_binding_tests.rs#L392), [PCAP](../../../../src/identity_deployment_tests/phase15.rs#L50), [RECOVERY](../../../../ops/identity-recovery/test_reconcile.py#L136)<br>运行：[H23RESTORE](../../../../tests/unified-auth-https/phase23-browser.mjs#L591) | **PENDING**；EXTEND:实际重新启用后旧/新载体对照 |

### C13 跨库与标签页

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C13-S01` 原结果恢复 | 平台已消费而门户提交失败，重试恢复完全相同capability与截止 | [PSELECT](../../../../src/customer_portal_tests/selection.rs#L169)<br>运行：[H14RECOVER](../../../../tests/unified-auth-https/customer-browser.mjs#L418), [H14RETRY](../../../../tests/unified-auth-https/customer-browser.mjs#L446) | **PENDING**；保留平台先提交证据与同一个operation |
| `P25-C13-S02` 重试不重复消费 | 同operation重复result只一次消费/审计，不能生成新来源 | [PSELECT](../../../../src/customer_portal_tests/selection.rs#L169), [I12](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826)<br>运行：[H14RETRY](../../../../tests/unified-auth-https/customer-browser.mjs#L446) | **PENDING**；实际消费审计计数为1 |
| `P25-C13-S03` 切换冻结 | 发起组织切换同步冻结旧业务写入/缓存/通知/微应用直到新来源完整 | [UUPDATE](../../../../../frontend/tests/micro-context-updates.mjs#L86), [UNOTIFY](../../../../../frontend/tests/notification-context.mjs#L66)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299) | **PENDING**；EXTEND:实际资源及UI在切换待定窗口拒绝旧请求 |
| `P25-C13-S04` 旧请求不能覆盖 | 忽略AbortSignal的迟到成功/错误无法覆盖新member/version上下文 | [UUPDATE](../../../../../frontend/tests/micro-context-updates.mjs#L86), [UNOTIFY](../../../../../frontend/tests/notification-context.mjs#L66), [PLATE](../../../../src/bff_tests/canonical.rs#L1035)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299) | **PENDING**；EXTEND:实际HTTPS延迟窗口与两个上下文对照 |
| `P25-C13-S05` 多标签页同步 | 同浏览器两个标签页切换或登出后旧标签不能继续业务或覆盖新Cookie | [UTABS](../../../../../frontend/tests/bff-integration.mjs#L52), [PLATE](../../../../src/bff_tests/canonical.rs#L1035)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299), [H20GLOBAL](../../../../tests/unified-auth-https/phase20-browser.mjs#L201) | **PENDING**；GAP:独立browser context不等于两个tabs，需同context双page |
| `P25-C13-S06` 微应用同步 | 当前SDK/宿主收到新完整上下文，旧token/生命周期/调用失效 | [UUPDATE](../../../../../frontend/tests/micro-context-updates.mjs#L86), [UMICRO](../../../../../frontend/tests/micro-runtime-faults.mjs#L62)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299) | **PENDING**；GAP:实际micro app mount/update与stores/work-orders集成 |
| `P25-C13-S07` 通知同步 | 切换/成员代际变化取消旧HTTP/socket来源，新连接仅收准确recipient | [UNOTIFY](../../../../../frontend/tests/notification-context.mjs#L66), [NOTIFY](../../../../../../saas-notification/tests/identity_isolation.rs#L774)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299) | **PENDING**；GAP:实际notification进程/socket与新member proof，不能仅collector |

### C14 管理安全

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C14-S01` 查看权限 | VIEW可以读最小登记元数据，不能写/审/轮换 | [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [PPERMS](../../../../src/auth_review/tests.rs#L508), [UCLIENT](../../../../../frontend/tests/unified-clients-browser.mjs#L83)<br>运行：[H19REGISTRY](../../../../tests/unified-auth-https/phase19-browser.mjs#L369) | **PENDING**；EXTEND:实际VIEW-only actor负例 |
| `P25-C14-S02` 修改权限 | REGISTER/SCOPES只允许授权范围写入，不能审核或发布自己申请 | [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [PREVIEW](../../../../src/auth_review/tests.rs#L360), [PPERMS](../../../../src/auth_review/tests.rs#L508)<br>运行：[H19](../../../../tests/unified-auth-https/phase19-browser.mjs#L374), [H19REVIEW](../../../../tests/unified-auth-https/phase19-browser.mjs#L385) | **PENDING**；当前申请人与审核人独立 |
| `P25-C14-S03` 审核发布权限 | REVIEW与PUBLISH分离，申请不可自批；生产默认关闭且缺真实binding保持关闭 | [PREVIEW](../../../../src/auth_review/tests.rs#L360), [PPERMS](../../../../src/auth_review/tests.rs#L508), [I19](../../../../../../saas-identity/src/auth_review/tests.rs#L407)<br>运行：[H19REVIEW](../../../../tests/unified-auth-https/phase19-browser.mjs#L385), [H19PUBLISH](../../../../tests/unified-auth-https/phase19-browser.mjs#L423) | **PENDING**；任务fixture绑定不完成Q3真实发布责任 |
| `P25-C14-S04` 轮换权限 | ROTATE与KEY_ADMIN权限独立，无权actor不能改client/signing/service secret | [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [IKEYPERM](../../../../../../saas-identity/src/oidc/keys/lifecycle_tests.rs#L160)<br>运行：[H23KEY](../../../../tests/unified-auth-https/phase23-key-cases.mjs#L50) | **PENDING**；EXTEND:三种轮换权限分别拒绝，不能混算 |
| `P25-C14-S05` 越权拒绝 | 角色/应用委派撤销后新写与重放均失败，其他app对象不可写 | [PPERMS](../../../../src/auth_review/tests.rs#L508), [PAPP](../../../../src/identity_binding_tests.rs#L472)<br>运行：[H19PUBLISH](../../../../tests/unified-auth-https/phase19-browser.mjs#L423) | **PENDING**；EXTEND:跨app与撤权后replay |
| `P25-C14-S06` CSRF拒绝 | 缺失/错误Origin、CSRF、重复Cookie/参数不触发敏感变更 | [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9), [PMFA](../../../../src/auth_review/tests.rs#L481)<br>运行：[H19](../../../../tests/unified-auth-https/phase19-browser.mjs#L374), [H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:平台、SSO、RP实际负例分别记录 |
| `P25-C14-S07` 审计失败原子回滚 | 审计插入拒绝/静默抑制时mutation、receipt和能力撤销均回滚 | [PAUDIT](../../../../src/management_audit_tests.rs#L922), [PVERSION](../../../../src/routes/admission_policy_tests.rs#L670), [IKEYPERM](../../../../../../saas-identity/src/oidc/keys/lifecycle_tests.rs#L160)<br>运行：[H19](../../../../tests/unified-auth-https/phase19-browser.mjs#L374) | **PENDING**；EXTEND:仅任务DB窄故障注入且finally移除 |
| `P25-C14-S08` 幂等 | 相同operation同输入恢复原结果；不同输入冲突且不重复secret/审批 | [PREVIEW](../../../../src/auth_review/tests.rs#L360), [I04](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9), [PLOGOUT](../../../../src/bff_tests/canonical.rs#L999)<br>运行：[H14RETRY](../../../../tests/unified-auth-https/customer-browser.mjs#L446), [H19REVIEW](../../../../tests/unified-auth-https/phase19-browser.mjs#L385) | **PENDING**；EXTEND:管理操作丢失响应重试 |
| `P25-C14-S09` 版本冲突 | 陈旧expectedVersion不能覆盖新配置、审批或删除事实 | [PVERSION](../../../../src/routes/admission_policy_tests.rs#L670), [PREVIEW](../../../../src/auth_review/tests.rs#L360), [I07DELETE](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L318)<br>运行：[H19PUBLISH](../../../../tests/unified-auth-https/phase19-browser.mjs#L423) | **PENDING**；EXTEND:实际并发管理版本负例 |

### C15 登出

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C15-S01` 当前应用范围 | CURRENT_APP关闭原grant，保留同账号其他RP/同RP其他grant及consent | [ILOGOUT](../../../../../../saas-identity/src/oidc/oauth/logout/tests.rs#L54), [PLOGOUT](../../../../src/bff_tests/canonical.rs#L999)<br>运行：[H20APP](../../../../tests/unified-auth-https/phase20-browser.mjs#L177) | **PENDING**；实际两个RP及同RP独立设备对照 |
| `P25-C15-S02` 全局范围 | GLOBAL关闭账号全部原SSO与USER grant/离线family，保留consent | [ILOGOUT](../../../../../../saas-identity/src/oidc/oauth/logout/tests.rs#L54)<br>运行：[H20GLOBAL](../../../../tests/unified-auth-https/phase20-browser.mjs#L201) | **PENDING**；实际BFF/标准RP/SSO/离线范围 |
| `P25-C15-S03` 授权撤销范围 | consent revoke关闭准确组全部派生来源，其他组保持 | [I07DELETE](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L318), [IDELIVERY](../../../../../../saas-identity/src/delivery/worker/tests.rs#L41)<br>运行：[H20APP](../../../../tests/unified-auth-https/phase20-browser.mjs#L177) | **PENDING**；EXTEND:单独consent revoke API，不以CURRENT_APP替代 |
| `P25-C15-S04` back-channel签名 | 错误issuer/aud/signature/sub/events拒绝；有效通知只关闭原sid/sub | [ISIGNED](../../../../../../saas-identity/src/delivery/worker/tests.rs#L84), [PBACK](../../../../src/bff_tests/canonical.rs#L1120), [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H20SIGN](../../../../tests/unified-auth-https/phase20-browser.mjs#L207) | **PENDING**；实际worker与接收方都核对 |
| `P25-C15-S05` back-channel重放 | 同jti重放拒绝；未知sid tombstone阻止迟到callback/refresh复活 | [PBACK](../../../../src/bff_tests/canonical.rs#L1120), [RPCAP](../../../../../../saas-examples/oidc-rp/tests/capacity.test.mjs#L66)<br>运行：[H20SIGN](../../../../tests/unified-auth-https/phase20-browser.mjs#L207) | **PENDING**；EXTEND:未知sid与callback在途实际竞态 |
| `P25-C15-S06` back-channel重试 | 接收方不可用不阻塞已提交撤销；有界lease/retry，恢复原operation和预算 | [IDELIVERY](../../../../../../saas-identity/src/delivery/worker/tests.rs#L41)<br>运行：[H20RETRY](../../../../tests/unified-auth-https/phase20-browser.mjs#L184) | **PENDING**；当前实际collector中断并恢复 |
| `P25-C15-S07` 降级时效 | 无back-channel按本地<=300秒/在线<=60秒合同拒绝，不能宣称即时退出 | [IDELIVERY](../../../../../../saas-identity/src/delivery/worker/tests.rs#L41), [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H20DEGRADE](../../../../tests/unified-auth-https/phase20-browser.mjs#L196) | **PENDING**；EXTEND:实际最长观测值，不只读取配置标签 |

### C16 浏览器

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C16-S01` 桌面 | 当前身份/宿主/门户/独立RP主流程在桌面Chromium完成无页面错误 | [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33), [UCLIENT](../../../../../frontend/tests/unified-clients-browser.mjs#L83)<br>运行：[H13](../../../../tests/unified-auth-https/browser.mjs#L229), [H14TYPE](../../../../tests/unified-auth-https/customer-browser.mjs#L508), [H21DISCOVERY](../../../../tests/unified-auth-https/phase21-browser.mjs#L140) | **PENDING**；记录browser版本及受信TLS |
| `P25-C16-S02` 移动 | 390x844等窄屏不横向溢出且完成登录/同意/上下文/撤销 | [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33), [UCLIENT](../../../../../frontend/tests/unified-clients-browser.mjs#L83)<br>运行：[H21DISCOVERY](../../../../tests/unified-auth-https/phase21-browser.mjs#L140) | **PENDING**；EXTEND:不能只RP首页截图，完成身份与上下文移动流程 |
| `P25-C16-S03` 键盘 | Tab/Shift+Tab/Enter/Escape完成关键流程，Radio与按钮可操作 | [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33), [UCLIENT](../../../../../frontend/tests/unified-clients-browser.mjs#L83)<br>运行：[H13](../../../../tests/unified-auth-https/browser.mjs#L229) | **PENDING**；GAP:实际HTTPS键盘操作断言 |
| `P25-C16-S04` 焦点 | 对话框打开聚焦、焦点圈定、关闭回触发器；错误后焦点可恢复 | [UCLIENT](../../../../../frontend/tests/unified-clients-browser.mjs#L83)<br>运行：[H19REGISTRY](../../../../tests/unified-auth-https/phase19-browser.mjs#L369) | **PENDING**；GAP:activeElement/焦点循环断言 |
| `P25-C16-S05` 弹窗滚动 | 长client/consent表单可滚动且操作条可达，正文不超viewport | [UCLIENT](../../../../../frontend/tests/unified-clients-browser.mjs#L83)<br>运行：[H19REGISTRY](../../../../tests/unified-auth-https/phase19-browser.mjs#L369) | **PENDING**；EXTEND:实际长表单与窄屏滚动 |
| `P25-C16-S06` 开放重定向拒绝 | returnTo/callback/restart及URL参数不能导航未登记origin/path | [I12CALLBACK](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2546), [ICALLBACK](../../../../../../saas-identity/src/oidc/protocol.rs#L362), [RPFAIL](../../../../../../saas-examples/oidc-rp/tests/deployment.test.mjs#L45)<br>运行：[H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；EXTEND:实际浏览器不能只拒绝helper |
| `P25-C16-S07` XSS拒绝 | 恶意显示名/组织/应用/scope文本不创建可执行DOM；CSP保持 | [UCONTEXT](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33)<br>运行：[H13SWITCH](../../../../tests/unified-auth-https/browser.mjs#L299) | **PENDING**；EXTEND:任务新实体恶意文本并断言无执行 |
| `P25-C16-S08` Cookie受限 | 禁用/拦截Cookie有明确可恢复失败且不URL/本地存储降级泄漏凭据 | [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H13](../../../../tests/unified-auth-https/browser.mjs#L229), [H21LOGIN](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) | **PENDING**；GAP:实际阻断Cookie的context，无无限重定向/静默通过 |

### C17 故障

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C17-S01` 身份不可用 | identity中断时BFF/资源失败关闭，恢复后按原来源合同处理 | [POUTAGE](../../../../src/identity_binding_tests.rs#L502), [SFAIL](../../../../../../saas-stores/backend/src/authorization/tests.rs#L624), [WFAIL](../../../../../../saas-work-orders/backend/src/authorization/tests.rs#L537)<br>运行：[H22IDENTITY](../../../../tests/unified-auth-https/phase22-runtime.mjs#L236) | **PENDING**；不退回旧平台签发/本地密码 |
| `P25-C17-S02` 平台不可用 | 私有平台proof超时/拒绝时身份和资源拒绝且恢复不复活旧上下文 | [POUTAGE](../../../../src/identity_binding_tests.rs#L502), [SFAIL](../../../../../../saas-stores/backend/src/authorization/tests.rs#L624), [WFAIL](../../../../../../saas-work-orders/backend/src/authorization/tests.rs#L537)<br>运行：[H22PLATFORM](../../../../tests/unified-auth-https/phase22-runtime.mjs#L211) | **PENDING**；当前实际进程/有界网络故障 |
| `P25-C17-S03` 数据库不可用 | readiness失败而liveness正确，业务503且恢复不删合法来源 | [I22POOL](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L370)<br>运行：[H22DB](../../../../tests/unified-auth-https/phase22-runtime.mjs#L193) | **PENDING**；实际独占fixture库连接失败与恢复 |
| `P25-C17-S04` 连接池满 | 真实pool耗尽有界503，不能伪造401/撤销合法token或无限排队 | [I22POOL](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L370), [I22LOCK](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L463)<br>运行：[H22DB](../../../../tests/unified-auth-https/phase22-runtime.mjs#L193) | **PENDING**；EXTEND:runtime实际pool saturation，不仅DB关闭 |
| `P25-C17-S05` 队列满 | 密码/请求/投递队列满有界拒绝，释放后恢复且无无界pending | [IQUEUE](../../../../../../saas-identity/src/admission/tests.rs#L123), [IDELIVERY](../../../../../../saas-identity/src/delivery/worker/tests.rs#L41)<br>运行：[H22QUEUE](../../../../tests/unified-auth-https/phase22-runtime.mjs#L255) | **PENDING**；记录queue上限与实际拒绝 |
| `P25-C17-S06` 超时 | socket/slow body/upstream deadline按合同结束，旧异步成功不能提交 | [INET](../../../../../../saas-identity/src/network/tests.rs#L43), [PDEADLINE](../../../../src/bff_tests/canonical.rs#L886), [SFAIL](../../../../../../saas-stores/backend/src/authorization/tests.rs#L624), [WFAIL](../../../../../../saas-work-orders/backend/src/authorization/tests.rs#L537)<br>运行：[H22PLATFORM](../../../../tests/unified-auth-https/phase22-runtime.mjs#L211), [H22QUEUE](../../../../tests/unified-auth-https/phase22-runtime.mjs#L255) | **PENDING**；EXTEND:迟到reply及deadline以后源不可附加 |
| `P25-C17-S07` 时钟偏移 | 超出允许skew的ID Token/logout token/deadline拒绝；容许边界正确 | [PDEADLINE](../../../../src/bff_tests/canonical.rs#L886), [RPOIDC](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9)<br>运行：[H20SIGN](../../../../tests/unified-auth-https/phase20-browser.mjs#L207) | **PENDING**；GAP:过去/未来边界token及实际validator断言，不改宿主时间 |
| `P25-C17-S08` 错误语义与恢复 | 区分403/409/429/503，正文脱敏且恢复不重耗不确定refresh/code | [I22POOL](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L370), [PREFRESH](../../../../src/bff_tests/canonical.rs#L839), [PLATE](../../../../src/bff_tests/canonical.rs#L1035), [IREDACT](../../../../../../saas-identity/src/observability/tests.rs#L315)<br>运行：[H22DB](../../../../tests/unified-auth-https/phase22-runtime.mjs#L193), [H22IDENTITY](../../../../tests/unified-auth-https/phase22-runtime.mjs#L236), [H22QUEUE](../../../../tests/unified-auth-https/phase22-runtime.mjs#L255) | **PENDING**；逐端点状态/固定错误码与副作用计数 |

### C18 运维

| 子例 | 必须观察的行为 | 组件 / 候选运行 | 当前结果及剩余动作 |
| --- | --- | --- | --- |
| `P25-C18-S01` 密钥生命周期 | 预发布窗口、独立SDK缓存、原期限退役、紧急撤销和无活动signer失败关闭 | [IKEY](../../../../../../saas-identity/src/oidc/keys/lifecycle_tests.rs#L53), [IKEYPERM](../../../../../../saas-identity/src/oidc/keys/lifecycle_tests.rs#L160)<br>运行：[H23KEY](../../../../tests/unified-auth-https/phase23-key-cases.mjs#L50) | **PENDING**；实际full lifecycle；recovery profile单独不能替代 |
| `P25-C18-S02` 备份恢复deny事实 | 新目标restore/reconcile保留撤销/墓碑/代际/receipt/密钥与secret版本，未自然过期旧载体仍拒绝 | [RECOVERY](../../../../ops/identity-recovery/test_reconcile.py#L136)<br>运行：[H23RESTORE](../../../../tests/unified-auth-https/phase23-browser.mjs#L591), [H23FRESH](../../../../tests/unified-auth-https/phase23-browser.mjs#L635) | **PENDING**；实际RPO/RTO及carrier剩余期；stores/work-orders恢复不由身份工具自动覆盖 |
| `P25-C18-S03` 容量 | 实际成功CODE/REFRESH/INTROSPECTION负载报告速率、p95/错误/CPU/内存/队列；候选目标与生产承诺分开 | [I22POOL](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L370)<br>运行：[H22CAP](../../../../tests/unified-auth-https/phase22-runtime.mjs#L137) | **PENDING**；当前冻结版本实测，Q9生产容量/值班仍硬门禁 |
| `P25-C18-S04` 限流与告警 | client/account/source预算生效，socket轮换不重置；终止故障告警实际送达并确认 | [I22RATE](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L521), [IQUEUE](../../../../../../saas-identity/src/admission/tests.rs#L123)<br>运行：[H22QUEUE](../../../../tests/unified-auth-https/phase22-runtime.mjs#L255), [H22ALERT](../../../../tests/unified-auth-https/phase22-runtime.mjs#L129) | **PENDING**；EXTEND:实际429/Retry-After与独立alert sink |
| `P25-C18-S05` 日志脱敏 | 密码/secret/token/cookie/verifier/PHC/private keys在服务/网关/报告为0泄漏 | [IREDACT](../../../../../../saas-identity/src/observability/tests.rs#L315)<br>运行：[H20REDACT](../../../../tests/unified-auth-https/phase20-browser.mjs#L249) | **PENDING**；当前全服务canary扫描；仅保存计数和输入hash |
| `P25-C18-S06` 完整归属清理 | 删除本轮DB/roles/进程/日志/private files/临时网络镜像缓存；共享保留项精确列明 | [PBASE](../../../../src/identity_deployment_tests.rs#L317), [RECOVERY](../../../../ops/identity-recovery/test_reconcile.py#L136)<br>运行：[H23RESTORE](../../../../tests/unified-auth-https/phase23-browser.mjs#L591) | **PENDING**；本轮owner报告、docker前后df与精确归属审计；Docker预算0，不声称VHDX压缩 |

## 运行前适配与最小充分批次

- **A01 / RUNNER_ADAPTATION — PENDING**：At mapping time phase20-browser, phase21-browser and phase22-runtime hard-code old cluster 7701548153213487841. Preserve ownership checks; bind the actual current owned manifest/cluster before execution.
- **A02 / ARTIFACT_NAMESPACE — PENDING**：Historical output/snapshot prefixes are restricted to phase-21 / phase-23. Use isolated current-phase wrappers or reviewed explicit current-phase prefixes; never overwrite historical reports.
- **A03 / CANONICAL_TEST_COVERAGE — PENDING**：Identity protocol regressions use handwritten schema and mocked intent/selection ports. Run current canonical install plus actual HTTPS; do not promote these tests to canonical evidence.
- **A04 / NEW_EXPLICIT_ASSERTIONS — PENDING**：Explicit gaps: human auth_time, wrong ID Token audience, plain/malformed PKCE, same-context tabs, actual micro-app/notification synchronization, keyboard/focus/Cookie restrictions, clock skew.
- **A05 / FULL_SERVICE_INSTALL — PENDING**：Identity recovery runtime is shared identity/platform + portal + RP. It does not automatically fresh-install or restore stores/work-orders/notification. Bind those schemas/binaries and actual API/socket paths separately. SDK is an artifact, not another app container.
- **A06 / CLEANUP — PENDING**：Matrix work created no runtime resource. Environment owner must produce current-phase complete cleanup/disk evidence; shared retained resources need an explicit active owner and future cleanup scope.

| 批次 | 运行范围 | 状态 |
| --- | --- | --- |
| B01 Freeze and own | Complete eight-repository input/config/static/build manifests, binaries, migrations, CA and tool versions; explicit task cluster, DB/role/process/private-file ownership. | PENDING |
| B02 Canonical fresh installation | Install shared identity/platform, portal, stores and work-orders canonical 0001, notification schema and roles; start actual services plus independent RP and machine CLI; SDK/current examples bind same candidate. | PENDING |
| B03 Personnel and registry | Run adapted current H13/H14/H19; add explicit registry/OIDC/subject/PKCE/consent/context assertions. Every scenario writes its own result/evidence ID. | PENDING |
| B04 Standard and machine resource integration | Run H21 with actual openid-client/jose, stores/work-orders resource paths, approved MACHINE CLI, wrong audience/subject/scope and client secret rotation. | PENDING |
| B05 Browser synchronization and accessibility | Same browser context two pages, actual host micro-app SDK and notification socket; narrow screens, keyboard/focus/scroll, redirect/XSS and restricted Cookie. | PENDING |
| B06 Logout, failure and capacity | Adapted H20/H22 current runtime; pools/queues/timeouts/skew, original-operation recovery, measured revoke/degraded windows, independent alert sink and canary redaction. | PENDING |
| B07 Keys and recovery | Full H23 key lifecycle and restore lifecycle with exact provenance; current binaries or verified input equivalence, unexpired carriers, source/target timing and deny facts. Business-service backup boundaries stay explicit. | PENDING |
| B08 Candidate review and teardown | Complete target/config/runbook and Q gates, independent P1/P2 review, audit every result, then remove owned processes/DBs/roles/private files/logs/temporary resources and record disk accounting. | PENDING |

同一本次环境内复用这些批次。构建／只读映射可并行；冻结 ENV/bin/manifest、DDL、HBA、故障、密钥、restore 和清理只有一个 owner 操作。

## 真实发布门禁

下列事实是 Phase26 硬前置，不把 fixture 或默认政策当作确认事实，也不阻止无关 Phase25 准备。

| 门禁 | 所需真实事实 | 状态 |
| --- | --- | --- |
| Q3 | Actual distinct reviewer/publisher accounts, delegated responsibility, contact and approval records. Fixture actors do not satisfy release binding. | PENDING_REAL_FACT |
| Q8 | Actual operating jurisdiction/contracts and applicability of retention/deletion policy. Defaults are not legal approval. | PENDING_REAL_FACT |
| Q9 | Named ops/on-call responsibility and measured actual-target capacity/SLO/RPO/RTO acceptance. Local fixture results are internal candidate evidence only. | PENDING_REAL_FACT |
| Q10 | User confirmed local host and auth.saas.test / console.saas.test. Verify exact target DB names/owners, protected secret injection, services/ports/CA/callback/resources, switch actions and rollback triggers; obtain corresponding actual-operation authorization. | PENDING_EXACT_TARGET_AND_OPERATION |

候选目标／参数／回退清单还须包含：实际主机及运行账号；issuer/CA/DNS/端口/精确callback/audience/scope；库名、owner/runtime角色及schema摘要；秘密注入引用和权限；binary／静态产物摘要；健康与告警；备份恢复边界；切换动作与授权；失败触发、停止接流、保留deny事实的安全回退；冒烟、观察和资源清理责任。身份恢复工具只处理声明的 shared identity/platform + portal，不能代表 stores/work-orders 业务恢复。

## 源码与运行入口索引

| ID | 层级 | 当前 selector | 文件 |
| --- | --- | --- | --- |
| I04 | `SMALL_SCHEMA_PG` | `async fn phase04_unified_client_api_enforces_policy_paging_and_credential_lifecycle` | [saas-identity/src/util/auth_admin_tests.rs:9](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L9) |
| I07GROUP | `SMALL_SCHEMA_PG` | `async fn phase07_consent_groups_require_explicit_context_bound_membership` | [saas-identity/src/util/auth_admin_tests.rs:526](../../../../../../saas-identity/src/util/auth_admin_tests.rs#L526) |
| I06 | `SMALL_SCHEMA_PG` | `async fn phase06_pairwise_subjects_are_sector_scoped_stable_and_retired_with_accounts` | [saas-identity/src/util/audit_tests/auth_subjects.rs:126](../../../../../../saas-identity/src/util/audit_tests/auth_subjects.rs#L126) |
| I07CONSENT | `SMALL_SCHEMA_PG` | `async fn phase07_consent_coverage_expansion_and_group_isolation_use_canonical_authority` | [saas-identity/src/util/audit_tests/auth_consents.rs:219](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L219) |
| I07DELETE | `SMALL_SCHEMA_PG` | `async fn phase07_revoke_delete_reauthorize_and_logout_keep_generation_fences` | [saas-identity/src/util/audit_tests/auth_consents.rs:318](../../../../../../saas-identity/src/util/audit_tests/auth_consents.rs#L318) |
| I08 | `SMALL_SCHEMA_PG` | `async fn phase08_authorization_code_pkce_consent_pairwise_id_token_and_userinfo_use_one_registry` | [saas-identity/src/util/audit_tests/auth_protocol.rs:1338](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1338) |
| I09 | `SMALL_SCHEMA_PG` | `async fn phase09_resource_access_is_audience_scope_bound_and_revocation_is_token_local` | [saas-identity/src/util/audit_tests/auth_protocol.rs:3167](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L3167) |
| I10 | `SMALL_SCHEMA_PG` | `async fn phase10_refresh_rotation_reuse_concurrency_limits_and_offline_source_are_enforced` | [saas-identity/src/util/audit_tests/auth_protocol.rs:2724](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2724) |
| I10REPLAY | `SMALL_SCHEMA_PG` | `async fn phase10_scope_shrink_cannot_mask_consumed_refresh_replay` | [saas-identity/src/util/audit_tests/auth_protocol.rs:2605](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2605) |
| I11 | `SMALL_SCHEMA_PG` | `async fn phase11_client_credentials_are_machine_bound_to_active_registered_permissions` | [saas-identity/src/util/audit_tests/auth_protocol.rs:2240](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2240) |
| I11AGE | `SMALL_SCHEMA_PG` | `async fn phase11_credential_age_caps_authentication_machine_grants_and_introspection` | [saas-identity/src/util/audit_tests/auth_protocol.rs:2184](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2184) |
| I12CALLBACK | `SMALL_SCHEMA_PG` | `async fn phase12_invalid_client_context_never_redirects_to_unregistered_callback` | [saas-identity/src/util/audit_tests/auth_protocol.rs:2546](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L2546) |
| I12 | `SMALL_SCHEMA_PG_MOCK_PORT` | `async fn phase12_unified_context_selection_is_source_bound_recoverable_and_one_time` | [saas-identity/src/util/audit_tests/auth_protocol.rs:1826](../../../../../../saas-identity/src/util/audit_tests/auth_protocol.rs#L1826) |
| I19 | `CANONICAL_PG` | `async fn phase19_third_party_client_registration_and_runtime_require_current_environment_admission` | [saas-identity/src/auth_review/tests.rs:407](../../../../../../saas-identity/src/auth_review/tests.rs#L407) |
| ICLAIMS | `UNIT` | `fn id_token_and_userinfo_share_only_consented_profile_claims` | [saas-identity/src/oidc/subjects.rs:296](../../../../../../saas-identity/src/oidc/subjects.rs#L296) |
| IPKCE | `UNIT` | `fn pkce_uses_the_rfc7636_s256_transform` | [saas-identity/src/oidc/oauth.rs:4066](../../../../../../saas-identity/src/oidc/oauth.rs#L4066) |
| IRETIRED | `ROUTER_FIXTURE` | `async fn retired_protocol_and_registry_routes_are_absent` | [saas-identity/src/oidc/protocol.rs:255](../../../../../../saas-identity/src/oidc/protocol.rs#L255) |
| ICALLBACK | `UNIT` | `fn callbacks_keep_exact_registered_url_and_bind_issuer` | [saas-identity/src/oidc/protocol.rs:362](../../../../../../saas-identity/src/oidc/protocol.rs#L362) |
| ILOGOUT | `CANONICAL_PG` | `async fn current_app_and_global_preserve_consent_and_original_operation` | [saas-identity/src/oidc/oauth/logout/tests.rs:54](../../../../../../saas-identity/src/oidc/oauth/logout/tests.rs#L54) |
| IDELIVERY | `CANONICAL_PG` | `async fn concurrent_leases_retry_budget_and_stale_completion_are_bounded` | [saas-identity/src/delivery/worker/tests.rs:41](../../../../../../saas-identity/src/delivery/worker/tests.rs#L41) |
| ISIGNED | `CANONICAL_PG` | `async fn logout_payload_is_signed_for_original_rp_and_target_generation` | [saas-identity/src/delivery/worker/tests.rs:84](../../../../../../saas-identity/src/delivery/worker/tests.rs#L84) |
| I22CODE | `CANONICAL_PG` | `async fn independent_nodes_redeem_one_actual_authorization_code_once` | [saas-identity/src/oidc/oauth/phase22_tests.rs:261](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L261) |
| I22REFRESH | `CANONICAL_PG` | `async fn independent_nodes_refresh_race_detects_reuse_without_crossing_sources` | [saas-identity/src/oidc/oauth/phase22_tests.rs:293](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L293) |
| I22POOL | `CANONICAL_PG` | `async fn saturated_and_closed_pools_return_503_without_clearing_legal_sources` | [saas-identity/src/oidc/oauth/phase22_tests.rs:370](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L370) |
| I22LOCK | `CANONICAL_PG` | `async fn actual_client_table_lock_failure_is_503_and_preserves_original_tokens` | [saas-identity/src/oidc/oauth/phase22_tests.rs:463](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L463) |
| I22RATE | `CANONICAL_PG` | `async fn client_budget_cannot_be_reset_by_rotating_socket_sources` | [saas-identity/src/oidc/oauth/phase22_tests.rs:521](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L521) |
| IQUEUE | `ROUTER_FIXTURE` | `async fn full_request_budget_and_oversized_body_do_not_reach_authority` | [saas-identity/src/admission/tests.rs:123](../../../../../../saas-identity/src/admission/tests.rs#L123) |
| INET | `TCP_FIXTURE` | `async fn slow_http_body_times_out_before_authority_and_releases_admission` | [saas-identity/src/network/tests.rs:43](../../../../../../saas-identity/src/network/tests.rs#L43) |
| IREDACT | `UNIT` | `async fn request_bodies_headers_and_queries_never_enter_metrics_or_password_logs` | [saas-identity/src/observability/tests.rs:315](../../../../../../saas-identity/src/observability/tests.rs#L315) |
| IKEY | `CANONICAL_PG_LOGIN` | `async fn phase23_keys_publish_activate_replay_terminal_and_emergency_boundaries` | [saas-identity/src/oidc/keys/lifecycle_tests.rs:53](../../../../../../saas-identity/src/oidc/keys/lifecycle_tests.rs#L53) |
| IKEYPERM | `CANONICAL_PG_LOGIN` | `async fn phase23_keys_permissions_intent_and_failed_audit_receipt_are_atomic` | [saas-identity/src/oidc/keys/lifecycle_tests.rs:160](../../../../../../saas-identity/src/oidc/keys/lifecycle_tests.rs#L160) |
| PRETIRED | `ROUTER_FIXTURE` | `async fn retired_protocol_and_registry_routes_stay_absent_for_every_deployment_mode` | [saas-plateform/backend/src/protocol_retirement_tests.rs:10](../../../../src/protocol_retirement_tests.rs#L10) |
| PBASE | `CANONICAL_PG_LOGIN` | `async fn phase02_auth_model_constraints_and_runtime_boundary` | [saas-plateform/backend/src/identity_deployment_tests.rs:317](../../../../src/identity_deployment_tests.rs#L317) |
| PMACHINE | `CANONICAL_PG_LOGIN` | `async fn phase11_machine_grants_require_a_persisted_client_credential_version` | [saas-plateform/backend/src/identity_deployment_tests.rs:461](../../../../src/identity_deployment_tests.rs#L461) |
| PCONTEXT | `CANONICAL_PG_MOCK_PORT` | `async fn real_platform_selection_http_is_member_and_application_scoped` | [saas-plateform/backend/src/context_selection_http/tests.rs:416](../../../../src/context_selection_http/tests.rs#L416) |
| PSELECT | `CANONICAL_PG_MOCK_PORT` | `async fn centralized_customer_consumption_recovery_and_switch_are_exact` | [saas-plateform/backend/src/customer_portal_tests/selection.rs:169](../../../../src/customer_portal_tests/selection.rs#L169) |
| PREFRESH | `CANONICAL_PG_MOCK_PORT` | `async fn canonical_cross_instance_refresh_single_winner_and_lost_reply_never_replays` | [saas-plateform/backend/src/bff_tests/canonical.rs:839](../../../../src/bff_tests/canonical.rs#L839) |
| PLOCK | `CANONICAL_PG_MOCK_PORT` | `async fn canonical_post_network_account_lock_rechecks_original_source` | [saas-plateform/backend/src/bff_tests/canonical.rs:868](../../../../src/bff_tests/canonical.rs#L868) |
| PDEADLINE | `CANONICAL_PG_MOCK_PORT` | `async fn canonical_short_id_and_online_access_deadlines_cannot_commit_after_lock_wait` | [saas-plateform/backend/src/bff_tests/canonical.rs:886](../../../../src/bff_tests/canonical.rs#L886) |
| PLOGOUT | `CANONICAL_PG_MOCK_PORT` | `async fn canonical_cookie_reload_recovers_only_original_unknown_logout` | [saas-plateform/backend/src/bff_tests/canonical.rs:999](../../../../src/bff_tests/canonical.rs#L999) |
| PLATE | `CANONICAL_PG_MOCK_PORT` | `async fn canonical_late_logout_cannot_erase_new_account_cookie` | [saas-plateform/backend/src/bff_tests/canonical.rs:1035](../../../../src/bff_tests/canonical.rs#L1035) |
| PBACK | `CANONICAL_PG_MOCK_PORT` | `async fn canonical_backchannel_before_callback_persists_fence_without_revoking_new_source` | [saas-plateform/backend/src/bff_tests/canonical.rs:1120](../../../../src/bff_tests/canonical.rs#L1120) |
| PRESOURCE | `CANONICAL_PG_LOGIN_HTTP_FIXTURE` | `async fn phase15_resource_http_enforces_private_service_user_and_machine_boundaries` | [saas-plateform/backend/src/identity_deployment_tests/phase15_http.rs:165](../../../../src/identity_deployment_tests/phase15_http.rs#L165) |
| PCAP | `CANONICAL_PG_LOGIN` | `async fn phase15_real_roles_keep_internal_capabilities_source_bound_and_nonreviving` | [saas-plateform/backend/src/identity_deployment_tests/phase15.rs:50](../../../../src/identity_deployment_tests/phase15.rs#L50) |
| PBIND | `CANONICAL_PG_MOCK_PORT` | `async fn identity_binding_canonical_exact_member_revocation` | [saas-plateform/backend/src/identity_binding_tests.rs:392](../../../../src/identity_binding_tests.rs#L392) |
| PAPP | `CANONICAL_PG_MOCK_PORT` | `async fn identity_binding_canonical_application_capability_isolation` | [saas-plateform/backend/src/identity_binding_tests.rs:472](../../../../src/identity_binding_tests.rs#L472) |
| POUTAGE | `CANONICAL_PG_MOCK_PORT` | `async fn identity_binding_canonical_authority_outage_does_not_execute_business` | [saas-plateform/backend/src/identity_binding_tests.rs:502](../../../../src/identity_binding_tests.rs#L502) |
| PREVIEW | `CANONICAL_PG` | `async fn phase19_canonical_submit_review_self_denial_and_durable_operations` | [saas-plateform/backend/src/auth_review/tests.rs:360](../../../../src/auth_review/tests.rs#L360) |
| PPERMS | `CANONICAL_PG` | `async fn phase19_canonical_permission_revocation_blocks_new_writes_and_replays` | [saas-plateform/backend/src/auth_review/tests.rs:508](../../../../src/auth_review/tests.rs#L508) |
| PMFA | `CANONICAL_PG` | `async fn phase19_canonical_missing_and_stale_mfa_facts_block_writes_but_allow_bootstrap` | [saas-plateform/backend/src/auth_review/tests.rs:481](../../../../src/auth_review/tests.rs#L481) |
| PVERSION | `CANONICAL_PG` | `async fn admission_policy_audit_atomicity_and_concurrent_conflicts` | [saas-plateform/backend/src/routes/admission_policy_tests.rs:670](../../../../src/routes/admission_policy_tests.rs#L670) |
| PAUDIT | `CANONICAL_PG` | `async fn application_and_menu_cascades_keep_snapshots_and_atomic_audit` | [saas-plateform/backend/src/management_audit_tests.rs:922](../../../../src/management_audit_tests.rs#L922) |
| UCONTEXT | `BROWSER_HTTP_FIXTURE` | `async function test(name,fixture,run` | [saas-identity/tests/context-selection-ui.mjs:33](../../../../../../saas-identity/tests/context-selection-ui.mjs#L33) |
| UCLIENT | `BROWSER_ROUTE_FIXTURE` | `const scenarios = [], screenshots = []` | [saas-plateform/frontend/tests/unified-clients-browser.mjs:83](../../../../../frontend/tests/unified-clients-browser.mjs#L83) |
| UUPDATE | `UNIT_JS` | `test('authorization, membership, complete entry tuple and deadline changes still cancel the prior snapshot'` | [saas-plateform/frontend/tests/micro-context-updates.mjs:86](../../../../../frontend/tests/micro-context-updates.mjs#L86) |
| UNOTIFY | `UNIT_JS` | `test('old HTTP body and old HTTP errors cannot cross into a new organization when transport ignores AbortSignal'` | [saas-plateform/frontend/tests/notification-context.mjs:66](../../../../../frontend/tests/notification-context.mjs#L66) |
| UMICRO | `UNIT_JS` | `test('a never completing mount reaches its deadline and cancels authorization instead of hanging'` | [saas-plateform/frontend/tests/micro-runtime-faults.mjs:62](../../../../../frontend/tests/micro-runtime-faults.mjs#L62) |
| UTABS | `UNIT_JS` | `test('a BFF member switch retires an actual API response even when transport ignores abort'` | [saas-plateform/frontend/tests/bff-integration.mjs:52](../../../../../frontend/tests/bff-integration.mjs#L52) |
| SFAIL | `HTTP_FIXTURE` | `async fn upstream_status_body_redirect_json_and_capacity_fail_closed` | [saas-stores/backend/src/authorization/tests.rs:624](../../../../../../saas-stores/backend/src/authorization/tests.rs#L624) |
| WFAIL | `HTTP_FIXTURE` | `async fn upstream_status_body_redirect_json_and_capacity_fail_closed` | [saas-work-orders/backend/src/authorization/tests.rs:537](../../../../../../saas-work-orders/backend/src/authorization/tests.rs#L537) |
| NOTIFY | `SMALL_SCHEMA_PG_HTTP_FIXTURE` | `async fn dual_identity_delivery_local_revocation_replacement_and_reconnect` | [saas-notification/tests/identity_isolation.rs:774](../../../../../../saas-notification/tests/identity_isolation.rs#L774) |
| RPOIDC | `STANDARD_CLIENT_TLS_FIXTURE` | `test('real HTTPS standard discovery, Basic, S256, signed ID Token and matching opaque UserInfo establish a separate session'` | [saas-examples/oidc-rp/tests/oidc.test.mjs:9](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L9) |
| RPFAIL | `STANDARD_CLIENT_TLS_FIXTURE` | `test('HTTPS transport rejects missing trust, a different origin, private paths and redirects'` | [saas-examples/oidc-rp/tests/deployment.test.mjs:45](../../../../../../saas-examples/oidc-rp/tests/deployment.test.mjs#L45) |
| RPCAP | `STANDARD_CLIENT_TLS_FIXTURE` | `test('source-full signed logout closes the exact local source without acknowledging or evicting earlier facts'` | [saas-examples/oidc-rp/tests/capacity.test.mjs:66](../../../../../../saas-examples/oidc-rp/tests/capacity.test.mjs#L66) |
| RECOVERY | `PYTHON_COMPONENT` | `def test_canonical_all_old_carriers_are_explicitly_closed` | [saas-plateform/backend/ops/identity-recovery/test_reconcile.py:136](../../../../ops/identity-recovery/test_reconcile.py#L136) |
| H13 | `HTTPS_BROWSER` | `step = 'administrator-real-login-single-employee'` | [saas-plateform/backend/tests/unified-auth-https/browser.mjs:229](../../../../tests/unified-auth-https/browser.mjs#L229) |
| H13SWITCH | `HTTPS_BROWSER` | `step = 'real-central-context-switch'` | [saas-plateform/backend/tests/unified-auth-https/browser.mjs:299](../../../../tests/unified-auth-https/browser.mjs#L299) |
| H13REVOKE | `HTTPS_BROWSER` | `step = 'revoked-source-fails-closed'` | [saas-plateform/backend/tests/unified-auth-https/browser.mjs:330](../../../../tests/unified-auth-https/browser.mjs#L330) |
| H14RECOVER | `HTTPS_BROWSER` | `step = 'committed-platform-selection-survives-local-save-failure'` | [saas-plateform/backend/tests/unified-auth-https/customer-browser.mjs:418](../../../../tests/unified-auth-https/customer-browser.mjs#L418) |
| H14RETRY | `HTTPS_BROWSER` | `step = 'same-operation-recovers-same-capability-without-expiry-extension'` | [saas-plateform/backend/tests/unified-auth-https/customer-browser.mjs:446](../../../../tests/unified-auth-https/customer-browser.mjs#L446) |
| H14TYPE | `HTTPS_BROWSER` | `step = 'same-browser-sso-with-independent-console-and-customer-cookies'` | [saas-plateform/backend/tests/unified-auth-https/customer-browser.mjs:508](../../../../tests/unified-auth-https/customer-browser.mjs#L508) |
| H19 | `HTTPS_BROWSER` | `step = 'real-partner-and-application-submission'` | [saas-plateform/backend/tests/unified-auth-https/phase19-browser.mjs:374](../../../../tests/unified-auth-https/phase19-browser.mjs#L374) |
| H19REGISTRY | `HTTPS_BROWSER` | `step = 'canonical-registry-pagination-through-real-private-guard'` | [saas-plateform/backend/tests/unified-auth-https/phase19-browser.mjs:369](../../../../tests/unified-auth-https/phase19-browser.mjs#L369) |
| H19MFA | `HTTPS_BROWSER` | `step = 'real-applicant-enrollment-and-fresh-primary-totp-new-bff'` | [saas-plateform/backend/tests/unified-auth-https/phase19-browser.mjs:368](../../../../tests/unified-auth-https/phase19-browser.mjs#L368) |
| H19REVIEW | `HTTPS_BROWSER` | `step = 'real-independent-test-approval-and-identity-admission'` | [saas-plateform/backend/tests/unified-auth-https/phase19-browser.mjs:385](../../../../tests/unified-auth-https/phase19-browser.mjs#L385) |
| H19PUBLISH | `HTTPS_BROWSER` | `step = 'fixture-bound-production-real-review-publish-admission-suspend'` | [saas-plateform/backend/tests/unified-auth-https/phase19-browser.mjs:423](../../../../tests/unified-auth-https/phase19-browser.mjs#L423) |
| H20APP | `HTTPS_BROWSER` | `step = 'current-app-original-grant-only-with-consent-and-other-rp-preserved'` | [saas-plateform/backend/tests/unified-auth-https/phase20-browser.mjs:177](../../../../tests/unified-auth-https/phase20-browser.mjs#L177) |
| H20GLOBAL | `HTTPS_BROWSER` | `step = 'global-closes-both-rps-and-sso-with-consent-preserved'` | [saas-plateform/backend/tests/unified-auth-https/phase20-browser.mjs:201](../../../../tests/unified-auth-https/phase20-browser.mjs#L201) |
| H20SIGN | `HTTPS_BROWSER` | `step = 'signed-receiver-audience-signature-sub-and-replay-denials'` | [saas-plateform/backend/tests/unified-auth-https/phase20-browser.mjs:207](../../../../tests/unified-auth-https/phase20-browser.mjs#L207) |
| H20RETRY | `HTTPS_BROWSER` | `step='collector-outage-does-not-delay-canonical-revoke-and-recovers-same-operation'` | [saas-plateform/backend/tests/unified-auth-https/phase20-browser.mjs:184](../../../../tests/unified-auth-https/phase20-browser.mjs#L184) |
| H20DEGRADE | `HTTPS_BROWSER` | `step='short-session-contract-is-explicitly-degraded-and-rejects-request-destinations'` | [saas-plateform/backend/tests/unified-auth-https/phase20-browser.mjs:196](../../../../tests/unified-auth-https/phase20-browser.mjs#L196) |
| H20REDACT | `HTTPS_BROWSER` | `step = 'durable-record-redaction-and-actual-tls'` | [saas-plateform/backend/tests/unified-auth-https/phase20-browser.mjs:249](../../../../tests/unified-auth-https/phase20-browser.mjs#L249) |
| H21LOGIN | `HTTPS_BROWSER_STANDARD_CLIENT` | `step = 'no-membership-account-standard-login-id-token-userinfo-and-refresh'` | [saas-plateform/backend/tests/unified-auth-https/phase21-browser.mjs:150](../../../../tests/unified-auth-https/phase21-browser.mjs#L150) |
| H21DISCOVERY | `HTTPS_BROWSER_STANDARD_CLIENT` | `step = 'standalone-desktop-mobile-and-exact-discovery'` | [saas-plateform/backend/tests/unified-auth-https/phase21-browser.mjs:140](../../../../tests/unified-auth-https/phase21-browser.mjs#L140) |
| H21SCOPE | `HTTPS_BROWSER_STANDARD_CLIENT` | `step = 'third-party-offline-and-unapproved-resource-scope-rejected'` | [saas-plateform/backend/tests/unified-auth-https/phase21-browser.mjs:171](../../../../tests/unified-auth-https/phase21-browser.mjs#L171) |
| H21RESOURCE | `HTTPS_BROWSER_STANDARD_CLIENT` | `step = 'standard-resource-audience-id-token-and-unapproved-business-denied'` | [saas-plateform/backend/tests/unified-auth-https/phase21-browser.mjs:180](../../../../tests/unified-auth-https/phase21-browser.mjs#L180) |
| H21REVOKE | `HTTPS_BROWSER_STANDARD_CLIENT` | `step = 'standard-rfc7009-revocation-and-refresh-replay'` | [saas-plateform/backend/tests/unified-auth-https/phase21-browser.mjs:187](../../../../tests/unified-auth-https/phase21-browser.mjs#L187) |
| H21MACHINE | `HTTPS_STANDARD_CLIENT` | `step = 'standard-machine-cli-approved-resource-scope-and-person-separation'` | [saas-plateform/backend/tests/unified-auth-https/phase21-browser.mjs:192](../../../../tests/unified-auth-https/phase21-browser.mjs#L192) |
| H21OFFLINE | `HTTPS_BROWSER_STANDARD_CLIENT` | `step = 'approved-offline-absolute-idle-and-natural-sso-expiry'` | [saas-plateform/backend/tests/unified-auth-https/phase21-browser.mjs:217](../../../../tests/unified-auth-https/phase21-browser.mjs#L217) |
| H22CAP | `HTTPS_RUNTIME` | `stage = 'bounded-successful-code-refresh-introspection-capacity'` | [saas-plateform/backend/tests/unified-auth-https/phase22-runtime.mjs:137](../../../../tests/unified-auth-https/phase22-runtime.mjs#L137) |
| H22DB | `HTTPS_RUNTIME` | `stage = 'canonical-database-outage-and-recovery'` | [saas-plateform/backend/tests/unified-auth-https/phase22-runtime.mjs:193](../../../../tests/unified-auth-https/phase22-runtime.mjs#L193) |
| H22PLATFORM | `HTTPS_RUNTIME` | `stage = 'private-platform-response-timeout-and-recovery'` | [saas-plateform/backend/tests/unified-auth-https/phase22-runtime.mjs:211](../../../../tests/unified-auth-https/phase22-runtime.mjs#L211) |
| H22IDENTITY | `HTTPS_BROWSER` | `stage = 'identity-unavailable-and-recovery-with-original-cookie'` | [saas-plateform/backend/tests/unified-auth-https/phase22-runtime.mjs:236](../../../../tests/unified-auth-https/phase22-runtime.mjs#L236) |
| H22QUEUE | `HTTPS_RUNTIME` | `stage = 'password-admission-queue-deadline-and-recovery'` | [saas-plateform/backend/tests/unified-auth-https/phase22-runtime.mjs:255](../../../../tests/unified-auth-https/phase22-runtime.mjs#L255) |
| H22ALERT | `HTTPS_RUNTIME` | `stage = 'independent-local-alert-pipeline'` | [saas-plateform/backend/tests/unified-auth-https/phase22-runtime.mjs:129](../../../../tests/unified-auth-https/phase22-runtime.mjs#L129) |
| H23KEY | `PORT_DRIVER` | `export async function runKeyCases` | [saas-plateform/backend/tests/unified-auth-https/phase23-key-cases.mjs:50](../../../../tests/unified-auth-https/phase23-key-cases.mjs#L50) |
| H23RESTORE | `HTTPS_BROWSER_STANDARD_CLIENT` | `step='OLD_RAW_CARRIERS_REJECTED'` | [saas-plateform/backend/tests/unified-auth-https/phase23-browser.mjs:591](../../../../tests/unified-auth-https/phase23-browser.mjs#L591) |
| H23FRESH | `HTTPS_BROWSER_STANDARD_CLIENT` | `step='FRESH_RECOVERED_BUSINESS'` | [saas-plateform/backend/tests/unified-auth-https/phase23-browser.mjs:635](../../../../tests/unified-auth-https/phase23-browser.mjs#L635) |
| H23SERVICES | `PORT_DRIVER` | `export async function checkServiceAccessControls` | [saas-plateform/backend/tests/unified-auth-https/phase23-service-access-cases.mjs:43](../../../../tests/unified-auth-https/phase23-service-access-cases.mjs#L43) |

## 本文自检

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| 架构类别与独立子例 | PASS | 18 类、111 个稳定 ID，所有架构行均有子例 |
| 当前源码定位与摘要 | PASS | 每个 selector 存在，JSON 保存 SHA256 与行号；变化后须重核 |
| JSON 与相对文件链接 | PASS | 生成后核查唯一 ID、引用闭包、文件和 selector |
| Phase25 动态验收 | PENDING | 本文未执行 DB、HTTPS、浏览器、标准客户端、故障或恢复 |
| 实际环境清理 | PENDING | 由环境 owner 产出本轮证据；本文没创建运行资源 |

本文未修改台账，未执行 Phase26。
