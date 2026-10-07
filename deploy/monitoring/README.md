# 本地运行监控

标准 Node 24 运行 `monitor.mjs`。配置必须显式提供目标、环境、绝对告警日志路径和阈值；配置文件不包含密钥，指标 Bearer 从指定环境变量读取。

```sh
export MONITOR_CONFIG=/etc/rust-saas/monitor.json
export NODE_EXTRA_CA_CERTS=/etc/rust-saas/test-ca.pem
# PLATFORM_METRICS_TOKEN、WORK_ORDERS_METRICS_TOKEN、NOTIFICATION_METRICS_TOKEN
# 由权限 0600 的实际环境文件注入，不在命令行或日志中输出。
/opt/rust-saas/node/bin/node deploy/monitoring/monitor.mjs
```

`config.example.json` 使用每 5 秒一次探测、连续 2 次失败告警、连续 2 次恢复确认。进程连续运行以保持状态，稳定正常、稳定故障时不重复写日志。`--once` 只做一次诊断：任意目标失败则退出码 1；它不绕过告警连续次数门槛，不作为周期调度的替代。

## 探测规则

- 公共平台、工单、通知健康与应用版本资源必须用 HTTPS。生产环境不允许公共 HTTP；不接受凭据、query、fragment 或隐式地址。
- `private: true` 仅用于精确 `127.0.0.1`／`[::1]` 的健康或指标端点，可在同机进程之间使用 HTTP。Prometheus 探针必须从 `tokenEnv` 读取独立监控 Bearer。公网网关拒绝全部指标路由，应用版本资源不能使用这个例外。
- HTTPS 使用 Node 的正常信任验证。测试 CA 通过 `NODE_EXTRA_CA_CERTS` 加载；`NODE_TLS_REJECT_UNAUTHORIZED=0` 会使监控拒绝启动。
- 不跟随重定向，默认 2 秒超时，响应最多 1 MiB（可配置上限 4 MiB）。健康必须返回 200、JSON `status: UP`；指标必须返回 200、Prometheus 文本及至少一个数值样本；版本资源必须包含配置的精确标记。
- 版本探针验证所配置的发布资源及预期版本。管理员发布或回滚时同步更新监控目标和标记；这个探针不代替平台对应用启用、租户授权和当前激活记录的检查。实际注册／激活／回滚链路在联合验收中验证。

本地日志仅有 `timestamp/event/target/reason/consecutive`，`event` 为 DOWN／RECOVERED，原因使用固定代码。无目标 URL、请求头、密钥、响应内容或人员标识；没有 webhook、邮件或 Slack 发送。

## 安装与停止

安装前创建 `rust-saas` 运行账号、Node 24 路径、配置及 `/var/lib/rust-saas/monitor`，配置与秘密环境文件设为 0600。服务文件为模板，须检查本机实际路径。`saas-monitor.service` 常驻，`saas-monitor.timer` 可在启动／服务退出后重新拉起同一服务，不并行启动多份检查。启用 timer 时，管理员停止应同时停止 service 和 timer。

Prometheus 与告警规则例子供已有私有监控 runner 使用，不会自动安装 Prometheus、Alertmanager 或配置外部接收人。演练的本地告警证据来自实际监控进程、受控服务故障与恢复；仅验证配置不能视为实际故障演练。

## 测试

```sh
node --test deploy/monitoring/tests/monitor.mjs
```

测试使用真实 loopback HTTP 和明确夹具，覆盖正常依赖、健康／版本契约、私有指标认证、重定向不转发、超时、响应上限与连续 DOWN／RECOVERED 阈值。实际 HTTPS 与新库恢复由本机准生产验收执行。
