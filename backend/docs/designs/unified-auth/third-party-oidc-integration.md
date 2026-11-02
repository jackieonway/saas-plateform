# 独立第三方 OIDC 接入指南

本文对应统一台账 Phase 21。[可运行独立示例](../../../../../saas-examples/oidc-rp/README.md)使用通用 `openid-client 6.8.8`，不需要平台 SDK、内部 Rust 实现、平台成员或身份数据库。运营方先按[合作方审核合同](phase-19-partner-onboarding.md)完成当前环境申请、独立审核和准确登记，再将公开客户端配置与自身秘密经受保护部署通道交给接入方。

## 取得配置与最小资格

| 配置／登记 | 接入方需要的事实 |
| --- | --- |
| issuer | 唯一、准确的 HTTPS origin；必须与 Discovery `issuer` 和签名声明 `iss` 完全一致 |
| client | 当前环境的 CONFIDENTIAL／WEB_BFF／USER／ACCOUNT_ONLY，认证方法 `client_secret_basic` |
| redirect URI | 完整登记的 `https://rp.example.com/auth/callback`，不使用通配或浏览器传入的任意回调 |
| scopes | 第三方首版仅 `openid profile`；`email`、`phone`、业务资源及 `offline_access` 默认不开放 |
| credentials | 部署注入仅该客户端的秘密文件，列表、前端、Git、URL 和日志均不保存秘密 |
| back-channel | 运营方核实域名／目标后登记的 `https://rp.example.com/auth/backchannel-logout`，与当前审核 manifest、client 和 target 版本一致 |
| trust | 公共 CA 或部署提供的 CA 文件，保留 hostname 和证书链校验；不关闭 TLS 验证 |

ACCOUNT_ONLY 登录不要求门店、平台员工、客户成员或组织选择。账号由受控创建／邀请取得，没有公开自助注册。邀请投递和单次领取遵循[Phase 20 合同](phase-20-logout-delivery-contract.md)，第三方不能通过创建业务成员来代替账号来源核验。

当前审核实现不允许 THIRD_PARTY 申请离线或 MACHINE。独立示例的离线配置与机器 CLI 是显式资格下的协议用法；环境变量不能扩大审核上限。Phase 21 正向离线／机器实测使用另外登记的获批 FIRST_PARTY fixture，报告与默认第三方证据分开说明。

## 标准登录与身份绑定

1. 服务端从固定 issuer 获取 `/.well-known/openid-configuration`，核对准确 issuer、HTTPS 端点和已声明能力。浏览器请求不能选择 issuer、JWKS 或 token URL。
2. 每次登录产生独立 state、nonce、S256 verifier／challenge，绑定当前浏览器的服务端流程。使用 `response_type=code` 和准确回调；不发送 password、implicit、device 或动态注册请求。
3. 回调只消费一次原流程，核对 state、授权响应 `iss`、PKCE 和 nonce。示例显式使用 `ClientSecretBasic` 与 `enableNonRepudiationChecks`；ID Token 必须存在，RS256 签名、当前 JWKS、issuer、audience 和期限均由标准库校验。
4. 使用 Access Token 请求 UserInfo，并由标准库比较 UserInfo `sub` 与已验证 ID Token 的 `sub`。ID Token 不作为 API bearer；浏览器只收到最小身份显示和本地会话。
5. 应用以 `(issuer, sub)` 作为外部账号唯一键。`sub` 与会话 `sid` 都按有界不透明字符串保存和精确比较，不能解析为内部 UUID、成员 ID、手机号或邮箱。

同一邮箱、名称、头像或可猜测标识不能证明已有本地账号与外部账号属于同一人。关联已有账号必须同时具备当前本地账号的近期重新认证，以及这次独立 OIDC 登录的服务端校验结果；关联确认还需绑定当前浏览器、双方原账号和一次性操作，避免登录 CSRF、并发换绑或迟到结果覆盖。示例不提供缺少这两个控制权证明的自动关联：`POST /api/account/link` 固定拒绝并返回 `EXPLICIT_DUAL_CONTROL_LINK_REQUIRED`。需要关联产品流时，应在实际本地认证系统完成双证明后单独实施和验收。

## 刷新、撤销与登出

refresh 由服务端单消费者执行，每次替换原 refresh token，不能并发重耗、沿用旧 token 或把不确定结果当作可自动重试。原 issuer、sub、sid 和 scope 不得改变或扩大；没有新 ID Token 时保留原已验证身份。在线 grant 不超过原 SSO／业务来源；另行获准的 ACCOUNT_ONLY 离线 family 使用绝对 7 天、空闲 24 小时上限，轮换不延长绝对截止。自然浏览器到期与显式来源撤销分别处理。

| 操作 | 已实现行为及边界 |
| --- | --- |
| RFC 7009 撤销＋本地退出 | 示例先关闭本地原来源，再撤销 refresh family 和 Access Token。远端失败时本地仍关闭；不自动重试不确定的令牌操作 |
| 标准 back-channel logout | 固定 issuer JWKS／RS256 验证已登记 aud、sid／sub、events、jti 和期限；同 jti 重放拒绝。未知 sid 的已签名通知保留有界撤销事实，阻止迟到回调重新建立来源 |
| CURRENT_APP／GLOBAL | 这是 Discovery 明确声明的 `rp_logout_command_endpoint` 命令扩展。CURRENT_APP 只撤销原 grant；GLOBAL 撤销原账号的全部 USER 来源；两者保留同意 |
| RP-Initiated Logout | 当前 Discovery 没有 `end_session_endpoint`，不能调用或宣称已支持标准浏览器 RP-Initiated Logout，也不把 Basic 命令伪装成该协议 |
| 未登记通知的降级 | 本地会话不超过 5 分钟，敏感请求每次或至多 60 秒做在线检查；示例的 UserInfo 动作每次检查，普通会话视图最多 60 秒复核 |

扩展命令绑定原 token、sid、sub、scope 和同一个 UUIDv7 operation。超时／网络失败后查询原 operation；只有明确 `404 NOT_FOUND` 才允许以原编号再次提交。错误回包、任意 404、另一 operation 或新会话不能用来猜测旧操作是否提交。当前应用退出不等于删除同意，也不承诺所有第三方页面即时关闭。

## 机器调用与资源边界

[machine.mjs](../../../../../saas-examples/oidc-rp/machine.mjs)使用独立 SERVICE／MACHINE／NONE、`client_credentials` 和 `client_secret_basic` 配置。机器应用、资源、scope 与凭据版本均须另行批准；人员 RP 凭据不能切换为机器主体。MACHINE 不产生人员 ID Token、UserInfo 身份或用户 refresh token。

Access Token 是 opaque。资源端在线内省时以自身独立资源客户端认证，并精确核对 active、issuer 权威、audience、scope、主体模式和当前授权来源。仅仅取得某个 scope 字符串或用户成功登录，不代表获准访问门店、工单、通知或平台业务 API。示例 CLI 只调用部署固定的一个 HTTPS 资源 URL，调用者不能在浏览器中提供任意目的地址。

## 错误处理与运维交接

| 情形 | 接入方处理 |
| --- | --- |
| state／nonce／issuer／签名／PKCE 不符，UserInfo sub 改变 | 失败关闭这次流程，不建立本地身份，重新发起独立授权 |
| code／refresh 已消费、过期、来源撤销或离线期限已过 | 不重耗原凭据，清除本地认证状态并要求重新登录 |
| scope、audience、审核环境或主体不符 | 停止请求，按准确资源和用途重新申请；不增加请求参数绕过 |
| 认证中心、JWKS、数据库或网络不可用 | 有界超时并失败关闭，不切换旧 issuer、内网证明或本地密码 fallback |
| 登出结果未知 | 保留原 operation 并查询；本地原来源已关闭，不恢复旧会话 |
| 来源／重放事实达到容量上限 | 已验证原来源的全部本地会话先关闭，再返回明确 503，不伪报 ACK、不驱逐未过期事实；原 operation 可恢复查询。来源事实无法记录时以一个有截止标记阻止等待中的新 code 结果挂载，现存无关来源不被无差别清除 |
| 接收方故障 | 使用固定可观察错误和有界等待，按合同受控处理；不使用无限 Map、队列或无界重试 |

示例按单进程、单副本有界内存运行，重启会失去本地会话及待确认操作。多副本、跨重启恢复和持久业务账号关联需要生产接入方选择原子共享存储并单独验收，不能把这个演练实现当作这些能力的证据。运行日志不输出请求 URL 查询、Cookie、code、verifier、token、秘密文件内容或完整上游正文。

当前阶段的具体输入版本、标准库测试、实际 HTTPS 矩阵、owner fixture 时间边界与清理见 [Phase 21 验证报告](validation/phase-21.md)。通用客户端互操作通过不等于身份服务已获得 OpenID 官方认证；真实运营责任、正式域名、发布授权和外部开放仍受后续发布阶段门禁约束。
