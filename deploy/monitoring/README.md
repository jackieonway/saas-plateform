# 私有运行监控与候选基线

`saas-monitor` 是独立 Rust 服务。它直接探测平台、工单、门店和通知的固定健康路径，以连续失败/成功样本确认稳定状态；初始健康不发送事件，稳定 DOWN 与 RECOVERED 各输出一次 JSON 到本地 stdout。UNKNOWN 不表示健康。当前没有配置外部接收渠道，stdout 不能作为外部告警送达证据。

从仓库根构建：

```sh
cargo test --locked --manifest-path deploy/monitoring/Cargo.toml
docker build -f deploy/monitoring/Dockerfile -t saas-monitor:candidate .
docker build -f business-apps/work-orders/backend/Dockerfile -t work-orders-api:candidate .
docker build -f business-apps/stores/backend/Dockerfile -t stores-api:candidate .
```

两业务 Dockerfile 使用仓库根作为 context，以包含无第三方依赖的固定指标协议 crate；业务 SQL、平台模型和数据库连接仍分别独立。历史镜像证据不证明当前源码 builder 已运行。

复制 `config.example.json` 到操作者控制的私有路径，替换两个不同凭据并核对内网目标。配置不得进入源码、静态资源或镜像层。设置 `MONITOR_IMAGE=repository@sha256:...` 和 `MONITOR_CONFIG_FILE=/absolute/private/config.json`，显式合并 `deploy/compose.monitoring.yml` 并启用 `monitoring` profile。模板不发布端口，也不挂载 Docker socket；仅私有采集网络允许连接。各健康路径的含义不同：平台 health 验证平台依赖，业务 ready 验证本业务库 schema，live 只说明进程存活；业务 ready 不能证明平台授权 HTTP 可用。

| 端点 | 独立运维凭据 | 默认 |
| --- | --- | --- |
| 平台 `/api/internal/metrics` | `METRICS_TOKEN` | 无有效凭据拒绝 |
| 工单 `/work-orders-api/internal/metrics` | `WORK_ORDERS_METRICS_TOKEN` | 未设置禁用 |
| 门店 `/stores-api/internal/metrics` | `STORES_METRICS_TOKEN` | 未设置禁用 |
| 通知 `/metrics` | 通知服务的 `METRICS_TOKEN` | 独立配置 |
| 监控 `/internal/metrics` | config `metrics_token` | 必须显式设置 |
| 监控 `POST /internal/events` | config `event_token` | 必须显式设置 |

公开 Nginx 拒绝这些 metrics 路径，包括携带 Bearer 的请求。运维 token 不能复用应用 token、JWT 签名密钥或机密客户端密码。`prometheus.example.yml` 使用私有凭据文件；只提供采集与告警规则样例，不默认安装 Prometheus、Alertmanager 或外部 receiver。`saas-alerts.yml` 是候选阈值，正式 SLO/资源预算与渠道需另外确定。

监控配置中的 `metrics_token` 和 `event_token` 必须各为 16–1024 字节的可见 ASCII 字符串（不含空格或控制字符），且两个值不同，以保证能够用于 HTTP Bearer 鉴权。

指标只使用闭合的 service/stage/outcome 枚举、静态路由和状态码。新增直方图 `saas_stage_duration_seconds` 包含完成数、累计桶、耗时和在途数；取消计为 cancelled，不能当作成功。业务 authorization 包括当前平台授权验证，platform_http 包括网络和响应处理；business_sql 是业务读用例耗时，包含池等待、事务和解码，不能当作 PostgreSQL 服务端纯执行耗时。pool_acquire 包含获取连接及 BEGIN。DB lock gauge 只计当前数据库 Lock 等待者，查询失败以 observation/database up 表示；零等待者不等于数据库可用。

平台 context_work_orders/context_stores 分别记录应用授权上下文；password_queue 包括准入和等待工作槽，password_execution 记录真实 Argon 工作。超时 HTTP 可以先结束，已启动阻塞工作的 CPU 仍可能继续消耗，两者不应合并计算。连接池 size/idle、密码 running/queued 为瞬时观测。

浏览器 entry、ready 是从开始挂载到 entry load、认证通道 READY 的累计时间，mount 是 READY 后 MICRO_MOUNT 确认耗时。来源是实际 iframe 生命周期，通过受认证、有界、限速的 TIMING 事件进入平台固定桶；浏览器值可被客户端操纵，不能作为安全决策。它们不是 LCP 或 INP。真实浏览器基准单独使用 PerformanceObserver 与 Event Timing，并明确样本/网络/CPU条件；短脚本交互序列只能提供实验室交互延迟，不能当作现场 INP。

`platform_notification_committed_rows` 是持久 inbox 当前行数，删除/保留策略会使它下降，因此是 gauge；读取失败设置 observation_up=0。通知 `saas_notification_emit_accepted_total` 是传输提交接受数，保留旧 delivered_total 兼容；它不证明浏览器收到、用户看到或已读。通知队列/分区、LISTEN/数据库/平台健康和补拉收件须联合检查；用实际收件 ID 集合验证恢复完整性。

备份扩展端点仅接受 `backup_age`、`backup_run`、`backup_verify`、boolean healthy 和严格递增 sequence；乱序返回409，连续样本同样防抖。监控本身不执行备份或判断文件年龄；[Phase33 Rust恢复CLI](../recovery/README.md)在取回认证远端集合后负责年龄判定及事件生产。外部投递仍未启用。

候选容量用例：`backend/src/phase32_monitoring_tests.rs`。在现有 owned Linux runner 中先 locked 构建平台、双业务和 monitor 到 `target/ci-rust`，再执行：

```sh
node scripts/ci/monitoring-postgres.mjs --run-tests --out-dir /absolute/fresh/report
node scripts/ci/monitoring-browser.mjs --browser chromium --out-dir /absolute/fresh/browser --source-revision FULL_HEAD_SHA
```

第一个入口也接入现有 CI postgres 门禁。它拒绝开发/生产数据库，使用独立非超级用户角色和三库，断言清理后退出。5万工单（状态/优先级/时间分布，5%软删除）与5万日报（4组织/门店、不同日期/金额）不增加索引。一个主体持有200个不同应用token，100/150/200活跃会话每个顺序发5次读取；在线会话、活跃数、实际在途峰值分开记录。全部请求包含实时授权、SQL和响应体；报告p50/p95/p99、状态分布、5xx及传输失败率、耗时、吞吐与三个独立服务的CPU ticks/RSS/HWM。短突发不能推导持续容量，单主体不能推导100个独立用户容量。数据库单独运行于 owned Docker 网络，服务和驱动共用工具容器CPU/内存；硬件、cgroup和时钟参数与原始报告一起保存。候选失败也保留数据，不修改授权缓存或伪称生产SLA。

完整本轮结果与已知限制见 `docs/validation/phase-32.md`。

夹具预算须分开阅读：API容量实验先用真实 Rust 初始化地区参考目录，再按默认5000ms statement timeout 启动受测平台；通知/浏览器夹具为冷库的665,923行参考目录导入显式使用60000ms平台SQL预算，测量从ready之后开始。它们没有修改生产默认值，不能混合声称同一API容量口径。当前冷库首次导入在该硬件上可能超过默认5s，应独立审查引导配置；本阶段记录该限制，不通过长授权缓存掩盖延迟。
