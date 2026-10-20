# Suggested Task Ledger Update

## Phase Status Update

Phase 36: COMPLETED（仅本机隔离实现与演练）。

## Output Summary

独立Rust agent、数据契约、0005追加迁移、固定目标/仓库/digest/模板的任务API与管理UI、可信回执/激活门禁、回滚和引用保护清理。

## Completed Items

- 正式81步fast / 1127通过 / 0失败与跳过；18项部署实证与3浏览器；17项发布/ZIP/升级回归。
- 相对/编码/重复分隔符别名不能绕过回执；CLEANING期间新环境引用被事务拒绝。
- 原始ELF/镜像归档、源码/宿主不变、owner资源与临时信任/缓存清理均已复核。

## Remaining Items

本阶段必需验收无剩余。真实受管目标/外部registry和生产部署未授权也未宣称通过。

## New Decisions

| Decision | Reason | Impact |
|---|---|---|
| 固定静态Nginx模板与独立agent | 平台不持Docker权限 | 可选启用，现有外部URL流程保持 |
| 网关路径归一与应用版本保留 | 封住URL别名和新引用竞态 | 不能仅凭公共预检切换或清理保护资源 |

## New Open Questions

本机隔离范围无阻塞输入。

## Next Recommended Phase

Phase 35：用户已明确授权本机隔离多节点 ZIP 无共享存储分发，单独恢复执行；Phase37保持未触发。
