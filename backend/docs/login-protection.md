# 登录保护与密码执行预算

平台使用两层进程内保护：登录来源/账号固定窗口预算控制证明尝试，统一 PasswordService 控制 Argon2 运行和排队资源。实现位于 `src/login_limits.rs`、`src/util/password.rs`、`src/client_ip.rs`、`src/routes/auth.rs` 和 `src/config.rs`。这些机制服务于平台登录、OAuth 密码相关路径及管理密码写入，与 qiankun 的挂载方式独立。

## 平台登录流程

`POST /api/auth/login` 接受 JSON 的 username 与 password。username 可以是用户名、电话或 Email。请求先验证原始账号字节数及密码长度，再修剪账号空白、标准化电话或 Email，接着执行：

1. 消耗来源请求额度，成功登录也不退还来源额度。
2. 预留标准化输入身份的账号槽，避免随机未知账号绕过容量。
3. 查询最多 65 个候选账户，超过 64 个匹配时要求换一种登录标识。
4. 对每个候选账户预留账号槽，在同一个总验证截止时间内执行密码验证。
5. 成功证明只清除对应账号和本次输入的失败预算，不处罚共享同一电话或 Email 的其他账户。
6. 没有有效成员的账户不能建立平台上下文；失败凭据记录相应失败预算和登录审计。

单账户成功返回短期 credential ticket；共享标识匹配多个成功账户时返回 credential_multi ticket、可选 memberships 和 accountCount。用户仍需调用 `POST /api/auth/context` 选择真实用户和组织，才建立平台会话。总密码验证预算超时不会返回部分成功账户的 ticket。

credential ticket 不带选定租户/组织，不是受保护 API 或 qiankun 应用的访问令牌。应用令牌签发见 [OAuth 与应用令牌](oauth-issuance.md)。

## 固定窗口预算

来源额度和账号失败额度共享一个硬容量，身份键使用哈希存储。账号预留在密码计算前发生；容量已满时拒绝新身份，不驱逐仍存活的身份来腾空额度。过期项按固定批次清理，计时使用单调时钟，窗口不会被系统时钟回退重置。

| 配置 | 默认 | 允许范围 |
| --- | --- | --- |
| `LOGIN_MAX_ENTRIES` | 4096 | 3–1000000，共享来源及账号条目容量 |
| `LOGIN_WINDOW_SECONDS` | 600 | 1–86400 |
| `LOGIN_SOURCE_REQUESTS` | 60 | 1–100000，每来源每窗口 |
| `LOGIN_USER_FAILURES` | 5 | 1–1000，每账号失败预算 |
| `LOGIN_IDENTIFIER_MAX_BYTES` | 320 | 1–1024 |
| `LOGIN_VERIFY_BUDGET_MS` | 10000 | 1–120000，共享标识候选总验证时间 |

已存在来源或账号的额度在窗口到期后恢复。成功证明不会恢复来源额度。保护状态属于单个进程，重启会重置，不同副本不共享；部署多副本需要结合网关限速，并根据总资源预算分配每副本参数，不能把它当作集群配额或持久账户锁定。

## Argon2 资源控制

所有 HTTP 密码 hash/verify 路径共享 PasswordService。先以总容量准入，再等待运行许可，拿到许可后才复制敏感输入并提交 blocking 任务。这样排队请求不会无限占用内存或 CPU。

| 配置 | 默认 | 允许范围 |
| --- | --- | --- |
| `PASSWORD_CONCURRENCY` | 2 | 1–32，同时运行数 |
| `PASSWORD_QUEUE_CAPACITY` | 8 | 0–1024，额外等待数 |
| `PASSWORD_QUEUE_TIMEOUT_MS` | 250 | 1–30000 |
| `PASSWORD_EXECUTION_TIMEOUT_MS` | 10000 | 1–120000 |

密码输入最多 1024 字节。超长输入返回 400；容量满、排队超时、执行响应截止或登录聚合截止返回 429。响应带 `Retry-After`，密码工作预算失败通常为 1 秒；来源/账号窗口预算返回到窗口释放的等待秒数。

Argon2 已开始后不能强制取消。即使 HTTP 请求超时或调用方断开，blocking 工作仍持有准入与运行许可直到真正结束，避免取消请求后无限启动新计算。服务保持现行 Argon2/PHC 格式，不通过降低密码成本换取吞吐；非法或异常过长的存量 hash 按验证失败处理。

## 可信代理与客户端 IP

`bootstrap.rs` 在路由前调用 `client_ip::normalize`。默认使用真实 socket peer，忽略直接客户端提交的 `X-Real-IP`、`X-Forwarded-For` 和 `Forwarded`。仅当实际 peer 在 `TRUSTED_PROXY_IPS` 的精确 IP 列表内时才解析转发地址。

可信代理链优先解析 `X-Forwarded-For`，从实际 peer 向左检查，停在最近的不可信跳点；单跳代理无 XFF 时可使用其覆盖写入的 X-Real-IP。重复头、错误 IP、超过 2048 字节或 32 跳的链回退为 socket peer，不转而信任另一个伪造头。无 socket 的进程内请求进入共享 `unknown` 来源桶。

规范化后删除原转发头，写入唯一规范 X-Real-IP，路由限流和审计都使用这个结果。运维必须把真正连接平台的受控代理 IP 配入 allowlist，并让代理覆盖外部转发信息。不能把任意公网客户端或宽泛网络范围当作可信代理。

## 会话安全与问题判断

修改本人密码、管理员重置密码等安全变更会更新身份版本并撤销相关平台会话及用户 OAuth 授权；机器凭据保持独立。关键写入在数据库锁等待后重查当前权限和会话，不能复用等待前的旧判断。相关撤销行为由 `src/session.rs` 及对应路由实现，受[管理审计](management-audit.md)事务约束。

| 现象 | 检查方向 |
| --- | --- |
| 429 且 Retry-After 较长 | 来源/账号窗口预算或身份容量饱和 |
| 429 且 Retry-After 为 1 | 密码运行/排队预算、执行截止或共享标识聚合截止 |
| 同一代理后的所有用户共用额度 | 检查真实 socket peer、可信代理 IP 和代理覆盖的转发头 |
| 密码正确但无可用组织 | 检查直接成员、租户和完整组织链状态/有效期 |
| 旧会话突然 401 | 检查退出、密码修改/重置、管理员踢出或会话到期 |

## 回归验证

纯单元测试覆盖共享硬容量、随机身份洪泛、固定批清理、时钟变化、成功清理和并发额度；PasswordService 测试覆盖准入、取消和执行许可；client_ip 测试覆盖伪造、代理链、重复头及异常长度。

`src/login_protection_tests.rs` 的 ignored 集成测试另外覆盖真实 HTTP 登录洪泛、共享电话/Email、聚合验证截止、密码生命周期及高压下健康检查响应。它明确要求环境变量 `TEST_DATABASE_URL`，不读取 env 文件，并创建/删除隔离数据库：

```powershell
cargo test login_protection_tests -- --ignored
```

参数调整后应结合实际 CPU、内存与可观测性数据验证，不能仅凭默认值估算生产承载量。
