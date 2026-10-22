# 登录保护与来源地址

登录入口和历史 OAuth password 流程使用进程内有界准入及统一密码服务。实现分别在 src/login_limits.rs、src/util/password.rs、src/client_ip.rs，参数在 src/config.rs 校验。

## 关键参数

| 参数 | 默认 | 作用 |
| --- | ---: | --- |
| PASSWORD_CONCURRENCY | 2 | 同时执行的密码任务 |
| PASSWORD_QUEUE_CAPACITY | 8 | 额外等待名额 |
| PASSWORD_QUEUE_TIMEOUT_MS | 250 | 等待许可上限 |
| PASSWORD_EXECUTION_TIMEOUT_MS | 10000 | 调用者等待已启动任务的期限 |
| LOGIN_MAX_ENTRIES | 4096 | 来源、输入、账号共享条目上限 |
| LOGIN_WINDOW_SECONDS | 600 | 固定准入窗口 |
| LOGIN_SOURCE_REQUESTS | 60 | 每来源每窗口准入次数 |
| LOGIN_USER_FAILURES | 5 | 用户失败限制 |
| LOGIN_IDENTIFIER_MAX_BYTES | 320 | 登录标识输入上限 |
| LOGIN_VERIFY_BUDGET_MS | 10000 | 共享别名候选验密总预算 |

密码与 PHC 输入有独立长度限制，非法或损坏 PHC 不算成功。新密码使用既有 Argon2 策略，生产写入还校验长度和弱密码。超时不等于正在执行的密码计算已终止，执行许可按真实任务生命周期回收。

预算只在本进程生效，重启会丢失本地计数；多副本各有独立额度，不能当作全局限流。Compose 未透传的参数需通过明确的服务配置/override 注入，宿主环境有值不代表容器已采用。

## 代理身份

TRUSTED_PROXY_IPS 只接受精确 IP。非可信 socket peer 的代理头被忽略；可信 peer 的 X-Forwarded-For 从右向左剥离可信链，停在最近非可信地址。缺少 XFF 时才考虑可信 peer 写入的单个 X-Real-IP；非法、重复或过长链回退真实 peer。

中间件统一规范化来源并清理外部代理头。实际网关需受控追加/覆盖来源，API 私网不得让外部伪造可信 peer。部署时将 GATEWAY_PRIVATE_IP 与后端信任地址保持一致。

登录 429/503、密码等待超时与数据库错误应分别排查，不能通过增大所有配额掩盖问题。当前检查入口见 [工程与验证](engineering-baseline.md)。
