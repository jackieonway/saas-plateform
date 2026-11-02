# Phase 22 认证监控与故障边界合同

本文件定义当前实现接口与后续验收要求，不是容量验证报告。阶段状态、实际运行证据及未完成项由[实施台账](../unified-oauth-oidc-sso-task-ledger.md#phase-22-完成认证运行监控与有界故障处理)及 Phase 22 报告维护。容量与运营目标来自[Q9](../unified-oauth-oidc-sso-policy-decisions.md#q9容量服务保障与责任)，不构成已达到的 SLA。

## 私有指标与凭据

身份服务通过 `GET /internal/identity/metrics` 输出 Prometheus 文本。该入口只接受独立的 `IDENTITY_METRICS_TOKEN` Bearer 运维凭据；未配置或空值时关闭，返回 404。凭据不是 SSO Cookie、RP secret、内部协议服务 secret 或业务 access token，不能复用。已配置值需为 16–256 个 ASCII 可见字符；生产至少 32 字符、至少八种不同字符，拒绝常见占位值。运行配置中与其他显式 secret/token/password/key 相同的值也被拒绝。该检查不代替部署责任人对文件密钥和数据库已登记凭据的独立保管。

Settings 不提供 Debug/Serialize，只保留比较所需摘要。入口拒绝 Cookie、查询参数、多重 Authorization 和错误凭据；所有响应 no-store/no-cache/no-referrer，无 Set-Cookie。既有私有入口 guard 继续限制 16 个请求槽位和三秒总期限。不得向公开认证网关开放此路径。

[Prometheus 模板](../../../ops/monitoring/prometheus.example.yml)使用单独凭据文件和私有 TLS CA，示例域名必须替换为实际批准的目标，不能关闭证书验证。模板不会启动采集器，也不会建立外部告警接收通道。

## 有界、无秘密的观测

HTTP middleware 只读取 router 提供的 `MatchedPath` 和固定 HTTP method/status，既不读取原始路径、查询、header，也不读取请求/响应正文。路由标签来自源码静态白名单，未列入的模板折叠为 `other`；非标准方法折叠为 `other`，未列入的响应码折叠为固定状态类。取消的请求独立标记 `cancelled`。请求指标不统计 scrape 自身。计时截至应用产生响应头，不代表外部 TLS、浏览器交互或完整流式响应传输耗时。

注册表为每进程固定标签空间，HTTP 最多 30 × 7 × 34 = 7140 个 histogram 组合，密码为三个阶段 × 六种结果。Histogram 桶固定在 1ms–30s，另有 `+Inf`，计数和桶随进程重启归零；抓取响应不能当作全局权威或持久审计。HTTP 503 的 `failed` 只反映可见响应，不凭空细分数据库、私有依赖或槽位饱和原因。取消不计为成功；未识别响应码归入固定 `other/failed`，不误记为取消。

| 指标 | 含义 |
|---|---|
| `identity_http_requests_total`、`identity_http_request_duration_seconds` | 固定 route/method/status/outcome 的请求数和应用耗时 |
| `identity_password_stage_duration_seconds` | `password_queue`、`password_execution`、`password_result` 的阶段耗时与结果 |
| `identity_password_running`、`identity_password_queued` | 已占执行 permit 数及 admitted 减去执行 permit 数 |
| `identity_password_concurrency`、`identity_password_queue_capacity` | 单进程配置预算 |
| `identity_db_pool_size/idle/capacity` | 单进程数据库池的当前数量及上限 |
| `identity_tcp_connections/connection_limit/idle_timeout_seconds` | 单进程已接入 TCP 数、硬预算与空闲期限，包含 keepalive 连接 |
| `identity_database_up` | 本次在期限内成功取得连接并完成数据库探测 |
| `identity_delivery_observation_up` | 本次 jobs 与 source outbox 聚合均成功且结果符合固定维度 |
| `identity_delivery_jobs{kind,state}` | LOGOUT/INVITATION/SECURITY 的五种任务状态计数 |
| `identity_delivery_oldest_pending_seconds{kind}` | 当前 PENDING 任务自创建起的最大年龄；包含受控退避等待 |
| `identity_delivery_expired_leases{kind}` | 到期仍处于 LEASED 的任务数 |
| `identity_delivery_source_events{kind,state}` | CONSENT/SECURITY 源事实状态；覆盖没有 job 的渠道缺失降级 |
| `identity_audit_write_failures_total` | 必须审计未能持久化或被抑制的次数，不保存失败内容 |

`password_running` 包含持有执行 permit、但尚在 Tokio blocking executor 等待的工作，不是瞬时 CPU 活跃线程数。`password_execution` 记录实际 closure 工作；`password_result` 单独记录调用方等待结果及超时。HTTP 已返回 429 时，不能把后续 closure 的 Ok 误解为没有超时，也不能提前释放仍在运行工作的预算。

数据库观测总期限 500ms，共用一次 pool acquire 和同一连接的只读事务完成探测、jobs、source 聚合；事务内 statement timeout 为 400ms、lock timeout 为 250ms，避免调用方取消后数据库查询继续占用默认三秒预算。取得连接/探测失败时 `identity_database_up=0`；投递聚合失败时 `identity_delivery_observation_up=0`，不输出 jobs/source 零值来冒充空队列。只有成功观测才输出完整固定维度及真实零值。观测不更新租约、尝试次数、回执、核验事实或业务授权，也不扩张 runtime ACL。

## 日志、审计与候选告警

关键写入继续与审计在同一授权事务提交，失败关闭。运行日志、metric label、采集器配置输出和报告都禁止密码、PHC、Cookie、Authorization、原 token、邀请材料、code、PKCE verifier、TOTP seed/恢复码、私钥和完整连接串；不能为了检索泄漏而把原秘密拼入 SQL。账号、接收人、任意客户端值和操作 ID 不作指标标签。

运行环境必须实际采集 WARN，包含 `identity_delivery_terminal`、`identity_delivery_tick_unavailable` 和密码压力事件。仅有 ERROR 过滤即使数据库记录了终态，也不满足日志告警可见性。密码 DEBUG 明细可关闭，聚合指标不依赖 DEBUG。验收需在实际过滤配置下捕获成功、拒绝、超时、取消、数据库异常和永久投递失败，并用内存 canary 检查日志、指标和报告不含秘密。

[告警模板](../../../ops/monitoring/saas-alerts.yml)提供依赖不可用、观测失败、连接池占满、密码准入/结果超时、成功请求 p95、失败率、审计失败、积压年龄、租约恢复和新增终态/降级的候选规则。500ms、0.1%、120s 和各持续时间均须按实测负载调整，不是服务等级承诺。TERMINAL/DEGRADED 是保留的历史事实，`delta` 规则观察新增变化；首次接管已有积压还需人工核对基线，指标不能代替 WARN 事件。源表和 job 可能描述同一事件，告警不可相加为独立故障数量。

规则文件与可见日志不等于已经完成告警投递。运维主责、安全升级、产品对接角色需由真实部署责任人绑定，验收实际接收、确认和升级路径；不填写虚构人员或发送未获授权的外部消息。

## 受压与故障验收

### 当前可配置预算

| 配置 | 默认值 | 有效范围与用途 |
|---|---|---|
| `IDENTITY_DB_POOL_MAX_CONNECTIONS` | 16 | 1–64，单进程数据库连接总量 |
| `IDENTITY_DB_ACQUIRE_TIMEOUT_MS` | 3000 | 10–30000，取得池连接期限 |
| `IDENTITY_DB_STATEMENT_TIMEOUT_MS` | 3000 | 10–30000，数据库语句期限 |
| `IDENTITY_DB_LOCK_TIMEOUT_MS` | 2000 | 10–30000，数据库锁等待期限 |
| `AUTH_REQUEST_CONCURRENCY` | 32 | 1–256，公开认证请求同时准入总量 |
| `AUTH_RATE_MAX_ENTRIES` | 4096 | 3–1000000，来源与客户端限流共享条目上限 |
| `AUTH_RATE_WINDOW_SECONDS` | 60 | 1–3600，限流窗口 |
| `AUTH_SOURCE_REQUESTS` / `AUTH_CLIENT_REQUESTS` | 各 1200 | 1–1000000，每窗口来源或客户端请求预算 |
| `AUTH_BODY_TIMEOUT_MS` | 3000 | 10–30000，只限制进入授权处理前的请求体读取，最大 32768 字节 |
| `IDENTITY_MAX_CONNECTIONS` | 64 | 1–4096，包含空闲 keepalive 的 TCP 接入总量 |
| `IDENTITY_CONNECTION_IDLE_TIMEOUT_MS` | 30000 | 100–120000，无 I/O 的连接期限 |
| `TRUSTED_PROXY_IPS` | 空 | 最多 64 个明确 IP；拒绝通配符、CIDR、重复与无效地址 |

来源只采信真实 TCP peer；只有显式可信 peer 可通过唯一、合法的 `X-Real-IP` 转交来源。其它转发头被清理，缺少真实 peer 时共用 `unknown` 预算，不能靠伪造转发头重置额度。客户端预算覆盖初始 authorize、token/refresh/client credentials、内省、撤销、登出及 operation 查询和已验证 UserInfo 来源；interaction 轮询继续受来源和全局预算控制。额度是进程内防护，不能作为跨实例授权裁决。

`/healthz` 与 `/livez` 是不访问数据库的进程存活探针；`/readyz`、`/health`、`/ready`、`/health/ready` 保留数据库就绪检查。网络关闭可能使提交结果未知，调用方须保留原 operation/source 以恢复；不能因超时重新发行授权或清除原来源。授权事务本身没有新增通用 HTTP 取消期限。

没有设置或设置为空的 `RUST_LOG` 默认采用 `info`。显式过滤若屏蔽 `saas_identity::delivery`、`saas_identity::delivery::worker` 或 `saas_identity::password` 的 WARN，启动拒绝并返回固定错误 `IDENTITY_REQUIRED_WARN_LOGGING_DISABLED`。

复用现有账号/IP 限流、密码有界队列、数据库 acquire/statement/lock 期限、私有 HTTPS 期限和可靠 outbox。必须分别测量队列满、密码结果超时、池占满、数据库失联、平台私有依赖失联、投递方故障与恢复：429、401/403 和 503 各自语义明确，不能清空用户状态或回退到缓存/旧协议授权。进程限额需分配到实际实例总预算；code、refresh、撤销与幂等正确性继续由数据库事务和唯一约束保证。

压测只在固定源码、明确资源限额和专属环境进行。Q9 的 code/refresh 合计 50 请求/秒、内省 200 请求/秒、持续 30 分钟是待验证目标；报告有效成功吞吐、p95/p99、业务拒绝与依赖失败、429/503、队列/连接/CPU/内存瓶颈，不能用快速拒绝计算成功容量。测试资源与数据遵循工作区的范围确认、预算、finally 清理及磁盘核查，结束后核实无本次临时数据库、角色、容器、网络或卷残留。
