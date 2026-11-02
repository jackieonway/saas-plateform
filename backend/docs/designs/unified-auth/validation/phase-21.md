# Phase 21：独立第三方标准 OIDC 接入示例

日期：2026-10-28。状态：COMPLETED（追加 P2 修复与重新验收完成）。来源账本容量满时本地登出未失败关闭的问题已通过 HTTP RED 复现、最小修复、46 项组件回归、独立复审及最终快照完整实际 HTTPS 15 项复测。新旧实际证据分别保留准确 SHA，互不替代、不合并为一次通过数；本阶段私密运行资源已清理，TLS lease 已交还 Phase 22。

## 输入与交付边界

本阶段依赖 Phase 8、9、10、11、19、20，开始实际实施前已由主代理确认全部门禁完成。执行前 `saas-examples` HEAD 为 `f22ee4f0fe601b94a0f38783d3a76ef113ee47b0`，工作树干净。平台 HEAD 为 `34105757b040165daa5477bf6afff9637f617466`、身份 HEAD 为 `7025a0a39cff5577bc3aabf239aaae8df529714e`，已有前阶段未提交改动均保留；[工作树清单](phase-21-worktree-inputs.json)明确是交付时盘点，运行输入另按冻结 SHA 验证。

实际权威使用主代理已完成 Phase 20 的 `source-snapshot-phase20-freshness-r2` 身份二进制；平台资源拒绝检查与 migration／角色 SQL 使用 `source-snapshot-phase20-final-r1`。没有把并行 Phase 22 的身份监控改动混入互操作输入。本阶段不修改身份／平台产品代码、canonical SQL、旧协议退役范围或总台账。

- [独立 Node BFF 与机器 CLI](../../../../../../saas-examples/oidc-rp/README.md)，锁定 `openid-client 6.8.8`、`jose 6.2.13`，传递库 `oauth4webapi 3.8.8`；[package-lock](../../../../../../saas-examples/oidc-rp/package-lock.json)固定完整依赖和 integrity。
- [第三方接入指南](../third-party-oidc-integration.md)，说明公开配置、账号来源、双控制权关联、错误／撤销处理及标准与扩展登出边界。示例根 README 增加入口；CI 新增独立 `independent-oidc` job。
- 独立 [fixture owner 制备](../../../../tests/unified-auth-https/phase21-fixture.py)、[TLS gateway](../../../../tests/unified-auth-https/phase21-gateway.py)、[RP supervisor](../../../../tests/unified-auth-https/phase21-run-rp.py)、[标准协议 driver](../../../../tests/unified-auth-https/phase21-protocol.mjs) 和 [真实浏览器矩阵](../../../../tests/unified-auth-https/phase21-browser.mjs)。这些验证工具不进入独立示例的运行依赖。

## 示例行为与限制

BFF 只接收外部可获得的精确 HTTPS issuer、客户端标识、自己的秘密文件、RP origin 和 CA。Discovery 使用显式 `ClientSecretBasic`；`enableNonRepudiationChecks`、S256、state、nonce、授权响应 issuer、必须存在的 ID Token、RS256／JWKS、期限与 UserInfo subject 一致性均启用。ID Token 不作为 API bearer；令牌、code、verifier 与完整上游正文不返回浏览器或日志。客户端标识、sub、sid 均不解析为内部 UUID；sid 按有界 opaque 字符串精确比较，当前权威产生的 hex 形式只是其当前产物。

会话采用独立 `__Host-oidc_rp` Secure／HttpOnly／SameSite=Lax Cookie，写入要求准确 Origin 和 CSRF。单进程内存有界，本地会话最多 5 分钟，refresh 单消费者轮换且不延长本地截止；UserInfo 动作每次在线检查，普通会话视图至多 60 秒复核。callback、refresh 和网络等待后重新检查原来源和本地截止；签名未知 sid 通知保留 tombstone，不能被迟到回调复活。跨副本／跨重启持久恢复不在此示例中宣称完成。

已验证来源的本地关闭先于来源／重放账本容量检查，原 sid/sub 的所有本地会话同时关闭并推进 generation；无法记录事实时返回明确 503，不伪报 ACK、不扩容或驱逐未过期事实。来源事实满时使用一个有截止的 overflow 时间戳，阻止等待中的新 code 结果挂载；已有无关来源的读取和保持原来源的 refresh 不被无差别清除。原 operation 可继续查询，结果恢复不回退本地关闭。

RFC 7009 撤销先关闭本地原来源，再撤销 refresh family／Access Token；失败时仍本地关闭，不自动重耗不确定凭据。back-channel 是标准签名接收协议。CURRENT_APP／GLOBAL 明确作为已声明的 `rp_logout_command_endpoint` 命令扩展，保留原 operation／token／sid／sub；结果未知查询同一 operation，只有明确 `404 NOT_FOUND` 才同号再提交。当前 Discovery 没有 `end_session_endpoint`，没有将该命令冒充标准浏览器 RP-Initiated Logout。

已有账号关联必须证明本地账号与当前 OIDC 身份的双方控制权，不能按相同邮箱自动合并。指南定义需近期本地重新认证、独立 OIDC 校验、当前浏览器和一次性关联操作。样例没有虚构一个本地认证系统：缺少完整双证明的 `/api/account/link` 固定返回 `409 EXPLICIT_DUAL_CONTROL_LINK_REQUIRED`，无自动合并出口。

现行 THIRD_PARTY 审核只允许 USER／ACCOUNT_ONLY／`openid profile`。实际第三方测试证明 offline_access、业务 scope 与 client_credentials 均被拒绝。离线与机器正向使用**另行获批的 FIRST_PARTY fixture 配置**，通过相同标准客户端和公开协议验证；这不改变第三方默认权限，也不表示运营方已批准对外提供这些资格。

## 冻结输入与实际 HTTPS

[原运行证明](phase-21-runtime-attestation.json)记录身份／平台／migration 二进制、canonical SQL、全部 14 项 RP runtime／配置、依赖版本、验证工具与原结果 SHA。原 RP source manifest SHA 为 `791bb5b83fab6ed06f031fe48e6cc4e5fa9c9fa4eb16141b09995126c91e64b0`；它核对的是当次源码和私有运行副本，不作为容量修复后的当前源码证明。Node 为任务私有的官方 Linux `24.22.1`，下载体在内存中以官方 SHASUMS256 校验，SHA 为 `6c172a9ab6718f7b801fb5a7bd486af2ee5e32758086a04b84fcce2aa77095e6`，未全局安装，也未留下压缩包。

实际使用新 canonical 数据库与真实 runtime 角色，身份、平台资源服务、独立 Linux Node RP 经严格 CA gateway 服务；Windows Chromium 仅使用受控 CONNECT 传输，不忽略 TLS 错误。公开 RP origin `customer.saas.test` 是本次独立示例的隔离 fixture 域名，不运行门户产品或继承门户资格。标准客户端运行环境没有 DATABASE_URL、内部服务秘密、平台账号 UUID 或成员证明；[owner 制备工具](../../../../tests/unified-auth-https/phase21-fixture.py)单独登记准确授权事实。两个测试账号都无 memberships。

[原实际 HTTPS 报告](phase-21-https-browser.json)：**15/15 PASS，TLS 1.3**，仅属于原 `791bb5…` RP 快照。下表场景随后以修后最终 `72e685…` 快照完整重跑，[新实际 HTTPS 报告](phase-21-capacity-https-browser.json)也是 **15/15 PASS，TLS 1.3**；两次都保留独立结果及运行证明。

| 实际场景 | 结果 |
| --- | --- |
| 独立桌面／390px 页面、精确 Discovery 和明确非 RP-Initiated Logout 的能力声明 | PASS |
| 无成员账号真实密码／SSO／同意／code＋S256 回调，ID Token 与 UserInfo 一致，refresh 轮换 | PASS |
| 修改实际回调 state、响应 issuer；修改授权请求 nonce、PKCE challenge | 四项分别 PASS，全部拒绝且不建立本地会话 |
| 关联已有账号的自动合并拒绝、缺 CSRF 拒绝 | PASS |
| 默认第三方 offline_access／demo.read 请求直接 400，USER 凭据不能取得 MACHINE grant | PASS |
| userinfo Access Token／ID Token 不能调用已登记 demo 资源；实际平台 `/api/external/menus` 拒绝未批准业务访问 | PASS |
| RFC 7009 撤销后 UserInfo／refresh 拒绝；refresh 重用导致 family 失效 | PASS |
| 单独获批 MACHINE 经标准 CLI 取得 demo.read 并通过 HTTPS 资源 200，demo.admin 与 UserInfo 人员身份被拒绝 | PASS |
| CURRENT_APP 只失效原 grant，另一 grant 可用、同意保留，实际 outbox 投递 DELIVERED | PASS |
| 实际 JWKS 下 fixture 签名未知 sid 接收；同 jti 重放 409、错 aud／坏签名拒绝 | PASS；签名负向材料仅在测试内存中 |
| 单独获批离线配置实际 7 天／24 小时上限；自然 SSO 到期仍可刷新，准确 owner 截止缩短后 idle／absolute 真实 HTTPS 拒绝 | PASS |
| GLOBAL 从另一个标准协议 grant 发起，真实 worker 关闭独立 RP 会话、投递 DELIVERED、同意保留 | PASS |

GLOBAL 的 RP 本地会话在通知到达前刚建立，尚未到普通会话视图的 60 秒复核窗口；观察到的本地关闭因此不能以轮询 UserInfo 代替 back-channel 证明。离线截止测试明确由 owner 只缩短本库准确 fixture 来源／family 的时间，不伪称实际等待了 7 天。身份／API／签名校验与 refresh 拒绝仍经过实际 HTTPS 服务。

桌面和移动截图已视觉检查，无横向溢出：[桌面](phase-21-rp-desktop.png)、[移动](phase-21-rp-mobile.png)。截图为匿名独立页面，不含登录输入、Cookie、token 或回调查询。

## 组件门禁与修正记录

| 层级／命令 | 结果与证据限制 |
| --- | --- |
| `npm --prefix oidc-rp test` | 修后 **46/46 PASS，13.612 秒**，覆盖原 35 项和新增 11 项容量／并发 HTTP 回归；真实本地 TLS OP／RP 和 CLI 资源 fixture，不能冒充实际权威 HTTPS 的 15 项 |
| `npm --prefix oidc-rp run check` | PASS，运行时、页面脚本与测试 JavaScript 语法 |
| `npm --prefix oidc-rp audit --omit=dev` | 0 vulnerabilities；三项直接／传递库均 MIT |
| 两个受影响仓库 `git diff --check`、当前文档相对链接 | PASS；不运行未改 Rust 产品的重复 Cargo 全套 |
| 实际 INFO 日志脱敏 | [四份日志、0 发现](phase-21-log-redaction.json)；检查已知私有值和 credential／回调查询模式，原日志随后删除 |

## 追加容量审查与最终快照重新验收

独立审查 `/root/phase24_inventory` 发现原 `closeSource` 在 sources 达到 `maxEntries * 4` 时先抛 `SOURCE_CAPACITY`，没有清理本地 token；登出回包 503 后 `/api/session` 仍 authenticated、`/api/userinfo` 仍可使用旧来源。这是实际 P2，原阶段因此重开。

[修复组件记录](phase-21-capacity-components.json)记录先执行 HTTP RED：maxEntries=1、四个有效未知 sid 通知填满账本后，CURRENT_APP 与 GLOBAL 两例均以 `503 SOURCE_CAPACITY` 回包，但 `authenticated=true` 导致 `CAPACITY_FAILURE_MUST_CLOSE_LOCAL_AUTHENTICATION` 断言失败，**0/2 PASS、2 FAIL**。测试和临时 TLS peers 均 finally 清理，没有用纯内存复现替代 HTTP 失败证据。

[生产补丁](phase-21-capacity-production.patch)只改变 RP 的 `src/store.mjs` 与 `src/app.mjs`：

- 同步关闭准确 sid/sub 的所有本地 session、清除其待登录 flow 并推进 generation，再尝试记录来源事实；原 pending logout operation 保留，容量错误之后仍能按原编号恢复查询。
- 来源满时不增加记录、不驱逐未过期事实，用单个 `sourceOverflowUntil` 阻止未知来源的迟到 code 挂载；截止为最后一次容量失败加 `state TTL + session TTL + 120 秒`。这不是新增队列或多实例要求。
- 同 jti 重放仍先返回 409；新鲜有效通知遇到 replay 容量满时，先关闭／记录准确来源，再返回 503，不能伪报 ACK。
- callback 在网络等待前捕获原 generation，并在 rotate 时复验，防原来源被关闭后不同 sid 的晚到 code 结果覆盖同一个本地浏览器会话。refresh 保留原 generation／来源栅栏；无关来源的稳定读取与 refresh 继续可用，subject 不符仍在变更前拒绝。

新增 [11 项真实本地 TLS HTTP 检查](../../../../../../saas-examples/oidc-rp/tests/capacity.test.mjs)覆盖两种退出范围、同来源多会话 logout／RFC 7009、本地关闭后的 UserInfo／refresh 拒绝、原 operation 恢复、source／replay 满的签名接收、未知 sid 等待中的 callback、等待中的 refresh、同 cookie 新 code 的 generation 栅栏，以及不影响无关来源。定向 **11/11 PASS**；随后完整 **46/46 PASS、0 skipped**，语法和 diff 通过。

独立审查重新核对最终 store `0ea97aa7378c2ae368084acff4d100c63ccfb2ff783e5f011f018bc79aa06e70`、app `621129fc0838b3adc80204c2e03f29666e55dbaa85a19edc8c3b5dbb013e0109` 与新增 HTTP tests SHA，并在最终生产代码上重新执行原纯内存 repro：本地关闭、pending 保留、flow 清除、generation 推进、旧 generation 拒绝、未知来源 overflow 拒绝和 source 数量不扩大均成立。审查结论无新增 P1/P2；审查者只读代码和纯内存检查，没有冒称重跑全组件或使用数据库／TLS 环境。

与 Phase 22 owner 协调独占短窗口后，以同一已核验的 Phase 20 Rust 产品二进制、新 canonical 库／六角色和最终 RP 快照运行完整实际矩阵；没有重建或混用 Phase 22 产品、数据库或环境。[最终运行证明](phase-21-capacity-runtime-attestation.json)的 RP source SHA 为 `72e685c26f4aae783c8b0f16e9c5e0ce692cd86fa0782f912ca5250a13d3fc0a`，逐文件与当前源码、实际 Linux 运行副本一致。[最终实际 15 项](phase-21-capacity-https-browser.json)全通过；[本轮日志脱敏](phase-21-capacity-log-redaction.json)四份 INFO 日志 0 发现；[本轮清理](phase-21-capacity-cleanup.json)全部通过。

这一轮另保留一份 [deadline harness FAILED](phase-21-capacity-https-browser-red-deadline.json)：前 13 项已通过，新的纯账号 SSO 不足一秒，owner fixture 的 `expires_at=clock_timestamp()-1 second` 触发原有 `expires_at>created_at` CHECK。只读确认期限变更均回滚后，改为当前 statement 的 `clock_timestamp()`；下一实际 HTTPS 请求观察其自然到期，仍保留原 created_at／来源绑定，不用 sleep 或放宽结构。随后在同库、同实例重新完整通过 15 项。该失败不计入通过数。

新截图另存为 [修后桌面](phase-21-capacity-rp-desktop.png)／[修后移动](phase-21-capacity-rp-mobile.png)，原结果、原运行证明和原截图 SHA 均未覆盖。

真实互操作暴露并修复了[原生 HTTPS 传输](../../../../../../saas-examples/oidc-rp/src/transport.mjs)的缺陷：未声明 body 长度时 Node 使用 chunked，严格 gateway 返回空 400，标准库报告 `OAUTH_RESPONSE_IS_NOT_JSON`。修复按 UTF-8 字节数声明 Content-Length，并拒绝冲突长度或调用方 Transfer-Encoding。组件先用拒绝 chunked 的严格 TLS fixture 复现失败，再验证修后通过。没有放宽 gateway、JSON、签名、issuer、nonce 或 PKCE 校验。

保留的失败尝试均为 FAILED，不纳入 15 项通过数：

- [red-1](phase-21-https-browser-red-1.json)／[red-2](phase-21-https-browser-red-2.json)：owner fixture 的 consent group purpose 与 application purpose 不一致，权威正确拒绝。确认这两个准确 client 尚无 grant 后，在同一 owned 库对齐 purpose，并修正制备源码。
- [red-3](phase-21-https-browser-red-3.json)／[red-4](phase-21-https-browser-red-4.json)：真实纯账号登录／同意已成功，原客户端传输 framing 导致 callback 400；red-4 取得安全标准库错误码。旧 RP 输入见 [r1 manifest](phase-21-rp-r1-attestation.json)，修后改用最终冻结副本重新执行全矩阵。
- [red-5](phase-21-https-browser-red-5.json)：已有 7 项实际检查通过，harness 对未批准 scope 只等待错误 redirect；权威实际直接返回 400。改为断言实际拒绝行为，随后 15 项全部通过。

启动前另有 gateway 未转发精确私有 readiness 而导致平台失败关闭的传输准备尝试，未记为产品通过。最终 gateway 私有端口只沿用原精确 allowlist；RP 配置和所有标准协议请求只使用公开 443。

## 复现入口与资源生命周期

使用新任务 fixture，不接管旧数据库、不改 `_sqlx_migrations` 或 checksum。实际命令分为以下顺序；`TASK_DIR` 是主代理已核验的任务根，`ADMIN_URL` 仅在受保护 shell 内从 `admin-url` 读取，不能回显或写入报告。

```sh
python3 backend/tests/unified-auth-https/prepare.py \
  --task-dir "$TASK_DIR" --admin-url "$ADMIN_URL" \
  --cluster-id 7701548153213487841 \
  --migration-bin "$TASK_DIR/bin/platform-migrate" \
  --backend-source "$TASK_DIR/source-snapshot-phase20-final-r1/saas-plateform/backend"
python3 backend/tests/unified-auth-https/phase21-fixture.py \
  --task-dir "$TASK_DIR" --admin-url "$ADMIN_URL"
```

RP runtime 仅复制运行证明中的白名单文件，使用其 package-lock 在任务私有副本 `npm ci --ignore-scripts --no-audit --no-fund`。冻结工具目录中，原 `environment.sh`／`run-service.py` 保留原名；原 gateway 复制为 `base-gateway.py`，本阶段 gateway 复制为 `gateway.py`，并复制本阶段 RP supervisor／protocol driver。所有准备操作先验证 OWNER、无活动 lease 和输入 SHA。

namespace controller、identity、platform 仍使用既有 `environment.sh start/run`，设置 `TASK_TOOL_DIR` 为该冻结目录；RP supervisor 通过原 namespace 的 `nsenter --mount` 启动。浏览器公开 HTTPS 使用既有 CONNECT proxy，执行：

```powershell
node backend/tests/unified-auth-https/phase21-browser.mjs `
  --output <绝对验证目录>/phase-21-https-browser.json
```

浏览器 harness 在 finally 关闭 contexts、Chromium 和标准协议／测试资源子进程；RP supervisor 与 namespace controller 在 finally 停子进程并删除 lease。环境执行者最后停止 namespace，再用同一 prepare.py 的 `--cleanup` 验证 manifest／comment 后删除精确数据库及角色，定向删除私密材料与日志。失败／中断同样执行这条清理路径。保留的旧 FAILED JSON 与匿名截图是正式验证记录，不作为临时日志删除。

## 清理与剩余共享工具

[原定向清理记录](phase-21-cleanup.json)证明原 canonical 库 `17,438,399` 字节及 6 个角色已删除；[追加复核清理](phase-21-capacity-cleanup.json)证明另一份新 canonical 库 `17,495,743` 字节及 6 个角色同样删除，两轮严格串行、没有接管旧库。精确数据库／角色剩余均为 0。身份、平台、独立 Linux RP、测试资源、namespace、gateway、Windows CONNECT proxy 和 lease 全部结束。两轮各四份 INFO 日志完成脱敏后删除；ENV、签名／BFF／context key、四份客户端 secret、公开配置的私有副本、临时 hosts／controller 和无用 cache 均清除，没有持久测试 session 或压缩包。

创建容器 **0**、复用容器 **0**；Docker 清理前后 images／containers／volumes／build cache 均为 0。原轮 `/tmp` 从 `1,054,187,520` 字节降至 `1,036,455,936` 字节，释放 `17,731,584` 字节。原 PostgreSQL 5432 可用，Windows 原 443 PID 5044、通知 PID 24971／启动时 279512 保持；共享任务 PostgreSQL 55494、根 OWNER、其他阶段缓存与源码不动。未停止 WSL／Docker，未压缩 VHDX，不将 Linux 清理等同 Windows 物理空间已完全恢复。

追加复核清理的 `/tmp` 观察值为 `1,054,076,928` → `1,036,447,744` 字节，减少 `17,629,184` 字节；同样不将这项 Linux 观察值表述为 Windows VHDX 缩小。最终端口 lease 已明确交还 Phase 22 owner。

主代理明确要求保留以下**纯公开共享工具供 Phase 23／25 复用**，并由整个 goal 结束时统一清理；这里不声称它们已经删除：

| 任务根下保留项 | 字节 | 内容 |
| --- | --- | --- |
| `phase21-node-runtime` | 201,760,666 | 已核验官方 Node／npm，没有秘密或压缩包 |
| `phase21-rp-source` | 815,087 | 容量修复后最终白名单源码、示例配置、锁定公共依赖，没有部署 ENV／secret |
| `phase21-tools` | 40,730 | 已冻结公开验证工具，没有运行日志、lease 或无用 Python bytecode cache |
| `phase21-rp-attestation.json` | 1,370 | 公开源文件 SHA manifest |

源码仓库中新建示例的 node_modules 只含锁定公共依赖，保留作常规开发依赖；没有清空已有全局 npm／Cargo 缓存。正式 JSON、匿名截图与源码均保留。

## Self-Check

| 台账验收条件 | 结果 | 当前证据 |
| --- | --- | --- |
| 无业务成员账号完成纯账号登录，ID Token 与 UserInfo 身份一致 | PASS | 新库无 memberships、实际密码／SSO／同意／标准库回调及 UserInfo 一致 |
| state／nonce／issuer／PKCE 错误、撤销／离线期限／登出有实际证据 | PASS | 实际 HTTPS 分别拒绝四种篡改；RFC 7009、refresh replay、另行获准离线截止、CURRENT_APP／GLOBAL 及实际 worker 通知 |
| 第三方已有账号关联要求双方控制权，不按相同邮箱自动合并 | PASS | 指南明确双证明、原浏览器和一次性操作；样例对无完整双证明的关联固定 409，没有伪造本地认证 |
| 示例无法越过受众／scope 或访问未批准业务资源 | PASS | 默认第三方扩权 400、USER→MACHINE 拒绝、wrong audience／ID Token→API 拒绝、实际平台业务 API 拒绝；独立获批 MACHINE 只通过 demo.read |

追加 P2 的修后组件、独立复审、最终快照完整实际 HTTPS 和清理门禁均已闭合，建议主代理重新标记 Phase 21 COMPLETED。本组件不改总台账、不替代 Phase 22 监控或 Phase 26 真实发布事实。
