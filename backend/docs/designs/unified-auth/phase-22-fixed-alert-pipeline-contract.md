# Phase 22 固定条件告警链路合同

[`phase22-alerts.mjs`](../../../tests/unified-auth-https/phase22-alerts.mjs) 为任务专属
Node.js 24 fixture 提供实际私有 HTTPS scrape、有界固定条件求值和回环 HTTP
collector。它不解析 PromQL，不执行候选规则的 `for` 或时间窗口，不运行
Alertmanager，也不证明正式运营接收人、升级值班或持久告警投递已就绪。
[监控合同](phase-22-monitoring-contract.md)中的这些验收仍需独立证据。

模块没有启动即运行的入口；环境执行者显式调用导出 API。所有实际服务、
故障窗口、报告保存和 finally 清理均由上层 fixture 负责。纯自测运行：

```sh
node --test phase22-alerts.mjs
```

自测使用 fake metrics 和可控时钟；一项测试在当前 Node 进程启动独占临时
回环 collector，证明真实 HTTP 接收、丢失确认后的去重、鉴权与停止语义。
不启动身份服务、数据库或容器，不向外部地址发送告警。

## 输入与 API

`createAlertPipeline(inputPath)` 同步读取任务私有 JSON，形状严格如下，拒绝
额外字段：

```json
{
  "contractVersion": "identity-fixed-alert-input/1",
  "metricsOrigin": "https://auth.saas.test:24444",
  "caFile": "/owned/metrics-ca.pem",
  "metricsTokenFile": "/owned/metrics-token",
  "collectorOrigin": "http://127.0.0.1:24447",
  "collectorTokenFile": "/owned/collector-token"
}
```

input、CA 和两个 token 文件都必须为当前有效 uid 拥有的普通文件、精确
模式 `0600`，绝对路径，并以只读 `O_NOFOLLOW` 打开。环境执行者保证父
目录属于本次任务且其他 uid 不可写。token 为 16–256 个 ASCII 可见字符，
可带一个结尾 LF/CRLF；两个 token 不得相同。不得复用业务凭据。

metricsOrigin 只接受固定 `auth.saas.test`，标准 443 可省略，其他端口必须
显式填写，例如当前任务私有网关 24444；不得带路径、query、fragment 或
用户信息。DNS 所有结果必须为 `127.0.0.0/8` 或 `::1`。使用独立 Bearer、
显式 CA、`rejectUnauthorized: true` 及 Node 默认 `checkServerIdentity`，
真实主机名保留为 SNI。只有实际端点的 200 text/plain 完整响应才能求值。
public 443 对该私有路径的 404 应由环境执行者另行验证。

collectorOrigin 严格为带显式端口的回环 HTTP origin，固定 POST 路径
`/internal/identity/alerts`。HTTP 仅用于本任务相同 namespace 的回环接收
证明，不是正式外部接收端配置。两侧都不读取代理配置、不接受 redirect、
不传 Cookie，不记录正文或异常对象。

```js
const collector = await startCollector({ tokenFile: collectorTokenFile, port: 0 });
// port=0 请求一个本次独占端口；collector.origin 仅用于私有 input，不能写入报告。
// 上层 fixture 以 collector.origin 写好 inputPath 后：
const pipeline = createAlertPipeline(inputPath);
try {
  await pipeline.poll();                    // 健康基线
  // 上层进入真实故障窗口，至少 100ms 后再次 poll。
  await pipeline.poll();                    // 求值并尝试交付
  await pipeline.acknowledgeTerminal();     // 仅在已接收 TERMINAL FIRING 后确认
  const safe = { pipeline: pipeline.report(), collector: collector.snapshot() };
  // 上层以独占文件写入 safe，不序列化 input/collector.origin/原始 metrics。
} finally {
  pipeline.close();
  await collector.close();
}
```

同一 collector 只配一个 pipeline，复用这两个对象覆盖本次故障窗口。
sequence 是该 pipeline 的内存顺序号；不要把另一进程或重建 pipeline 的
序列号接到原 collector，也不要把 collector 重启描述成保留了去重历史。
错误只抛固定 `IDENTITY_FIXED_ALERT_FAILED`，不附加 cause、文件路径或秘密。

`poll()` 返回固定 status 和脱敏 report：OK、SCRAPE_FAILED、BUSY、THROTTLED、
EXPIRED 或 CLOSED。并发 poll 不排队，100ms 内重复调用不发请求。调用失败
或未知 metrics 不会把已有故障变成正常；已有待交付事件仍可按预算重试。
`acknowledgeTerminal()` 返回 accepted 和 report；accepted 只表示确认事件
已排入内存队列，须以 report 的 baseline/state 和 collector 接收记录核实
其 2xx，不能直接当成成功确认。

## 固定条件与 TERMINAL 基线

| 条件 | 触发 | 后续状态 |
|---|---|---|
| DATABASE_UNAVAILABLE | `identity_database_up == 0` | 同一指标为 1 时 RESOLVED |
| PASSWORD_QUEUE_PRESSURE | `identity_password_queued > 0` | 同一指标为 0 时 RESOLVED |
| NEW_TERMINAL_DELIVERY | 三个固定 kind 的 TERMINAL 总数高于内存已确认 baseline | 仅显式 ACKNOWLEDGED |

baseline 从 0 开始，首次采集已有未确认终态也会 FIRING。TERMINAL 求值要求
`identity_delivery_observation_up == 1`，且 LOGOUT、INVITATION、SECURITY 三种
TERMINAL 计数完整。缺失、重复、无效数值或未成功观测不解释为 0。
其他 series 不进入告警 payload 或动态标签。

终态 FIRING 得到 collector 2xx 后才能调用 acknowledgeTerminal。确认事件
本身也必须得到 2xx，才把 baseline 推进到原 FIRING 捕获的计数；确认前
增加的其他终态会在后续 poll 再次 FIRING。失败或丢失 2xx 都不推进 baseline。
TERMINAL 永远不发 RESOLVED；计数下降只增加 decreases，不形成恢复。
模块没有 SQL、数据库凭据、修改/删除终态或重置 outbox 的能力。

瞬态事件按观察到的转换排队，重复 scrape 不生成相同状态。RESOLVED 依赖
对应 FIRING 的已知交付；若 FIRING 到期或耗尽重试，依赖恢复事件也失败，
不能将未经证明的故障交付伪装成完整的接收/恢复链。

## 固定预算与交付

| 项目 | 上限 |
|---|---|
| scrape | 1 个 socket，2 秒总 deadline，2 MiB 正文、8 KiB header、40000 行 |
| 单次 collector 请求 | 1 个 socket，1 秒总 deadline，2 KiB 正文、8 KiB header |
| poll | 最短间隔 100ms，最多 18000 次/30 分钟，单次最多交付 4 个事件 |
| backlog | 16 项；满时记 backlogRejected，不假装已记录转换 |
| 同一事件 | 5 次尝试，60 秒 TTL，指数退避从 100ms 开始，硬上限 5 秒 |
| collector | 4 个连接，2 秒请求期限，128 项去重、120 秒 TTL、保留最近 64 项安全事件 |

不使用后台定时发送或无限重试。只有后续显式 poll/ack 调用才会交付已到期
重试的队首；上层等待相应退避后再调用。单项总 deadline 覆盖连接、TLS、
等待和完整正文读取；不解压响应。collector 拒绝未知字段、动态 condition、
无效状态、query、错误/重复 Authorization、Cookie 和超限请求。

固定 payload 只有 contractVersion、数字 sequence、固定 condition 与 state。
同一 sequence、相同 payload 重放返回 204；相同 sequence 改变内容返回 409。
去重集合满时返回 503，不提前驱逐有效记录来假装成功。仅 2xx 被 pipeline
认为确认；其余结果保留同一 sequence 重试。到期、重试耗尽、依赖失败和
关闭时放弃的 backlog 都单列计数，不计成功。close 取消请求并关闭 Agent；
collector.close 关闭本次 listener 和所有连接。

## 报告边界

pipeline.report 与 collector.snapshot 只含固定枚举、数值、布尔和安全的
固定事件，不含原始 metric 正文、账号、client、IP、token、路径、URL/query
或响应正文。为避免把私有 collector.origin 写入正式材料，仅保存 snapshot。
报告显式标记 FIXED_CONDITIONS、provesPromql=false、provesAlertmanager=false
和非持久交付。内存确认不代替正式工单、人员确认、值班升级或外部通知。

实际私有 TLS、真实 DB/密码队列/TERMINAL 故障窗口、接收和恢复证据由环境
执行者汇总。两次采集之间的短暂故障可能漏过；这个局限不能被描述为已
验证候选规则时间窗、正式监控覆盖率或已达到服务等级。
