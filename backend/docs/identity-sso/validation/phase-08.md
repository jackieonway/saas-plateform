# Phase 8：全局session与登录Cookie验收

日期：2026-10-25。状态：**COMPLETED**。依赖 [Phase 7](phase-07.md) 已完成。194项普通测试、21项新增数据库测试、182项既有ignored、3项rollout启动／receipt、16项normal binary配置与15项真实Edge检查均通过；本次浏览器进程日志扫描未命中六类敏感模式。集中门禁及既有回归两条串行命令链均exit 0，全部任务库清理成功。

范围依据 [台账Phase8](../task-ledger.md) 与 [全局session边界](../phase-08-global-session.md)：无成员账号认证、独立全局session与Cookie、CSRF／登录保护、全局及局部撤销、旧新凭据权威版本及故障拒绝，以及个人账号命令提交前再验。完整OIDC、BFF、门户与独立服务不在本阶段交付范围。

## 台账标准

| 验收项 | 结果 | 实际证据 |
| --- | --- | --- |
| 无成员账号可以认证，但不能取得组织context或业务权限 | **PASS** | 新增真实PG／HTTP及Edge全局登录／读取；业务入口拒绝；无成员自助成功且没有创建成员 |
| 全局注销、ban、reset后全部新旧凭据失效；成员局部撤销保留其他身份与全局认证 | **PASS** | 账号代际和全局session原子撤销；sid-less legacy access／refresh／code／device实时拒绝；同组织另一身份及其他组织保留 |
| Cookie fixation／伪造／CSRF／过期／故障有验证，敏感凭据不落日志 | **PASS** | 新增Cookie／CSRF／expiry／503用例、真实Edge属性及reload、audit敏感值检查和本次运行日志模式扫描；范围如下 |

## 已闭环的并发边界

Phase7已确认个人改密保留的在途请求语义：初始CurrentUser和旧密码验证之后，锁账号时仅比较hash；ban／kick／成员撤销若先提交，原命令仍可能完成，但不会恢复被撤凭据／成员／账号状态。本阶段已增加锁后原平台session／成员再验；新的SSO自助命令使用原global sid、账号状态及版本，不补读当前代际复活旧认证。实际账号锁等待回归验证ban、kick、具体成员撤销后的401及reset改hash后的legacy 409；无成员自助改密／重登／logout成功另有真实正控。

## 已执行与限制

台账、现有账号／凭据链与新SSO源码已完成只读安全设计审查，API／配置／表结构见 [说明](../phase-08-global-session.md)。本阶段首次集中运行见 [initial-gates.log](../../../../local-identity-sso-phase08-qa/initial-gates.log)，最终集中门禁见 [final-gates.log](../../../../local-identity-sso-phase08-qa/final-gates.log)，既有回归见 [existing-baseline.log](../../../../local-identity-sso-phase08-qa/existing-baseline.log)。以下结果均为本阶段实际执行，首次失败保留，不借用前置阶段PASS代替新增迁移后的验证。

| 验证组 | 当前实际状态 |
| --- | --- |
| Linux集中静态门禁 | **最终整条命令链exit 0**；fmt通过，check21.68s、Clippy35.98s。首次构建分别1m05s、1m18s，但初次整链后续测试失败，不记为初次整链通过。 |
| 普通测试 | **最终194项通过**：14 lib＋166 main＋14 external（4＋1＋7＋2）；main另有203 ignored。最终构建32.11s、main执行8.49s；首轮也为194项通过，构建1m47s、main7.99s。 |
| 21项新增 `phase08_` ignored首轮 | **6通过／15失败，57.14s**。legacy改密6项全部通过；SSO15项在fixture创建配置时统一报 `PUBLIC_APP_URL_INVALID`，尚未执行到各自安全断言。21次清理均 `database_removed=true,pool_closed=true`，SSO15次为 `assertions_ok=false`。修复与后续结果见下文。 |
| 15项SSO首次重跑 | **9通过／6失败，134.87s**，见 [global-retry.log](../../../../local-identity-sso-phase08-qa/global-retry.log)。6项在共享code／device撤销错误类别断言处失败；其余9项包括新增无成员改密／新密码重登／logout正控通过。15次隔离库清理均成功。该轮Clippy36.44s，test构建31.95s。 |
| 最终21项 `phase08_` ignored | **21／21 PASS，175.15s**，15项SSO＋6项legacy。最终冻结源码下全部重跑；21次隔离库均断言成功、库删除且pool关闭。 |
| 最终文档构建 | **PASS，27.02s**，生成backend crate及另外两个doc文件；前两轮命令链在测试失败处停止，不能补算为当时已执行doc。 |
| normal binary配置拒绝／正控 | **最终16／16 PASS**，见 [config-final-corrected.log](../../../../local-identity-sso-phase08-qa/config-final-corrected.log)；新增4类平台origin负例。保留初版12／12与最终binary首次13／16记录。全部停配置层、无DB连接；非法bind正控不算服务启动。 |
| 实际normal binary＋任务新库＋Edge HTTPS代理烟测 | **15项PASS**，见 [browser-result.json](../../../../local-identity-sso-phase08-qa/browser-result.json)及 [backend摘要](../../../../local-identity-sso-phase08-qa/browser-backend.log)。真实无成员登录、host-only Secure／HttpOnly Cookie、同源GET省略Origin、reload恢复CSRF、无业务权限、跨来源和缺CSRF拒绝、logout后失效均有记录。 |
| 既有后端基线 | **182／182 PASS，680.95s**，排除本阶段21项、包含Phase7新增4项，见 [existing-baseline.log](../../../../local-identity-sso-phase08-qa/existing-baseline.log)。这是新增0016迁移后的实际重跑，未借用Phase7旧结果。 |
| rollout启动／receipt基线 | **3／3 PASS，236.32s**，test构建0.87s；与上述182项在同一串行命令链，整链exit 0。三个用例分别验证development启动必须live receipt、inspect／apply／identity合同及拒绝原子性，均有owned database cleanup PASS；使用fresh任务自有库、当前16个迁移及新binary，不复用Phase6保留库／旧receipt。 |

最终集中命令由根代理在Linux环境执行，源码冻结后按下列顺序运行，整链exit 0：

```sh
cargo fmt --all -- --check &&
cargo check --locked --offline --workspace --all-targets --all-features &&
cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings &&
cargo test --locked --offline --workspace --all-targets --all-features &&
cargo test --locked --offline --bin saas-platform-backend phase08_ -- --ignored --test-threads=1 --nocapture &&
cargo doc --locked --offline --workspace --no-deps
```

既有回归按下列命令执行，两组全部结束且整链exit 0：

```sh
cargo test --locked --offline --bin saas-platform-backend -- --ignored --test-threads=1 --nocapture --skip sso_session_tests --skip legacy_password_authority_tests &&
cargo test --locked --offline --test membership_rollout -- --ignored --test-threads=1 --nocapture
```

## 首轮执行失败

首轮15项SSO失败的共同前提是测试fixture开启 `SSO_ENABLED=true`，却没有提供 `PUBLIC_APP_URL`；生产配置校验需要解析平台origin以证明认证host独立，因而先返回 `PUBLIC_APP_URL_INVALID`。这是夹具配置失败，不能写成Cookie、CSRF或撤销断言已经执行失败，也不能删除首轮记录后只留重跑结果。修复责任在测试fixture，不通过放宽生产host隔离规则消除失败。

fixture已补显式 `PUBLIC_APP_URL`，并在原无成员测试内增加改密成功、用新密码重新登录及logout成功的正控，测试总数仍为15项SSO＋6项legacy。该次重跑9项通过、6项失败；后续doc仍因命令链在测试处停止而未执行。

第二轮6项失败分别为全局logout、自助改密、ban、admin reset、kick及局部成员撤销，共同报 `revoked grant failed for a reason other than lost authority`。只读核对确认0014撤销函数将已批设备码标为 `EXPIRED`，现有OAuth设备码端点对该终态返回 `expired_token`；测试错误地将code和device统一要求为 `invalid_grant`。修复已按两种协议类别分别严格断言，并保留真实兑换正控、HTTP400及没有access token的检查；另增加 `OwnerRevoke`／`StaleAuthority` 前置数据库状态及未来expiry证明，不能以自然过期或任意失败冒充撤销生效。修正后的最终21项测试及doc已通过，集中整链exit 0。

## 最终新增测试覆盖

以下结果来自最终21项实际执行，不把测试数量解释为所有可能交错均已穷尽：

| 组 | 项数 | 已执行的核心证明 |
| --- | --- | --- |
| 无成员全局账号闭环 | 1 | 账号可认证／读取，不创建成员，不能访问平台／app业务；改密成功、旧session失效、新密码显式重登、logout成功；账号代际分别推进 |
| 全局撤销 | 5 | logout／自助改密／ban／admin reset／kick撤销两条旧SSO、平台access／refresh、人员OAuth access／refresh、app access及code／device；其他账号及机器凭据保留，成员不被改写 |
| 实时权威代际 | 1 | 故障注入恢复旧凭据物理live状态，仍因旧账号版本拒绝，不等物理回收 |
| 具体成员局部撤销 | 1 | 双向成员停用保留同组织另一身份、其他组织与全局SSO，账号版本不变；重新启用不复活旧context |
| Cookie／来源／CSRF | 2 | 属性、防固定、伪造／重复／非canonical值、一次挑战与expiry；同源GET安全metadata fallback、坏Origin不能被补救，写操作仍要求Origin |
| 权威故障 | 1 | pool关闭时五个SSO入口均503，没有账号／CSRF投影和Set-Cookie |
| 新SSO并发与期限 | 3 | 自助命令真实账号锁等待后的原sid／版本／状态再验；预验证login不能借锁等待采纳新代际；审计锁等待跨expiry时整体回滚 |
| 密码证明预算 | 1 | 旧密码输入上限、共享账号失败预算及429／Retry-After，失败不改凭据、代际、审计，不影响其他账号 |
| legacy个人改密与通用reset | 6 | ban／kick／具体成员撤销先提交时401；管理员reset保留409；其他成员撤销时有效context正控；disabled账号仍可受授权admin reset |

本组全局撤销直接断言的是profile／userinfo／refresh／ticket／code／device等消费路径，未另写人员／app introspection的否定断言；机器introspection有保留有效正控。故障行仅指本阶段五个SSO入口，其他授权入口故障证据需结合既有基线。业务凭据通过账号代际关联全局撤销，本阶段没有从SSO派生业务context，不能声称已验证逐global sid的业务联动。

## normal binary配置与真实浏览器

初版 [config-check.log](../../../../local-identity-sso-phase08-qa/config-check.log) 的12项覆盖默认关闭、合法独立HTTPS origin两个正控，以及缺失origin、HTTP、带path、多origin、同host不同port、非法enable值、session TTL零／超上限及challenge TTL低于／高于界限的拒绝。开启时必须同时具备合法 `PUBLIC_APP_URL`，认证host与平台host不同。

第二轮后另有只读配置审查修正：开启SSO时只对 `PUBLIC_APP_URL` 调用通用URL parser还不足以证明它是平台origin，`mailto:`／`data:` 等可解析URL可能没有host。当前源码已复用既有origin解析器，强制单一精确HTTP(S) origin并拒绝凭据／path／query／空值，再比较host；SSO关闭时legacy配置语义保留。最终normal binary增补非HTTP scheme、含凭据、path、空平台origin四类负例，总数16。

最终binary首次配置运行见 [config-final.log](../../../../local-identity-sso-phase08-qa/config-final.log)：13通过／3失败；三个畸形平台URL用例被由平台URL派生的CORS配置先拒绝，实际拒绝类别与预期SSO层不符。仅在QA环境显式指定合法 `CORS_ALLOWED_ORIGINS`，并保持合法 `MICRO_GATEWAY_ORIGIN`，修正后 [16／16通过](../../../../local-identity-sso-phase08-qa/config-final-corrected.log)。没有放宽生产校验；全部用例均未连接数据库，正控通过预置非法bind终止。

浏览器首次driver执行见 [browser-backend-first.log](../../../../local-identity-sso-phase08-qa/browser-backend-first.log)，因参数namespace缺 `expected_cluster_system_identifier` 在数据库连接／创建之前报错；修正QA参数后使用新UUID重试。它不是服务／Cookie失败，也不能遗漏该首次运行记录。

最终使用Microsoft Edge **142.0.3595.94**、Playwright **1.64.1**，以实际normal backend binary、任务自有新库及隔离HTTPS代理完成15项检查，详见 [browser-check.log](../../../../local-identity-sso-phase08-qa/browser-check.log)：

| 实际请求／检查 | 结果与意义 |
| --- | --- |
| 同源GET preauth／session | 无Origin，`Sec-Fetch-Site: same-origin` 且实际Host匹配，均200；session请求发生于页面reload之后并恢复CSRF |
| 精确Origin的login | 200，无成员真实密码认证；响应no-store、清除preauth Cookie |
| 浏览器保存及脚本可见性 | session Cookie为Secure、HttpOnly、host-only；页面JS不能读取该Cookie，运行于真实secure context |
| 只有全局Cookie请求平台profile | 401，全局认证不能直接充当平台业务凭据 |
| 缺CSRF的logout／错误Origin的preauth | 均403；真实写请求有精确Origin，跨来源请求的Origin不匹配时拒绝 |
| 用reload恢复的CSRF logout，再GET session | logout200并清Cookie，随后认证态401；没有页面异常 |

TLS仅由该隔离浏览器context接受临时自签证书；没有修改系统信任，也不将该运行视为生产TLS验收。HTTPS loopback桥接退出0。

本次binary SHA-256为 `370b2fdb5b49fcb2cdee8b6d8ff914f4dcf4056f090e65fda9ee8d7c7f7c4a26`。backend摘要证明memberless仍为true、全局session共1／active0，成功审计 `SSO_LOGIN=1,SSO_LOGOUT=1`；backend退出0，删除前连接0，任务库已删除。没有复用Phase6旧receipt或改动生产数据库。

driver对本次完整捕获的backend stdout **5555字节**、stderr **0字节**实时内存扫描，PHC、认证Cookie、JWT、CSRF、password body及已知fixture密码六类均无匹配；仅保留字节数、SHA-256与命中类别，没有输出原运行日志。它与新增测试对 `audit_logs` 的敏感值断言共同提供本次fixture的保密证据，不能扩大为任何环境或未知秘密模式均无泄漏的保证。

## 源码审查发现及修正记录

以下三项在首次集中门禁之前通过设计／代码审查发现，当时没有执行对应失败测试。后续真实回归结果见上表，不把发现方式改写成运行失败：

- **同源GET的Origin缺失。** 浏览器通常不为同源GET发送Origin，且脚本不能自行设置此请求头。实现已增加限定的Fetch Metadata＋实际Host分支，坏／重复Origin不能被fallback补救，写操作仍强制精确Origin＋CSRF；后续真实Edge已验证reload后的认证态与CSRF恢复。
- **有效Cookie不能绕过旧密码失败预算。** 新SSO个人改密最初没有接入LoginLimits，已补来源及 `user:{id}` 共享预算、输入上限、验证deadline、错误旧密码记失败、429与成功提交后才clear。该修正没有扩改旧legacy个人改密的失败预算，不笼统称所有密码入口已统一改造。
- **审计等待后跨绝对期限提交。** 仅在账号锁后证明session有效，仍可能在后续审计／凭据锁等待中越过期限。新SSO登录及自助命令已增加数据库 `clock_timestamp()` 提交前截止时间检查，失败回滚账号／凭据及审计。不能据此声称旧legacy JWT自身exp的在途窗口也已统一修复；legacy本阶段补的是原平台session／成员锁后再验及管理员reset的hash冲突优先。
