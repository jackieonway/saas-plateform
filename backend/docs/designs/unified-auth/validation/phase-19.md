# Phase 19 — 合作方审核与受控生产接入

日期：2026-10-28  
状态：COMPLETED；组件验收、真实 HTTPS 13/13 及隔离环境定向清理均已完成。

## 范围与交付

实现依据为 [Phase 19 台账](../../unified-oauth-oidc-sso-task-ledger.md)与已采纳的 Q3–Q8 政策。具体人员权限、状态、API、配置及人工清单见[合作方接入合同](../phase-19-partner-onboarding.md)。本阶段复用统一客户端和 Phase 4 凭据权威，不创建第二套客户端注册簿。

- 平台人工资料保存伙伴主体、负责人和域名证据、固定应用归属、test/production manifest、明确应用委派及申请版本。退回、批准、测试验收、生产批准、发布和暂停分别记录操作者及审计；原申请人、最近提交人和应用登记人不能审核或启用自己的申请。
- 管理权限拆分为 view/register/review/publish/rotate/disable/scopes/invite/keys/assign。旧 `system:oauth` 只导航。平台根组织员工、实际菜单/角色/组织权限、当前 canonical BFF/SSO 来源和应用委派在事务及身份提交时复验。
- 生产管理员必须绑定验证器；敏感写入要求当前来源的 TOTP 在数据库当前时间之前 5 分钟内完成。恢复码、未来/陈旧 MFA、已撤销来源及被撤销权限均拒绝。安全页引导与 `/auth/step-up` 区分：新 MFA 不能附加到旧平台 grant。
- 独立 AUTH_REVIEW 服务只接收当前人工批准事实的严格快照。身份按应用/环境保存 admission，限制第三方的 openid/profile、精确 HTTPS 域名/回调及 USER/ACCOUNT_ONLY 客户端；客户端登记和在线 USER/MACHINE token 验证均消费当前 admission。
- 测试验收必须先确认当前批准版本已同步，不能用先前已同步版本验收修订后的 manifest。生产责任表初始关闭，runtime 只读；没有真实审核/发布账号和运营/数据政策绑定时保持关闭。
- 账号邀请为独立管理用途，固定收件人和用途、24 小时、摘要持久化、单次受控领取，支持撤销。管理端与回执不返回原 token；材料只向独立投递 worker 取得。Phase 20 继续实现真实已验证渠道投递；此阶段不开放公开注册或自动授予成员权限。
- 浏览器在写入前持久化原 operation。已提交本地审批后的远端不确定保留原编号，PENDING replay 不伪报完成；已知 NOT_APPLIED 与完成快照在所有恢复路径一致。迟到结果只更新原 receipt，不覆盖较新的未审核申请。
- 凭据轮换继续使用一次性 secret 返回/确认机制；90 天凭据寿命与最多 24 小时旧 secret 重叠同时生效。新 DTO 返回准确 `credentialExpiresAt`。授权码默认 TTL 收紧至政策要求的 60 秒。

## 已完成的组件验证

| 证据层级 | 验证内容 | 当前结果 |
| --- | --- | --- |
| 前端源码与生产构建 | Node、细权限策略、TypeScript、ESLint、Qiankun 交付检查 | 445/445 Node；5/5专项策略；407项交付检查；全部通过 |
| 真实 Chromium + 独立 HTTP fixture | 伙伴提交/独立审核、默认生产关闭与显式配置fixture、390px表单/邀请、近期验证引导、冲突/未知结果恢复及敏感字段拒绝 | [16/16](phase-19-ui/phase19-partner-intake-browser.json)；截图保留于同目录 |
| 注册簿浏览器回归 | 单次 secret、组/目录、版本冲突、会话和手机布局 | [10/10](phase-19-ui/unified-clients-browser.json) |
| 平台 canonical PG | 真实细权限/BFF来源/MFA事实、申请/自批/版本/证据、委派/跨应用、生产默认关闭/明确fixture绑定正向、原receipt晚到不覆盖及NOT_APPLIED一致性 | 8/8，0 ignored，16.42s |
| 平台 facade canonical PG及纯回归 | 当前root员工、细权限、stale/recovery/future MFA、生产无factor、实际客户端归属、纯停用、原会话恢复、审计失败回滚、仅写权限无VIEW的内部metadata读取 | 27/27，0 ignored（19纯回归、8canonical PG） |
| 身份 canonical PG + 协议 HTTP | review投影CAS/完整快照/环境准入及真实guard分页/过滤/错误query；邀请摘要/24h/绑定/单次领取/撤销/失效/无key/审计回滚 | review/query 3/3（新query用例修前FAIL、修后PASS）；invitation 2/2，另独立keyset单元测试1/1 |
| 身份 PG + 协议 HTTP、精简owned schema | 实际签发token后删除当前环境准入、保留另一环境、恢复/暂停/回调漂移；原USER/MACHINE资源proof回归 | 新准入1/1、原资源proof1/1 |
| 空库与实际LOGIN角色 | 当前canonical/provision/verify、逐项撤销资源服务或邀请表的UPDATE权限后验证器必须拒绝、恢复后通过 | Phase15共享schema矩阵3/3；含auth_invitations单项权限收缩 |
| 静态检查 | 平台和身份 all-targets strict Clippy；定向 rustfmt与diff；当前文档相对链接 | 最终接口/8组测试版本已通过 |
| 实际浏览器 + 严格CA HTTPS + 实际双runtime角色 | 三名独立人员真实绑定/密码/TOTP、新BFF及旧代际失效、真实列表分页、审核/准入/邀请、陈旧和恢复码拒绝、默认关闭及临时fixture生产发布/暂停 | [13/13](phase-19-https-browser.json)，TLS1.3，无HTTP fixture、无TLS绕过、未用SQL种正向MFA |

组件 fixture 不冒充真实 TOTP 登录。平台 PG 将实际 canonical 来源和 MFA 行作为明确测试事实；facade 的私有客户端 metadata 是显式fixture。身份 review/invitation 测试保留 canonical 通用 intent 的锁、期限和绑定，人工 actor/批准快照边界为显式fixture。精简schema的在线token回归不证明部署ACL。浏览器 HTTP fixture 不证明实际 SSO、域名控制、真实生产批准或邀请投递。

## 验收自查

| 台账验收条件 | 结果 | 证据及剩余门禁 |
| --- | --- | --- |
| 生产管理员及敏感写入强制 TOTP 与最近5分钟验证；缺失/过期由服务端拒绝 | PASS | canonical PG/facade覆盖生产无factor与权限矩阵；实际HTTPS三名人员真实绑定及新BFF、缺factor/陈旧/真实恢复码拒绝 |
| 申请、退回、批准、生产登记及暂停有明确操作者与审计 | PASS | 平台8组覆盖退回/修订/自批/审计；真实HTTPS完成独立测试批准、身份准入及明确fixture责任下的生产批准/发布/暂停，MFA审计各新增3条 |
| 未批准scope、回调、跨伙伴及test凭据调用production拒绝 | PASS | 当前环境admission、客户端凭据绑定及在线token检查覆盖环境缺失/暂停/回调漂移；平台PG覆盖跨应用与manifest，实际HTTPS拒绝email扩大，未开放业务生产数据scope |
| 轮换复用Phase4且不会二次泄露secret | PASS | 原operation绑定、审计回滚、浏览器10场景与凭据寿命回归；列表/伙伴审核均不返回secret |
| 只有获授权的平台管理员可签发/撤销邀请；绑定不可变、摘要持久化、过期/撤销/消费不可重用 | PASS | 身份canonical PG+HTTP覆盖生命周期/单次领取；实际HTTPS由独立授权审核员签发/撤销、24h元数据，陈旧/恢复码来源均403；实际投递仍属Phase20 |
| 未确认运营责任和数据政策保持受控 | PASS | 默认enabled=false、runtime只读、缺失实际绑定生产失败关闭；fixture责任人不转为真实发布事实 |

## 当前发布门禁与资源

真实 HTTPS 首轮使用固定运行时快照 `605ac9…`，三个服务通过严格 CA 健康检查后运行。已保留三份失败报告，均为 FAILED，不作为阶段完成证据：

- [网络空闲等待失败](phase-19-https-browser-red-networkidle.json)：实际登录/回调/session已成功，管理页持续轮询使 harness 的 networkidle 条件不成立；随后改为已验证回调后直接在真实 `/auth/config` 页面成为唯一刷新驱动。
- [首次独立审核员同意等待](phase-19-https-browser-red-reviewer-wait.json)：已完成真实申请方 TOTP 绑定、新 BFF/原代际失效与申请提交，首次审核员同意提交后 harness 重复等待按钮。
- [首次发布员同意竞态诊断](phase-19-https-browser-red-consent-race.json)：已有9项实际检查通过，包括独立测试批准/身份准入、邀请签发/撤销、陈旧及真实恢复码拒绝和默认生产关闭。诊断只记录固定 substep、origin/pathname和视图布尔值，确认同意已实际回调而 harness 重复点击；改为单候选提交一次后等回调。

这些运行还暴露独立的产品缺陷：身份内部 guard 拒绝了 canonical 客户端列表的合法分页 query。修复仅允许 GET 与精确 `/internal/identity/auth/clients` 路径携带查询，保留 handler 的未知/重复字段及边界校验；真实 canonical PG+HTTP 先复现失败，再验证修复通过。最终HTTPS已明确断言注册簿分页200和准确records/page DTO。

最终通过使用[组合运行时证明](phase-19-runtime-attestation.json)：平台、门户、schema和静态前端继续使用 [r1构建证明](phase-19-build-attestation.json) 的 `605ac9ef8aba39fbeed7037287380559bab9a0c075bbc61a49362f71fc1d87f3`；身份服务单独使用 [r2更新证明](phase-19-identity-update-attestation.json) 的 `9d33d0bde98e3a3a02b2da729b610ce69d828c74cd2b88e2b65cf885a9b2b580`，身份binary为 `fb2ff4e1552825eb5c8701def2b7e2935a7748a1b14032ce0094cbe0875ac0c9`。r2运行时只修改identity guard及其测试，没有重建数据库、平台或门户。当前harness SHA为 `4ce41eaa92055bc9158542a5e9c1fae2512207d4dcdafbc3d82eeca44e358422`，与首轮snapshot中的harness修订明确区分。

[最终正式报告](phase-19-https-browser.json) SHA为 `33d10f080c887354d4aba4a8834614994c802bd014cdbbe8712db6250d22ae8d`。报告证明3次真实绑定及启用审计、6次真实重新认证入口、合法分页200、私有审核/邀请成功及相应403拒绝；未导出seed、恢复码、密码、Cookie、token或完整回调查询。临时生产责任仅为隔离fixture，不能勾选实际运营清单。

真实 Q3 审核/发布账号、Q8 运营责任/数据政策仍未提供，因此生产保持关闭。这是当前源码执行的发布条件；测试中的独立账号和引用只是验证fixture，不代表实际责任人已确认。未配置邀请keyset或独立服务用途时相关操作失败关闭。

本分支组件验证只使用 root 明确提供的 loopback PostgreSQL `55494`，核对 cluster identifier `7701548153213487841`，按 UUID 数据库范围创建/删除。没有创建或复用 Docker 容器、镜像、网络、卷或构建缓存。各组件fixture均在退出/finally关闭连接并删除本次库/角色/临时签名目录，初始化失败路径也执行定向清理；平台intake累计21个fixture库已删除，相关前缀残留0。

实际HTTPS由主流程拥有新库、独立runtime角色、三个WSL进程、namespace、TLS网关及CONNECT代理的完整生命周期。身份guard修复仅替换身份进程，数据库及其余进程复用。最终浏览器已关闭，原production binding已恢复；三个专属fixture账号的factor/enrollment/recovery row/未撤销SSO均为0，审计在该次验收结束前保留。随后主流程停止三个服务，按原manifest删除共享与门户两个任务库及owner/runtime角色，独立catalog复核残留0/0；namespace、gateway、proxy与服务记录均为0，24445监听退出，原本机443服务未改动。12个私有环境/密钥/应用、网关及代理日志文件已逐个删除，详见[定向清理记录](phase-19-cleanup.json)。任务专属原生PostgreSQL及共享构建缓存/源码快照保留供后续阶段使用，尚不代表整个任务资源已清空。

浏览器 Vite/Chromium已关闭，原两份 target 输出按明确文件清理；正式 JSON与截图保留在本报告的 `phase-19-ui`。无 Docker空间变化，不将数据库/文件清理表述为 Windows WSL虚拟磁盘已缩小。
