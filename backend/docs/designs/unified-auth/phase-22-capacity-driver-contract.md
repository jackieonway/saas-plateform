# Phase 22 有界 HTTPS 容量驱动合同

[`phase22-load.mjs`](../../../tests/unified-auth-https/phase22-load.mjs) 是任务专属真实
TLS 客户端，不创建服务、数据库或容器。环境执行者负责准备、核对和清理实际
fixture；本驱动的纯自测通过不表示容量验收通过。Q9 尚未确认，不建立 SLA。

驱动只支持 WSL/Linux 中的 Node.js 24。由环境执行者选定公开 Node runtime，
在任务专属网络 namespace 内执行；输入中的文件均为绝对路径。

```sh
node phase22-load.mjs --input /owned/capacity-input.json --output /reports/capacity.json
node --test phase22-load.mjs
```

常规 CLI 仅接受上述顺序的 `--input` 和 `--output`，不读取 stdin、环境凭据、
代理配置或其他参数。输出文件必须尚不存在，使用 `wx` 和 `0600` 写入。
失败时 stderr 仅输出固定 `IDENTITY_CAPACITY_DRIVER_FAILED`，退出码为 1；
信号取消后仍写已完成请求的脱敏报告，退出码为 2。正常执行退出码为 0，
报告中的早停、拒绝和错误仍必须由环境执行者检查。

模块同时导出 `await runLoad(inputPath, outputPath)`，执行相同私有文件验证、
请求和正式 JSON 写入，返回同一个脱敏报告。API 不输出日志，也不设置
process.exitCode；调用者检查 completion。API 抛出的异常也只有固定错误码。

## 私有输入

input、client、CA、source 均必须是当前有效 uid 拥有的普通文件，精确模式
`0600`，以只读和 `O_NOFOLLOW` 打开。最终路径禁止符号链接；环境执行者
另负责父目录属于本次任务且禁止其他 uid 写入。不会打印路径或文件内容。

```json
{
  "contractVersion": "identity-capacity-input/1",
  "kind": "REFRESH",
  "origins": ["https://auth.saas.test:443"],
  "ratePerSecond": 20,
  "concurrency": 2,
  "durationSeconds": 30,
  "requestTimeoutMs": 5000,
  "clientFile": "/owned/client.json",
  "caFile": "/owned/ca.pem",
  "sourceFile": "/owned/refresh-source.json"
}
```

所有 JSON 都拒绝额外字段。`kind` 为 `CODE`、`REFRESH` 或 `INTROSPECTION`。
`ratePerSecond` 是 1–1000 的整数；`concurrency` 是 1–32 的整数；
`durationSeconds` 为 0.001–1800；`requestTimeoutMs` 为 1–10000 的整数。
没有隐式预算默认值。

`origins` 含 1–2 个不同的 HTTPS 端口，例如 443 和 24446，只允许固定
主机 `auth.saas.test`，不接受用户信息、路径、query、fragment 或端口 0。
`https://auth.saas.test` 与显式 `:443` 同义，其他端口必须显式填写；同一
443 不得用两种写法重复配置。各次实际发出的请求轮转分配 origin。DNS 结果必须全部为
`127.0.0.0/8` 或 `::1`；禁止连接外部地址。namespace 内由环境执行者绑定
实际测试实例。TLS SNI 保持真实主机名，使用输入 CA、`rejectUnauthorized`
及 Node 默认 `checkServerIdentity`；不会关闭证书验证。

client 文件形状：

```json
{"clientId":"TASK_CLIENT_ID","clientSecret":"TASK_CLIENT_SECRET"}
```

凭据只存在私有文件和内存中。机密客户端使用 OAuth form 编码后的
`client_secret_basic`。`CODE` 可明确指定 `"clientSecret": null` 使用 public
client，此时只在表单中传 `client_id`；其他 kind 必须有 client secret。

source 文件形状按 kind 固定：

```json
{"codes":[{"code":"ONE_TIME_CODE","verifier":"43_TO_128_UNRESERVED_CHARACTERS_LONG_VALUE_____","redirectUri":"https://rp.saas.test/callback"}]}
```

```json
{"refreshTokens":["INDEPENDENT_WORKER_0_REFRESH","INDEPENDENT_WORKER_1_REFRESH"]}
```

```json
{"tokens":["VALID_INTROSPECTION_TARGET"]}
```

source 至少含 1 项、至多 100000 项且文件至多 32 MiB。code 和 token 不重复。
`REFRESH` 的初始 token 数须恰等于 concurrency，每项属于独立刷新链；
不允许多个 worker 共享同一 token。`CODE` 每项仅取用一次；其 redirectUri
只进入表单，不被访问，必须为无用户信息和 fragment 的 HTTPS URI。
`INTROSPECTION` 按给定列表轮转，只执行只读请求。

## 调度、协议和失败语义

全局固定 pace，每个时间槽最多发出一项；事件循环迟到时跳过已经错过的
槽位，不补发突发流量。worker 忙时记为 skippedBusy，pending 永不超过
concurrency。HTTPS keep-alive 的 maxSockets、maxTotalSockets、maxFreeSockets
均至多 concurrency，不使用代理、redirect 或隐式重试。每项从请求创建到
完整响应有一个总 deadline，覆盖 DNS、连接、TLS、等待及响应读取；输入
预算至多 10 秒。请求和响应各最多 64 KiB，包括保守估计的头部；响应头
另限 8 KiB，不解压压缩响应。超限、截断、取消和网络错误不会暴露错误对象。

`CODE` 与 `REFRESH` 调用 `POST /oauth/token`，`INTROSPECTION` 调用
`POST /oauth/introspect`。只有完整 JSON 的 HTTP 200 且 token 字段有效才计
token success：非空有界 access_token、Bearer token_type、正 expires_in；
返回的 refresh_token/id_token 如存在也须有效。刷新成功还必须返回不同于
原 token 的有效 refresh_token，然后才更新该 worker 的链。

429 单列 rateLimited，刷新 worker 保留原 token，等待下一正常 pace 槽位。
503 单列 unavailable；其他非 200 单列 invalid。`CODE` 和 `REFRESH` 遇
503、其他 5xx 或传输结果未知时退役 worker，增加 uncertainRetired；不重放
可能已经消费的凭据。刷新 200 材料无效也按结果未知退役；刷新明确 4xx
拒绝会退役，但不计未知结果。内省只读，后续节拍可继续独立请求。

内省仅 `active === true` 计 effectiveSuccess；`active === false` 为 denied；
缺失或非布尔 active 的 200 为 invalid。快速拒绝和 429 不计成功容量。

CODE 队列取完立即停止发新请求并 drain，报告 sourceExhausted 和实际耗时；
不会伪称完成请求时长。所有可消费 worker 退役也提前结束。正常 run 保持
完整 requestedDuration，末尾等待已经发出的请求完成，最多增加一个请求
deadline；取消由 AbortSignal 立即终止活动请求。所有路径 finally 关闭 Agent。

## 脱敏正式报告

`identity-capacity-report/1` 只保留固定枚举、数值、布尔和固定 hostname。
不包含响应 body、原 URL/query、凭据、source、文件路径、账号、client ID、
用户标识、IP 或动态错误标签。主要字段：

- `target`、`budgets`：固定目标主机、端口及实际 socket、pending、时间和字节预算。
- `completion`：reason、sourceExhausted、requestedDurationCompleted、schedulingSeconds 和 elapsedSeconds（含 drain）。
- `counts`：attempts、effectiveSuccess、denied、invalid、rateLimited（429）、unavailable（503）、network、uncertainRetired、workersRetired。
- `throughput`：successRps 只除以实际 elapsedSeconds；attemptsRps 独立呈现。
- `latencyMs`：effectiveSuccess 和 all 各自的 nearest-rank p50/p95/p99；无样本为 null。
- `scheduler`：inflightPeak、pendingPeak、skippedBusy 和 skippedLate。
- `client`：本次 CPU 用户/系统时间、按单核计的 CPU 百分比、起始及采样峰值内存；processLifetimeMaxRssBytes 明确为进程生存期峰值。

有限样本内存最多按 `ceil(rate × duration)` 预分配两组 Float64Array，至多
180 万项/组；不会为每个未发出的槽位创建任务。采样峰值内存每 250 ms
观测，可能漏过更短尖峰，因此同时呈现内核进程生存期峰值。

驱动不独自判定服务瓶颈。正式容量结论还需同窗口服务/数据库/队列指标、
CPU/内存、实例数、限额配置、测试来源及负载客户端瓶颈证据。报告中的
`evidence` 明确未建立 SLA 或独立瓶颈结论。实际运行、资源归属核对及完整
清理由环境执行者汇总到 Phase 22 正式报告。
