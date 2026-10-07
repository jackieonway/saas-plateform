# 运行可观测性与本地告警

## 服务探针

| 服务 | 存活 | 就绪 | 私有指标 |
| --- | --- | --- | --- |
| 平台 API | `/api/health/live` | `/api/health` | `/api/internal/metrics` |
| 工单业务 API | `/work-orders-api/health/live` | `/work-orders-api/health` | `/work-orders-api/metrics` |
| 通知服务 | `/health/live` | `/health`，网关 `/notification-health` | `/metrics` |

工单就绪检查真实所需表和字段（工单归属／版本／软删，以及租户计数器）可查询，并向固定平台 origin 的保护上下文路由发起无凭据探针，必须收到 401 才判定保护路由可达。该探针不认为任何用户有效。通知就绪同时检查通知新 schema、独立 LISTEN 与平台保护路由可达；原有逐投递和定时会话验证保留。

指标端点只由同机私有监控访问。生产进程必须提供独立 `METRICS_TOKEN`，Node 要求 32–256 个随机 URL 安全字符、至少 8 个不同字符并拒绝占位值。建议使用 32 随机字节的 base64url 或 64 位随机 hex；不同服务使用不同值。精确 `Authorization: Bearer` 验证采用常量时间比较。开发模式未设置监控 token 时，指标仍拒绝访问。

## 日志与请求追踪

Node HTTP 接收合法 UUID `X-Request-Id`，其它值生成新 UUID，响应返回相同 ID，平台依赖调用传递该 ID。工单每请求 JSON 日志包含固定事件名、服务、静态路由、HTTP 方法、状态、请求 ID 与耗时；租户和组织字段仅在实际应用内省、上下文及数据 grant 校验通过后增加。用户 ID、工单 UUID、通知内容、URL/query、请求体、Bearer、机密客户端秘密和数据库连接串均不写请求日志。

通知 Socket.IO 会话验证使用握手中的合法 UUID 或新 UUID 作为内部平台请求追踪，公开认证／通知协议不新增 session 或 token 字段。可观测性不缓存授权，不延长已知期限，不改变数据权限与通知接收范围。

## Node 指标

| 指标 | 含义 |
| --- | --- |
| `saas_http_requests_total` | 已完成和中断 HTTP 请求 |
| `saas_http_request_duration_seconds` | 累积直方图，含 count/sum |
| `saas_http_errors_total` | 服务端错误计数 |
| `saas_http_auth_denials_total` | HTTP 401／403 计数 |
| `saas_database_up / saas_platform_up / saas_health_up` | 最近健康探测状态 |
| `saas_notification_clients / saas_notification_authenticated_clients` | 当前连接数及认证连接数 |
| `saas_notification_pending_hints` | 实时提示待处理数，队列上限 1000 |
| `saas_notification_listen_up` | 独立 PostgreSQL LISTEN 状态 |
| `saas_notification_auth_denials_total` | 会话拒绝或到期断开次数 |
| `saas_notification_hint_errors_total / saas_notification_hint_drops_total` | 查库错误／过载提示丢弃 |

HTTP labels 只有有限集合中的 service、固定路由模板、方法和状态，所有资源 ID 归一化为 `:id`。未知路径／方法／状态使用 OTHER，中断用 ABORTED；人员、租户、组织、工单、会话、URL 或动态业务内容不得作为 label。新服务发布要保持路由模板集合有限。

健康 gauge 表示最近的探针状态，不是授权结果。工单检查由 `/health` 实时触发，通知按配置周期及 `/health` 触发；监控应同时探测公开就绪路径和私有 scrape。

## 平台 Rust 指标与运行事件

平台使用 `dante_http_requests_total` 与 `dante_http_request_duration_seconds`，labels 为固定 route/method/status。`dante_http_authentication_failures_total`、`dante_http_permission_denials_total` 对应 401／403；`dante_token_validation_failures_total` 的 reason 只有 invalid_client/inactive_token/expired_token/invalid_token/other。

`dante_service_up / dante_database_up / dante_uptime_seconds` 表示进程、数据库与运行时长。`dante_micro_runtime_events_total` 按受验证的应用和固定 LOAD_FAILED／LIFECYCLE_FAILED、mount/update/unmount 阶段统计宿主上报提示。此提示不是权威健康证明；上报使用当前应用上下文，按应用／用户／租户限流。审计记录包含 request ID 与应用 ID，用于关联平台变更。

## 本机准生产演练

部署与配置见 [本地监控](../deploy/monitoring/README.md)。公共探针使用实际 HTTPS 与受信测试 CA，私有指标仅同机精确 loopback，网关拒绝指标路径。每个目标默认连续 2 次失败生成本地 DOWN、连续 2 次正常生成 RECOVERED；稳定状态不重复通知。

平台、通知和工单故障／恢复、HTTPS 证书验证、预期应用发布资源版本由实际原生验收记录。本地告警为 JSON 文件及标准输出，不向任何外部接收人发送消息。Prometheus scrape 和规则是可评审的 runner 配置，不把配置检查当作线上运行证据；阈值是初版运行目标，不构成商业 SLA。

平台 Rust 请求、私有指标、审计关联及最终故障门禁见 `docs/phase12-validation.md`。本机演练中，平台库故障时 `dante_database_up` 降为 0，通知独立 LISTEN 指标也降为 0；恢复后探针和本地告警回到正常状态。
