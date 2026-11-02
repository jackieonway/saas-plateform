# 合作方审核、管理员验证与受控邀请

本合同对应 [Phase 19 台账](../unified-oauth-oidc-sso-task-ledger.md)。统一客户端、凭据和应用投影继续由身份服务的 canonical `auth_*` 管理；平台保存人工资料、独立审核事实和原操作回执。不得另建客户端权威，或使用浏览器提交的审核人、伙伴归属、已批准 scope 作为可信事实。

## 人员与权限

入口位于统一客户端页 `/system/oauth/clients`。`system:oauth` 只控制导航。下列细权限必须同时具备实际应用、租户、平台根组织菜单授权以及当前角色；普通租户管理员不符合平台根组织员工条件。

| 权限 | 操作 |
| --- | --- |
| `system:oauth:view` | 查看注册簿、申请和本人验证状态 |
| `system:oauth:register` | 登记伙伴/应用，提交被明确委派应用的申请 |
| `system:oauth:review` | 退回、独立审核及测试验收 |
| `system:oauth:publish` | 启用已经批准的生产申请 |
| `system:oauth:rotate` | 复用 Phase 4 凭据轮换 |
| `system:oauth:disable` | 暂停或纯停用，不能夹带 scope/回调等修改 |
| `system:oauth:scopes` | 修改客户端、资源 scope、应用权限或同意组 |
| `system:oauth:invite` | 签发、查看及撤销受控账号邀请 |
| `system:oauth:keys` | 管理公钥 |
| `system:oauth:assign` | 委派已有平台根组织员工管理指定应用 |

初始 bootstrap/demo 种子只把这些细权限授予初始平台管理员角色及平台根组织；不授予业务租户或分支组织管理员。伙伴管理员必须有明确的 `platform_auth_application_operators` 关联，且伙伴当前为 ACTIVE；伙伴归属不会自动创建租户管理员。全局目录及委派由当前平台 superadmin、审核或委派人员管理。

所有敏感写入在事务内要求原 canonical 平台 BFF/SSO 来源仍有效、已绑定 TOTP、`mfa_method=totp`，且数据库时间处于该 MFA 完成后的 5 分钟内。未来时间、恢复码方式、缺失/失效 factor 以及过期来源均拒绝。生产管理读取也要求绑定 TOTP。本人安全引导可在尚未绑定时读取，但不返回应用管理资料。

已绑定人员通过固定同源 `/auth/step-up?return=/system/oauth/clients` 重新验证。平台强制 `prompt=login`、`max_age=0`，产生新 state/nonce/PKCE 和新来源。首次没有 factor 时先完成近期主认证，在认证中心安全页绑定验证器，再从平台重新验证以建立带 TOTP 的新 BFF。不能把另一 SSO 会话的 MFA 时间写入旧平台 grant。

## 资料、状态与生产门禁

1. 平台接入人员登记伙伴主体、负责人和受控证据引用，再登记统一应用 ID 与固定伙伴归属。
2. 被授权人员分别提交 test/production manifest，提供用途、隐私说明、精确域名及 HTTPS 回调、联系人/域名证据 UUIDv7 和登出方案。首版只接受 `openid/profile`、USER/ACCOUNT_ONLY；拒绝个人联系方式、离线及业务数据 scope。
3. 独立审核员将 SUBMITTED 退回 RETURNED，或将测试申请批准为 TEST_APPROVED。原申请人、最近提交人及应用登记人均不能审核自己的申请。改变资料必须重新提交并推进版本，旧证据或版本不能沿用。
4. 平台通过独立审核服务将当前批准事实同步给身份权威；只有已知身份成功确认的版本才能验收。测试验收为 TEST_ACCEPTED，并保存当前测试证据引用。
5. 生产申请须先有同一应用的有效测试验收，再由实际绑定审核人员批准为 PRODUCTION_APPROVED，实际绑定发布人员明确启用为 ENABLED。审批与发布写入独立操作者、版本和审计；获准人员可以 SUSPENDED 暂停，身份权威同步收缩接入。

`platform_auth_release_bindings` 的初始唯一行 `enabled=false`，不含真实责任人。平台 runtime 只有 SELECT，不能从 UI/API 自行开启。部署责任人必须先确认真实 reviewer/publisher 账号、运营责任及数据政策引用，再通过受控部署流程填写；验证 fixture 的账号与证据不能成为真实上线批准。缺失绑定、账号停用或测试证据缺失时生产保持关闭。

身份 `auth_application_admissions` 按应用和 test/production 分别限制当前批准回调和 scope。第三方的伙伴归属不可改；没有对应环境批准、批准已暂停、回调/范围越界均不能创建或扩大客户端。OAuth 客户端加载再次检查当前 admission。test 凭据、客户端 ID 与 production 独立登记；不得用测试 secret 调用生产客户端。

## 幂等命令与结果恢复

浏览器在写入前生成并读回持久化 UUIDv7 `x-identity-operation`，按完整账号/会话/上下文分区。平台本地审核回执与私有远端命令绑定原操作者、会话、摘要、路径、服务用途和申请版本，使用相同 advisory lock 及首次接收时间窗口。

本地审批提交后远端同步不确定，API 返回原 operation 的 UNKNOWN 语义，浏览器冻结后续写入并查询原操作；刷新后仍恢复原编号。原会话丢失、当前权限/委派被撤销时拒绝恢复敏感结果。不得改用新 operation 重放未知同步。只有原操作权威确认不可能再应用，才允许明确重试当前审核事实。成功返回及恢复使用原授权版本快照，即使新申请并发提交，也不把新申请当作原操作结果。

客户端 secret 继续复用 Phase 4 一次性创建/轮换及确认机制，不能在伙伴列表或审核回执再次返回。secret 最多有效 90 天；旧凭据重叠截止为原 90 天到期与轮换后 24 小时的较早者。准确新到期由 `credentialExpiresAt` 返回，列表不回传 secret。

## 受控邀请

平台管理路径为 `/api/identity/invitations` 及 `/{id}/revoke`。仅 `system:oauth:invite` 及当前 TOTP 验证通过的平台管理员可操作；不向公开注册开放。签发保存不可变 `recipientRef` 与 `purpose`，两者改变必须另发新邀请；24 小时后、撤销后或成功领取后均拒绝再次消费。管理 API、浏览器、日志、审计和普通回执只包含元数据，不包含原 token。

身份私有用途严格分离：

| 服务用途 | 私有操作 | 返回 |
| --- | --- | --- |
| `AUTH_INVITATION` | 列表/详情/签发/撤销，原操作查询 | 元数据 |
| `AUTH_INVITATION_DELIVERY` | `POST /internal/identity/auth/invitations/{id}/delivery-material`，绑定已验证渠道引用 | 仅授权投递 worker 可取得原邀请材料 |
| `AUTH_INVITATION_CONSUME` | `POST /internal/identity/auth/invitations/{id}/consume`，绑定原收件人、用途、版本及可信核验引用 | 单次领取事实 |

消费是受控的单次领取原语，不自行创建账号、成员或权限。Phase 20 实现已验证渠道投递及有界重试；在真实投递/核验消费者就绪前，不得声称已可公开完成邀请注册。投递 worker 不得记录原材料。数据库仅保存 token 摘要与 key version；身份可用独立密钥按原邀请确定性重建发放材料。

## 部署配置与窄数据库边界

平台分别配置 `IDENTITY_REVIEW_SERVICE_CLIENT_ID/SECRET` 和 `IDENTITY_INVITATION_SERVICE_CLIENT_ID/SECRET`，身份登记对应 `AUTH_REVIEW` 与 `AUTH_INVITATION`。未配置时相应能力关闭，不能回退到 OIDC_ADMIN、AUTH_ADMIN 或其他服务凭据。投递与消费服务各自登记，不能复用管理秘密。私有 HTTPS 使用现有证书校验与限时调用；公开网关拒绝 `/internal/*`。

身份邀请密钥通过绝对 `AUTH_INVITATION_KEYSET_PATH` 读取，文件最多 64 KiB，格式为：

```json
{"purpose":"saas-identity/auth-invitation/1","activeVersion":1,"keys":[{"version":1,"key":"<32字节随机密钥的无填充base64url>"}]}
```

密钥必须与 TOTP 加密密钥、RP secret、服务 secret 独立。轮换时新增版本并保留仍可能在 24 小时生命周期内使用的旧版本。不得把实际密钥放入仓库、浏览器、命令行日志或验证报告。

身份 runtime 拥有 admission/invitation DML，但无平台申请、成员、角色或生产责任表权限。平台 runtime 无 auth_* 直接权限，仅可执行 `identity_admin_assurance` 取得当前绑定来源的最小 TOTP 事实。identity owner 提供该窄证明；platform owner 提供审核/邀请 intent fence，仅 identity runtime 可调用，并在身份提交时重新检查当前会话、细权限、委派、申请版本和完整批准事实。[角色交接与验证](../../../ops/identity-deployment/README.md)同时核对这些 owner/ACL 与生产责任表只读条件。

## 人工接入清单

- [ ] 实际伙伴主体、联系人、域名控制证据均已核验，引用对应当前申请。
- [ ] test/production 分别登记客户端与独立秘密；回调精确匹配受控域名。
- [ ] 仅 openid/profile，未登记敏感或业务数据范围。
- [ ] 当前实际审核、发布账号与运营/数据政策责任已由部署责任人确认；申请人未自批。
- [ ] 测试环境完成授权、回调、退出及过期检查，验收引用已记录。
- [ ] 轮换、紧急停用、未知操作恢复及审计查询的实际负责人已明确。
- [ ] 邀请投递通过已验证渠道，原材料不会进入管理端/日志；未就绪时保持关闭。

清单由实际责任人逐项完成。自动化 fixture 的 PASS 不自动勾选上述运营事实。
