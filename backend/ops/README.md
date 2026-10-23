# 平台运维工具

本目录包含两个独立 Rust 工程，不参与平台前后端统一编译。它们提供固定范围的运行状态和恢复协议，实际目标、私有凭据、服务监督与调度由部署环境准备。

| 工程 | 当前能力 | 操作入口 |
| --- | --- | --- |
| `monitoring` | 探测最多四个已知服务健康路径，记录 DOWN/RECOVERED，接收备份事件，输出私有 Prometheus 指标 | [监控指南](monitoring/README.md) |
| `recovery` | pilot 模式下平台、工单、门店共同停写备份，远端取回验证，加密集合恢复到新的隔离目标 | [恢复指南](recovery/README.md) |

平台镜像部署见[部署指南](../docs/deployment.md)。监控 Compose overlay 不公开指标端口、不挂载 Docker socket。两项工具均没有自动安装的 systemd 服务或定时器，外部告警投递需单独配置，production PITR 尚未实现。

当前前端运行方式为 qiankun。服务健康 200 只能证明相应 API 探针成功，不能证明宿主 CSP、HTML/UMD 生命周期、静态资源、组织上下文或业务权限正常。恢复工具仍强制固定的两业务各两版本及四份静态镜像材料；它不会自动收集数据库引用的任意 qiankun URL/ZIP 资源。运行与恢复验收需要覆盖真实前端入口和完整版本依赖。

各工程在自己的目录执行 `cargo test --locked`、`cargo build --release --locked`。执行结果、跳过项与目标环境演练应分开记录，具体工程检查见[工程基线](../docs/engineering-baseline.md)。
