# 健康探测与备份事件监控

`saas-monitor` 是独立 Rust 服务，主动探测已知平台/业务/通知健康路径，并接收恢复工具事件。状态机经连续失败或成功样本确认后记录 `DOWN`/`RECOVERED`。它输出私有指标，不直接发送邮件、短信或其他外部告警。

## 构建与配置

在本目录执行：

```sh
cargo test --locked
cargo build --release --locked
docker build -t saas-monitor:local .
```

复制 [config.example.json](config.example.json) 到源码外私有文件，替换实际地址与凭据。配置使用 snake_case，拒绝未知字段，文件上限 16 KiB。

| 字段 | 约束与含义 |
| --- | --- |
| `bind` | 私有监听地址，示例为 `0.0.0.0:9098`，不应公开到互联网 |
| `interval_ms` | 100–60000 毫秒探测间隔 |
| `timeout_ms` | 50–10000 毫秒单次 HTTP 期限 |
| `failures`、`recoveries` | 1–20 个连续样本确认状态 |
| `metrics_token`、`event_token` | 不同的独立 ASCII 凭据，各 16–1024 字符 |
| `targets` | 1–4 个不重复已知组件，按实际已部署服务选择 |

每个组件只允许固定路径，URL 不含凭据、查询和 fragment：

| component | 健康路径 | 示例私网上游 |
| --- | --- | --- |
| `platform` | `/api/health` | `http://backend:3001` |
| `work_orders` | `/work-orders-api/health/ready` | `http://work-orders-api:3004` |
| `stores` | `/stores-api/health/ready` | `http://stores-api:3031` |
| `notification` | `/health/ready` | `http://notification-ws:3003` |

只配置确实已部署且可达的目标，不为未部署服务伪造健康端点。容器 DNS 需目标加入同一受控网络。准确 HTTPS 信任由部署环境配置，不能关闭 TLS 校验作为上线方案。

## 运行和接口

```sh
cargo run --release --locked -- /srv/saas/private/monitor.json
```

| 接口 | 访问方式 |
| --- | --- |
| `GET /internal/metrics` | 指标专用 Bearer，返回 Prometheus 文本 |
| `POST /internal/events` | 事件专用 Bearer，接收固定备份事件 |

事件携带递增 sequence，旧事件不能覆盖新状态。恢复工具使用 `backup_run`、`backup_verify`、`backup_age` 等固定组件，标签不包含用户、URL 或秘密。指标示例与告警规则分别见 [prometheus.example.yml](prometheus.example.yml)、[saas-alerts.yml](saas-alerts.yml)。采集器与外部告警接收器仍需实际部署。

## Compose 运行

设置正式 `MONITOR_IMAGE` 镜像摘要，以及实际私有 JSON 绝对路径 `MONITOR_CONFIG_FILE`。配置须允许容器用户 `10004:10004` 读取。在平台 `backend` 执行：

```sh
docker compose --env-file /srv/saas/private/platform.env -f compose.yml -f compose.monitoring.yml --profile monitoring config --quiet
docker compose --env-file /srv/saas/private/platform.env -f compose.yml -f compose.monitoring.yml --profile monitoring up -d monitor
```

基础 overlay 不发布主机端口、不挂载 Docker socket，配置只读挂载到容器私有路径。Prometheus 必须通过受控网络访问，不经公共网关暴露 `/internal/metrics`。进程退出、配置读取失败、日志和采集器自身也需由外部监督。

## 验证范围

测试包含状态机、备份事件去重及真实 HTTP 故障恢复用例。运行时记录完整结果；若环境阻断回环监听，应保留失败证据并在允许测试程序回环入站的环境重验，不能只运行无网络用例来宣称完整通过。

探针成功不证明 qiankun 可用。上线还需按[部署指南](../../docs/deployment.md)验证最终 CSP、宿主和微应用加载、静态 MIME/404、登录、组织切换及业务权限。备份事件成功也不代表全部前端依赖已收集，恢复验收见[恢复指南](../recovery/README.md)。
