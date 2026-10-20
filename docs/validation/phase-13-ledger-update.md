# Suggested Task Ledger Update — Phase 13（已采纳）

用户明确要求执行 Phase13，沿用“完成后标记台账为已完成”的要求；以下已实际写入原台账。

## Phase Status Update

Phase 13: COMPLETED

## Output Summary

- 严格主事务审计、必要对象与删除身份快照、安全前后摘要、无变化重试及进程失败观测已完成。
- 最终 506 项输入 / 582d2759e38c06f62ea609bb9b0debfe634e3415cde6617cb604f401bccec9ab；PG10组64、fast27、Windows86及最终差异检查通过；既有严格Clippy/fmt失败和81ignored如实分列。

## Completed Items

- 关键管理/授权/发布现有调用链与业务主事务共同提交，审计异常和零插入都阻止提交。
- 67真实矩阵/134故障、18no-op、双租户第二审计失败、3锁屏障、通知十项及原46项验证完成。
- 修复已实证的组织/角色复合外键级联顺序，提交前重新验证，不改历史迁移。
- 现状文档、报告、输入/范围/运行索引、台账状态及下一指针同步。

## Remaining Items

- 无本阶段实施或必需验收余项；全仓既有Clippy/fmt债务保留，不以本阶段扩成无关清理。

## New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 复用audit_logs与主事务，严格一行插入 | 避免commit后best-effort或trigger跳过时丢失审计 | 无新迁移/依赖；关键失败回滚，诊断分类保留 |
| requestId只追踪，真实no-op不新增变更 | 防止同集合重试重复事件，同时不能吞掉同ID不同真实变更 | 保留既有通知/ZIP幂等与凭据操作约定 |
| 发布预检独立持久，指针与自身审计同tx | 网络不能持长锁；预检观测和发布是两个动作 | 不宣称整个HTTP SQL全部回滚；未知commit继续对账 |
| 父锁顺序及锁后重验/计数 | 关闭本次暴露的app/client反转和等待前旧快照 | 保持用户优先、app→client、tenant→org，3类交错实测 |
| 仅暂缓两个已有DEFERRABLE检查后立即复验 | 角色和组织树配套CASCADE的检查顺序已有SQL与HTTP复现 | 合法级联删除成功，最终完整性保留，历史checksum不变 |

## New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 无新增阻塞问题 | 生产环境值、全局策略和未执行后续阶段仍按原台账余项处理 | 原台账对应负责人 |

## Next Recommended Phase

Phase 14: 提供受控数据 grant 管理 API。状态 NOT_STARTED，本次未执行。
