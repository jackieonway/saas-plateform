# 健康监控指南

saas-monitor 是独立 Rust 服务，主动探测平台、工单、门店、通知四个健康端点，并接收恢复工具事件。连续失败/成功样本确认状态后输出 DOWN/RECOVERED，不直接发送邮件或其他外部告警。

## 构建

在本 monitoring 目录执行：

```sh
cargo test --locked
cargo build --release --locked
docker build -t saas-monitor:local .
```

本地 Windows 执行环境中，HTTP 故障恢复测试仍在成功基线处失败。交叉验证发现：Rust 客户端连接 PowerShell/.NET 的 `127.0.0.1` 和 `::1` 随机监听端口均成功，而 .NET 客户端连接新编译 Rust 程序的两种回环监听均超时，Rust 连接自身监听也超时。探测客户端已经禁用代理；换用 IPv6 或增加 HTTP 超时不能解决该监听入站限制，具体防火墙或主机策略仍需环境侧确认。测试保留真实 HTTP 成功基线及 DOWN/RECOVERED 断言，须在允许该测试程序回环入站连接的环境验收。备份事件去重和状态机另有独立无网络测试，不能用其通过替代真实 HTTP 验证。其他交付限制见 [验证状态](../../docs/repository-split-validation.md)。

## 配置与运行

将 config.example.json 复制到源码之外，只允许运维读取。bind 是私有监听地址；interval_ms、timeout_ms、failures、recoveries 控制探测与去抖。targets 的四个 component 保持固定，URL 改为运行环境实际可达的健康地址。

metrics_token 和 event_token 使用不同独立随机凭据。容器中的服务 DNS 需要对应服务加入同一受控网络；未部署的服务会持续不健康，不能靠伪造健康端点掩盖。

```sh
cargo run --release --locked -- /srv/saas/private/monitor.json
```

私有 GET /internal/metrics 使用指标 Bearer，POST /internal/events 使用事件 Bearer。恢复事件含递增序号，不接受旧事件重放；备份周期和状态确认阈值需要一起设计。

## 容器部署

设置 MONITOR_IMAGE 为正式镜像摘要，MONITOR_CONFIG_FILE 为私有 JSON 绝对路径，文件须可被容器用户 10004 读取。在平台 backend 目录执行：

```sh
docker compose --env-file /srv/saas/private/platform.env -f compose.yml -f compose.monitoring.yml --profile monitoring config --quiet
docker compose --env-file /srv/saas/private/platform.env -f compose.yml -f compose.monitoring.yml --profile monitoring up -d monitor
```

默认不发布主机端口、不挂载 Docker socket。Prometheus 示例和告警规则只是配置材料，实际采集与外部告警投递要由部署环境另外建立。服务退出、配置读取失败及日志也需要监测，不能只依赖服务自身事件。
