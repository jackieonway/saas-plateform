# Suggested Task Ledger Update — Phase 12（已按明确指令采纳）

日期：2026-10-16（Asia/Shanghai）。用户明确要求更新Phase11完成、执行下一阶段，并在完成后标记台账。因此以下更新直接采纳到 [原台账](../architecture-optimization-ledger.md)，不再保留“等待覆盖确认”的状态。

## Phase Status Update

- Phase 11：COMPLETED，正式采纳此前实现/证据；保留历史证据的执行时点语义。
- Phase 12：**COMPLETED**，三项验收全部PASS。
- 当前下一指针：Phase 13 / NOT_STARTED。项目总状态仍IN_PROGRESS，因为其余阶段尚未实施。

## Output Summary

- 进程级共享硬容量、TTL与可测时钟登录预算，源/input/user三类条目，固定摘要键与有界过期索引。
- 统一异步密码服务，全HTTP链复用运行/总入场双许可；取消或超时不释放后台仍在执行的计算预算。
- 真实socket/可信代理来源，XFF从右剥离、坏链回落和统一审计身份。
- 共享alias累计验证预算，输入边界，改密异步后锁内CAS；联系人初始化接入同一密码服务。
- 正式Linux PostgreSQL8组46与fast27全部PASS零skip；最终505输入9b016ace…、20路径变化/3新增、无删除。
- [执行报告](phase-12.md)、[输入](phase-12-inputs.json)、[runtime](phase-12-runtime.json)、[边界](phase-12-boundaries.json)、[协议与配置](../login-protection.md)。

## Completed Items

- [x] 100k随机用户名/来源键不超过配置容量，TTL回收与回拨/前跳均验证。
- [x] 密码运行/排队上限、过载、取消与超时语义可断言，混合真实HTTP负载不阻塞健康/普通请求。
- [x] 进程内预算、独立副本N倍与重启清空写清，未虚称共享配额或生产容量。
- [x] 摘要/详情/证据/决策/变更记录及下一指针同步，原Phase11证据不回写。

## Remaining Items

- Phase 12 无剩余项。
- 既有strict Clippy/fmt债务保留实际失败日志，不顺手批量修复。
- 实际多副本共享配额需求及生产资源预算须在相应部署/容量阶段明确；不以本地单进程预算宣称跨进程全局配额。

## New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 固定TTL+硬容量，饱和拒新，不驱逐活跃身份 | 随机输入不得扩内存或重置被攻击账号预算 | 来源/input/user共用cap，键和索引有界；LOGIN_MAX_ENTRIES最小3 |
| 双许可留在blocking closure，响应截止与计算结束分开 | 超时/取消不证明Argon2已停止 | 默认2运行+8排队，满载/截止429，不能提前补回在途预算 |
| 所有HTTP密码路径共享，包括联系人初始化 | 单一路由有界不足以保证进程总成本 | 用窄PasswordContext传递原policy+共享service，不新增业务功能或重构事务 |
| 共享alias用总验证deadline，改密用锁内旧hash CAS | 候选数量和异步等待放大成本与竞态 | 不返回部分ticket，不覆盖并发reset；既有候选登录元数据不承诺整体SQL回滚 |
| 保持Argon2默认与旧PHC，配置只提供进程内预算 | 不降低密码策略，单API模板不等于共享配额 | 非默认PHC资源另测，N副本独立、重启清空，无Redis/生产共享规则自动变更 |
| 普通OAuth保留独立撤销契约 | 密码reset自动撤普通access不是既有规则 | 测试明确平台/psr/app失效、旧密码拒绝和显式revoke，不借Phase12扩大策略 |

## New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 实际API副本数、总内存/CPU和是否必须跨实例共享来源配额？ | 当前进程内模式明确可验；未来共享模式须实现并按目标部署测试，不能直接水平扩容后称全局限流 | 运维/容量负责人（既有Q09） |

没有阻塞Phase12的开放问题。

## Next Recommended Phase

**Phase 13：使关键管理变更与审计同事务，NOT_STARTED。** 等待新的明确阶段执行请求。

已采纳的执行指针：

```yaml
current_phase: 13
current_status: NOT_STARTED
current_objective: 使关键管理变更与审计同事务；等待新的明确阶段执行请求。
last_completed_phase: 12
last_execution_authorized_phases: [12]
implementation_authorized_in_this_request: false
next_action_requires_new_execution_request: true
```

v0.13记录Phase11采纳，v0.14记录Phase12开始与完成。保留其余阶段顺序、依赖、条件分支、历史失败与未提供生产环境的问题。
