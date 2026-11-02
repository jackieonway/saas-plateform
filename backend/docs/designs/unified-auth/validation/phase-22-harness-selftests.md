# Phase 22 harness 自测证据

本报告记录当前会话已观察到的 harness 自测结果。机器可读数据及完整来源
SHA-256 见 [JSON](phase-22-harness-selftests.json)。它不证明实际身份服务、
数据库故障、密码压力、容量或持续运行通过，也不替代 Phase 22 实测报告。

## Node 自测：26/26 PASS

在任务保留的 WSL Linux Node.js **24.22.1** 中执行：

```sh
node --test backend/tests/unified-auth-https/phase22-load.mjs backend/tests/unified-auth-https/phase22-alerts.mjs
```

| 来源 | 通过 | 失败 | 跳过 |
|---|---:|---:|---:|
| [load](../../../../tests/unified-auth-https/phase22-load.mjs) | 15 | 0 | 0 |
| [alerts](../../../../tests/unified-auth-https/phase22-alerts.mjs) | 11 | 0 | 0 |
| 合计 | **26** | **0** | **0** |

Node test runner 总耗时 447.429964ms。新增回归模拟末尾 sleep 两次提前醒来但
单调时钟始终正向推进，证明 REFRESH/INTROSPECTION 会完整等待；另覆盖等待
期间取消，以及忙 worker 导致少发请求时按真实成功数计算吞吐。

其余检查覆盖输入/文件权限、固定标签、TLS 默认主机名校验选项、总期限、
队列上限、独立刷新链、未知消费结果退役、CODE 源耗尽、告警去重、TTL、
重试与终态确认。alerts 中一项实际启动临时回环 collector，验证丢失确认后
同事件去重、错误鉴权及停止语义；metrics 为 fake。listener、连接及临时
文件都在 finally 清理。这不是实际私有指标端点或运营接收通道的证明。

## 清理故障注入：4/4 PASS

已观察到的原始方法从 [runner](../../../../tests/unified-auth-https/phase22-run.py)
提取 main 的 finally AST，以 mock controller/subprocess/数据库清理函数及
四组可清除的临时文件 fixture 执行；不启动真实进程、不发送信号、不连接
数据库或 Docker。

| 场景 | 注入与断言 | 自测结果 |
|---|---|---|
| success | 模拟库/角色零残留，runner 应 PASS | PASS |
| control-failure | 控制器超时后仍清库、清私密文件；保留未清租约且 runner FAILED | PASS |
| database-failure | 模拟清库失败及库/六角色残留；仍清其它文件、保留 manifest 且 runner FAILED | PASS |
| malformed-pid | PID 文件 JSON 损坏后继续独立清理；保留归属记录且 runner FAILED | PASS |

失败场景的 **自测 PASS** 表示 harness 正确拒绝“清理成功”，不表示模拟中的
残留已经消失。四组原始临时文件 fixture 已删除。

原始临时代码现保存为可复跑的纯内存版本：
[phase22-cleanup-selftest.py](../../../../tests/unified-auth-https/phase22-cleanup-selftest.py)。
它只读取 runner 源码，所有 fixture 文件写入和进程/数据库/Docker 操作均为
内存 mock，并输出固定 JSON 和当次 runner/finally AST SHA。

```sh
python3 backend/tests/unified-auth-https/phase22-cleanup-selftest.py
```

容量窗口结束后已实际执行持久化版本：**4/4 PASS，退出码 0**。没有启动真实
进程、发送信号、连接数据库、调用 Docker 或写入真实临时文件。首次运行的
纯 mock 缺少宿主机 `df` 观测适配；只补齐自测适配后复跑通过，runner 未改。

本次精确来源为 runner SHA-256
`61e17c84d4580f4525f0c8b65ea03c7b2585ed1d8622cbd0b37a25facdcc41fa`，
finally AST SHA-256
`79fb838e5eb0ae601fe262c52c24c060901859b0985c7ebef059d392aeed75a5`。
自测脚本及原归档来源 SHA 全文保存在 JSON。原始 inline 运行未在当时记录
来源 hash，不把后来的归档 hash 冒充原始证明；本次持久化执行建立当前绑定。

## 容量复用只读核对

[独立容量组件](phase-22-capacity.json)的 PASS、三类完整时长及原报告 SHA
已核对，容量内容与保留的原报告逐项相同。原报告如实保留“容量之后因驱动
空 Cookie 失败”的整体结果；没有把原报告整体改写成通过。

[最终故障报告](phase-22-runtime.json)中的容量与该组件一致。faults-only
代码验证实际 Identity 二进制 SHA，并明确 separateOwnedFixture、复用已完成
组件与 newLoadClaimed=false。此次只读核对未重新计算大型二进制 hash、未
运行新负载，也不把新的故障 fixture 说成原容量 fixture 的继续执行。

这些自测不验证真实 POSIX 信号、进程复用、mount namespace、数据库权限或
Docker 回收，也不证明 PromQL、Alertmanager、持久告警交付或 Q9 SLA。
