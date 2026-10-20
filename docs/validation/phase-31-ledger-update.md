# Suggested Task Ledger Update

## Phase Status Update
Phase 31: COMPLETED

## Output Summary
- 支持矩阵/expand窗口/分类政策：docs/upgrade-and-retention.md。
- 新增0004、纯策略、平台GET和Rust CLI只读预览、持久操作保护；没有apply。
- 最终32步、108次Rust测试执行（含重跑）/17项Node政策通过；885项输入 621ea69d5326f9b043cde3a01bf6ccbd2dd454179abb6b71efc78a76847f339e。

## Completed Items
- 平台3→4、工单27项升级、门店初始化与不可变历史验证。
- 8份冻结旧消费者、combined/legacy真实三库；全表只读/引用/取消/未知结果保护、旧镜像URL和ZIP目录、发布/回滚/审计。
- 同源证据、全部owned资源/缓存清理、135历史证据及其他36阶段详情保持。

## Remaining Items
- Phase31范围内无未完成项。
- 生产期限/凭据缓冲/旧页面窗口仍待产品与运维确认；不启用apply，不实施下一阶段。

## New Decisions
| Decision | Reason | Impact |
| --- | --- | --- |
| 仅静态发布试点，30天或最近5版并集保护，参数只加严 | Q08尚未批准生产期限 | 所有结果仅人工审阅，生产apply不可用 |
| 未完成操作持久保护；已确定结束只释放本次UUID，取消/未知保留 | 网络/提交未知与审计失败最终状态均需守住 | 无TTL，无自动孤儿对账或批量删除 |
| 只expand，不contract | 旧前端及页面寿命尚未明确 | 保留旧API/checksum/UUID/编号/grants；旧ELF回退另验迁移器 |

## New Open Questions
| Question | Impact | Suggested Owner |
| --- | --- | --- |
| Q08的生产期限、凭据缓冲、旧页面支持窗口及外部任务权威库存 | 不影响隔离候选验收；阻止生产apply/contract | 产品/运维 |

## Next Recommended Phase
Phase 32: 建立真实指标、告警与容量基线（NOT_STARTED，需新请求）。
