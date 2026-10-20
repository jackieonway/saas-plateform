# Suggested Task Ledger Update — Phase 11

日期：2026-10-16（Asia/Shanghai）。这是对 `docs/architecture-optimization-ledger.md` 的**建议更新块**，尚未直接写入原台账。用户提供的 task-ledger-executor 明确要求未经直接更新指令不覆盖台账；本文件与 [实际执行证据](phase-11.md) 保持可独立审查。

## Phase Status Update

Phase 11: **COMPLETED** — 修复普通 OAuth 原子签发与实时失权。

将台账版本建议提升为 v0.13，顶部状态改为“Phase1–11 COMPLETED，其余实施阶段 NOT_STARTED”。只修改 Phase 11 的完成状态及下一指针，不启动 Phase 12。

汇总表 Phase 11 建议替换为：

| Phase | Name | Status | Dependencies | Output Summary |
| --- | --- | --- | --- | --- |
| [11](#phase-11) | 修复普通 OAuth 原子签发与实时失权 | COMPLETED | 4, 6 | user→client→credential 锁序；消费、完整 token 对与签发审计同事务；共享实时身份、随机 jti/摘要、0003 兼容迁移。PG 7 组 41 项（OAuth10/psr4）及 fast24 全通过；502 输入/12 路径，证据 phase-11。 |

## Output Summary

- 新增 `backend/src/oauth_issuance.rs`，在现有单 crate 内建立窄签发用例，不增加依赖或 workspace。
- `oauth2.rs` 的普通授权码、设备码、refresh 改为事务状态机；consent 写入失败可重试，密码等锁后复核 hash，签发审计同事务且检查精确行数。
- `oauth_admin.rs` 踢出将平台会话和普通 OAuth 撤销合入同一用户优先事务，闭合刷新窗口；其余管理审计改造仍属 Phase 13。
- 新增 `0003_oauth_issuance.sql`，兼容旧无 jti/原文行，禁止新普通 JWT 原文存储；0001/0002 校验和不变。
- 新增 10 项 PG 专项；受管 CI 纳入普通 OAuth10、原 psr4，形成 7 组41，fast 新增 expiry 单测形成24步。
- 协议/现状文档及验证证据已交付：[phase-11.md](phase-11.md)、[inputs](phase-11-inputs.json)、[runtime](phase-11-runtime.json)、[边界](phase-11-boundaries.json)、[协议与升级](../oauth-issuance.md)。

## Completed Items

- [x] 同一普通 refresh 至少20并发兑换仅一次成功，重放 invalid_grant。
- [x] 消费、新 access/refresh 或签发审计失败没有烧毁凭据/半套 token；授权码及设备码单次消费。
- [x] 停用/到期/删除直接成员后的用户身份和各用户 grant 一致失效，机器 token 保留独立规则。
- [x] 新 token 摘要/元数据存储、随机 jti、独立撤销、旧库真实升级和 psr_/app 回归通过。

Phase 11 详情建议改为 COMPLETED，四项验收勾选，并以本文件输出摘要及以下 Notes 替换“尚未实施”段：

> 2026-10-16完成。新增普通 OAuth 窄签发、用户优先锁序、凭据消费/完整 token 对/签发审计同事务、共享实时身份和0003存储兼容。consent/password等锁/管理踢出补齐相关时序。正式Linux PG七组41（OAuth10/psr4）和fast24全部PASS零skip；Windows默认64PASS/68ignored分列，严格fmt/Clippy既有失败未隐藏。最终502项输入162fb670…，比Phase10记录12路径变化；session/app_token/Cargo锁/旧迁移/SDK/前端输入不变。WIP旧7项6PASS/1fixture FAIL及修复、未知提交/回退和清理证据保留，详见phase-11。未执行后续阶段、生产迁移或外部发行。

## Remaining Items

- Phase 11 无剩余实施或必需验收项。
- 全仓既有 strict Clippy/fmt 债务仍记录，不在本阶段顺手清理。
- 正式旧 token 原文清理政策及生产升级窗口由后续保留/运维阶段处理；不影响本阶段兼容状态机验收。
- 此建议块等待明确更新原台账的指令；本次实现和证据已经完成。

## New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| user→client→credential 持锁，条件消费、完整 token 对和签发审计同事务 | 一次性凭据竞争和签发失败必须整体提交/回滚，锁序与会话/成员撤销一致 | 普通OAuth封装在单crate窄模块，client配置持锁复核，不优化未来Phase28路径 |
| storage_version=1保留旧行；新默认2，普通jwt必须NULL | NOT VALID仍会验证旧行UPDATE，直接全局jwt为空约束会阻止旧凭据撤销/轮换 | 无清库、不改0001/0002，不混跑旧issuer；保留兼容回退构建 |
| 用户token复用当前有效性，机器token全空身份独立；普通introspect保留opaque摘要契约 | 统一用户/直接成员/组织祖先/租户/客户端状态，不能误伤机器凭据和存量摘要调用者 | userinfo仍验JWT；恢复状态可恢复未撤销未到期凭据，显式撤销/消费不恢复 |
| 网络未知提交必须重新授权，无原文恢复接口 | 摘要存储不能重建已签发响应，超时不证明事务回滚 | 不自动重耗未知refresh；不将psr_或应用token协议重写进本阶段 |
| 管理踢出的一次事务包含会话版本、sessions和OAuth撤销 | 两段提交间可能产生新refresh对 | 仅闭合本期实时失权；全面管理审计仍归Phase13 |

## New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 存量storage_version1原文、过期token的正式清理窗口及生产升级停写安排？ | 兼容保留已实现；不自动清理或生产采用，纳入既有Q08/Phase29及运维计划 | 安全/产品/运维 |

没有阻塞 Phase 11 的新开放问题。

## Next Recommended Phase

Phase 12：限制登录内存和密码计算成本，保持 **NOT_STARTED**，等待新的明确执行请求。

建议 Current Execution Pointer 更新为：

```yaml
current_phase: 12
current_status: NOT_STARTED
current_objective: 限制登录内存和密码计算成本；等待新的明确阶段执行请求。
last_completed_phase: 11
last_execution_authorized_phases: [11]
implementation_authorized_in_this_request: false
authorization_scope: 用户明确要求Phase11；普通OAuth原子签发/实时失权/兼容迁移及回归已完成，不执行Phase12或后续阶段、外部发行或生产部署。
next_action_requires_new_execution_request: true
```

保留原台账其余执行顺序、依赖、条件阶段及未收敛环境问题。Forbidden Actions 与 Current Restrictions 中的最近完成/授权阶段建议从10更新为11，明确Phase12没有自动授权。

变更记录建议追加：

| Version | Date | Change |
| --- | --- | --- |
| v0.13 | 2026-10-16 | 完成Phase11：普通OAuth授权码/设备码/refresh条件消费与完整token对/签发审计同事务、共享实时身份、随机jti/摘要、0003旧行兼容；consent/password等锁与kick时序闭合。Linux PG7组41（OAuth10/psr4）、fast24通过，502项162fb670…/12路径；旧fixture FAIL、Windows64/68ignored、严格fmt/Clippy既有FAIL与清理分别保存。详情/摘要建议COMPLETED，下一指针12 NOT_STARTED；无后续或生产操作。 |
