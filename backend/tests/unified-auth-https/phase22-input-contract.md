# Phase 22 私有负载输入合同

`phase22-fixture.py` 只准备测试输入，不启动环境、不运行压力或故障，不写总台账，不修改 common prepare/environment/gateway、签名密钥、provider settings 或 schema。环境执行者拥有所有启动、配置合并、实际 HTTPS 结果与 finally 清理。

脚本只接受固定任务目录、`OWNER`、loopback 55494、集群 `7701548153213487841`、新 canonical 任务库、匹配 UUIDv7 deployment comment 和数据库/角色前缀。每次事务再次检查 database/schema owner、deployment role closure、六个角色的登录/非提升属性与无 membership、所写表的 identity owner，以及唯一成功的基线版本 1。runtime URL 必须为该任务的真正 identity LOGIN 角色。任何不一致都拒绝，不能用于旧业务库。

## 调用顺序

```sh
python3 phase22-fixture.py --task-dir "$TASK_DIR" --admin-url "$TASK_ADMIN_URL" --stage prepare
# 环境执行者把私有 phase22-fixture.json 的 identityOverrides 合并到该次身份配置，
# 再启动相同冻结源码的实例并等待实际 HTTPS health。
python3 phase22-fixture.py --task-dir "$TASK_DIR" --admin-url "$TASK_ADMIN_URL" --stage codes
# 立即先跑 CODE：code 的有效期始终为 60 秒。
```

prepare 默认 1200 个 code 输入名额与 8 个 refresh family，可用 `--code-sources 750..10000`、`--refresh-sources 1..32` 调整。prepare 阶段拒绝已有 service PID、Phase 22 JSON、grant 或 code；codes 阶段允许已启动的本任务服务，但只生成首批 code，拒绝再次发行、覆盖或延长原 code。阶段在写库前保存 SEEDING 状态；失败或未知结果由环境执行者核查并清理，不能盲重放。

所有输出均以 `O_EXCL`/`O_NOFOLLOW` 创建为 **0600**，只在任务目录内；manifest 替换使用同目录 0600 临时文件。不会向 stdout、正式报告或命令参数输出密码、token、Cookie、客户端 ID、账号 ID、SQL 或连接串。日志只含固定阶段和计数。

## 私有文件与 driver 映射

| 文件或字段 | 用途 |
| --- | --- |
| `phase22-fixture.json` | 任务绑定、provenance、identityOverrides、私有账号/来源引用；含秘密，禁止归档到正式报告。 |
| `phase22-user-client.json` | driver 的 CODE/REFRESH `clientFile`；精确 `{clientId,clientSecret}`。 |
| `phase22-machine-client.json` | INTROSPECTION `clientFile`；精确 `{clientId,clientSecret}`，也是正常 client_credentials mint 使用的独立凭据。 |
| `phase22-code-sources.json` | `{codes:[{code,verifier,redirectUri}]}`；只在 codes 阶段产生，逐个一次性消费。 |
| `phase22-refresh-sources.json` | `{refreshTokens:[...]}`；并发 worker 数须等于条目数，默认 8；每条属于独立 family。 |
| `phase22-introspection-sources.json` | `{tokens:[...]}`；初始为明确 owner SQL seed 的 MACHINE token。建议实际测量前通过正常 client_credentials 真实 mint 并由环境执行者安全替换该文件，更新 provenance。 |
| `phase22-ca.pem` | 已准备环境的公开 CA 证书副本，0600，供严格 TLS driver 使用；不复制私钥。 |

driver 输入仍严格遵循 [phase22-load.mjs](phase22-load.mjs) 的 `identity-capacity-input/1`，由环境执行者另写 0600 JSON：`kind`、1–2 个准确 issuer `origins`、rate/concurrency/duration/requestTimeout、以及上述绝对 client/source/CA 路径。短时计划为 CODE **25 RPS × 30s / concurrency 8**、REFRESH **25 RPS × 120s / concurrency 8**、INTROSPECTION **200 RPS × 120s / concurrency 16**；只是输入计划，不能提前写成实测结果或 30 分钟稳定性通过。

USER 客户端仅 CONFIDENTIAL / WEB_BFF / USER / ACCOUNT_ONLY、`openid profile`，授权来源为明确 owner SQL fixture：30 分钟 SSO、consent、grant、完整 scope 映射、rp sid/sub、refresh family 和 access；family deadline 不超过原 grant。code fixture 保持原 session/account/policy/consent、S256、nonce/state 摘要和 60 秒期限，**不代表实际浏览器授权/consent**。MACHINE 客户端独立为 SERVICE / MACHINE / NONE，只允许自定义资源的 `phase22.read`，没有 openid/profile/userinfo、人员、consent 或 SSO 字段。初始 machine access 为 5 分钟，可重用观察 active introspection；真实 mint 使用 `scope=phase22.read`，实际 token TTL 配置为 300 秒，测量前核对剩余期限。

`identityOverrides` 显式使用 source/client 额度各 1,000,000、精确可信代理 `127.0.0.1`、独立随机 metrics token、数据库池 4、acquire/statement/lock 各 300ms、密码执行 1、排队 1、queue/execution 各 100ms、总验证预算 300ms。它们只属于本次 fixture，脚本不自动回写环境文件。`passwordProbe` 提供单独纯账号和 **Argon2id m=65536,t=3,p=1** 的合法高成本 hash 输入，用于真实 preauth/并发 login 观察队列；不改变生产密码政策，正常 SSO 基线使用 prepare 的 employee。已有 fixture Cookie/来源和账号元数据均保密。

当前 `phase22-run.py` 先采用生产默认的 execution／总验证各 10000ms 完成真实正常登录；`phase22-runtime.mjs` 在本就需要的 Identity 故障重启时，再将该专属实例切为 execution 100ms、queue 500ms、执行 1／排队 1 的压力配置。500ms 排队窗口用于实际 scrape 捕获队列，生产默认值没有变化。驱动省略空 Cookie 头，并遵循 Set-Cookie 的 Max-Age 清理，保持浏览器 Cookie 语义。

完整运行使用 `python3 phase22-run.py`（WSL root，仅 namespace 控制；应用、Node 和夹具均降权至 OWNER 用户）。`--faults-only` 使用新的受控库，只复用已经完成的脱敏容量组件；运行时验证当前 Identity 二进制摘要和三类完整时长，并明确记录独立 fixture／没有新压力请求。旧 code／refresh 不会重新发行或重放。所有模式都由 runner 的 finally 完整清理。

## 终态告警输入与清理

环境执行者需要告警输入时可单次运行 `--stage terminal`。它新增一个仅属于本 fixture 的 owner SECURITY target 和**合成 MFA 通知 source**，让 canonical trigger 产生新 job，再在同一事务中只把该新 PENDING job 置为真实数据库 `TERMINAL`。不创建或启用真实 MFA 因子；不把 source 命名视为实际账号变更，也不声称 worker 在该次执行过 HTTP。提交前已为终态，worker 不会外发该合成消息。

该阶段不会重置、删除、回滚或重开任何已有 terminal，重复调用拒绝；canonical trigger 同步 source terminal 状态。它只支持实际 metrics→FIRING→ACKNOWLEDGED 演练，ACK 不等于投递恢复或 terminal 消失，环境执行者需如实记录。

成功、失败或中断都由环境执行者 finally 停止所属进程，以原 manifest 执行 `prepare.py --cleanup`，复核任务库/角色残留 0，再定向删除任务内 `phase22-*` 私密文件和所属运行日志。脚本不创建容器，不执行全局 Docker prune，不清其他项目、共享缓存、源码或正式报告。语法与纯内存构造检查通过不等于实际 SQL、HTTPS 或容量通过；实际运行由主流程报告。
