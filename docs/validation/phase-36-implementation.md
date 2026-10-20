# Phase Execution Result

## Executed Phase

Phase 36：平台受控镜像部署与容器生命周期。COMPLETED；验收范围仅本机隔离 Windows/WSL Docker。

## Execution Summary

正式输入964项，SHA256 2ea7c8ff026406727382e8ae905ce78317a433fa44f986c9e489812a7eb1d893；Git HEAD 32956c4532bbd6eff53584fd0f5e9bb6e188437c 标识基线，未提交源码由输入清单标识。最终采用 target/phase-36/implementation/final-r3/。源快照与宿主前后均不变。

平台新增可选受管静态发布，保留外部CI/Compose→URL登记流程。独立Rust agent通过正常HTTPS领取固定目标/可信仓库/digest任务，核验镜像/健康/路由后回执；公共预检与显式激活保持分离。平台没有Docker socket或容器控制权限。

## Deliverables

- [实现与权限/运行/故障恢复说明](../../deploy/managed/README.md)、[agent说明](../../tools/deployment-agent/README.md)。
- [实际输入](phase-36-implementation-inputs.json)、[运行/失败/ELF与镜像索引](phase-36-implementation-runtime.json)、[历史保护核对](phase-36-implementation-preservation.json)、[建议台账更新](phase-36-implementation-ledger-update.md)。
- 数据契约、独立agent、0005追加迁移、正式任务/重试/清理API与独立管理权限、管理页面及正式演练入口已落盘。

## Validation

- 完整fast：81步、1127项通过，失败/跳过/取消/TODO均0；filtered不计为执行。原始摘要与日志归档：target/phase-36/implementation/final-r3/fast.tar。
- 真实部署：18/18，通过本轮私有TLS registry push→候选清除→固定digest pull、非root只读/无cap/无端口/无bind模板、agent回执、公共HTTP和指针事务。
- 既有发布/ZIP/升级：17/17，通过；覆盖旧行/校验和保持及坏校验和拒绝、ZIP身份/重试/跨环境、预检、激活、回滚与旧客户端。归档：target/phase-36/implementation/final-r3/release-regression.tar。
- Chromium 156.0.8078.4、Chrome 155.0.8059.84、Edge 155.0.4283.62真实管理表单通过；没有TLS忽略。丢失POST响应后两次同requestId只产生一个任务，读取真实READY状态。
- 原始平台/agent ELF（machine62）及3个静态镜像归档的SHA256均重新计算匹配。所有本轮容器/网络/候选镜像/缓存卷/私有目录/registry端点CA已删除，cleanup.errors为空。

## Self-Check

| Acceptance Criterion | Result | Notes |
|---|---|---|
| 不可信仓库/无digest/越界目标/任意命令被拒绝 | PASS | 7类意图负例、独立平台根读写权限与白名单；代理仅固定argv；目标信息不泄露秘密 |
| 坏镜像/不健康/路由失败保持旧版本；幂等重试、可信回执 | PASS | 三类实际故障保持旧active/chunk；丢回执后公共预检PASSED仍409，含相对/百分号/重复斜线别名；旧nonce/跨target/伪造回执拒绝，同容器恢复；审计故障无部分提交 |
| 回滚/保留不删除active/previous/其他环境/在途资源 | PASS | 实际回滚双版本在线；保护引用与30天/最新5候选均409；CLEANING运行中新增绝对/相对/编码环境引用409且0新行；只删无引用失败孤儿，非本轮owner容器及哨兵保持 |

## Risks / Notes

- 本机隔离证明不等于真实远程/生产目标接入；没有外部发行、业务API/DB升级或Phase37协议切换。
- 不宣称空daemon/空编译缓存；本轮候选标识在pull前清除，基础层可复用。
- 169份既有正式证据、旧迁移1–4和其他阶段1–34/37正文保持；后端锁仅增加本地deployment-contract包引用，既有第三方包未变。
- 首轮复制属主与Docker日志配置失败、开发快照host变化、未带init的进程回收失败、报告路径权限失败、并行CLI测试fork/锁干扰均保留。原始失败/开发PASS不代替最终验收。30个无关格式化文件已按执行前哈希恢复。
- 标准整crate后端fmt/Clippy既有债务沿phase-01披露；新契约/agent严格fmt/Clippy和文档通过，后端检查、选定单元与数据库回归通过。

## Suggested Task Ledger Update

Phase36 COMPLETED；本阶段无必需剩余。Phase35已有用户的隔离多节点授权，单独恢复；37未触发。[完整更新块](phase-36-implementation-ledger-update.md)。
