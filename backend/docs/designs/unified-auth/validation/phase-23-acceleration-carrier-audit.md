# Phase 23 旧载体证据链并行审查

日期：2026-10-29。范围：当前源码的 checkpoint、原始载体保留、恢复后拒绝和标准 SDK 缓存时序。本文是静态审查及纯组件复验记录，不是 Phase 23 动态验收或恢复通过报告。

本审查不连接 PostgreSQL、不启停服务、不创建或复用容器，不修改环境 owner 的文件。`SCHEMA_COLUMN_DRIFT` 由现有恢复代理处理，不在此重复。

## 确认发现

### P2：拒绝请求跨过自然到期时仍可能被计为恢复撤销

`tests/unified-auth-https/phase23-browser.mjs:503`、`:505`、`:507`、`:509`、`:512` 只在提交旧载体前检查截止时间。随后真实网络请求可能跨过截止时间，但断言只检查 401、inactive 或授权码 400/401，未在响应结束后再次确认同一载体仍未自然到期。Access deadline 还来自 `phase23-protocol.mjs:421` 的 TokenData；USER 在独立 RP 的 `src/oidc.mjs:56`、MACHINE 在 `phase23-protocol.mjs:493` 以收到响应时的 `Date.now() + expires_in` 计算，本身也不等同原库的准确 `expires_at`。

复现条件：门禁检查时仍剩少量真实寿命，服务器处理 introspection/UserInfo/授权码请求时已经自然到期。当前脚本会把自然到期产生的拒绝计入恢复后的旧载体撤销证据。这个问题不会使产品接受旧载体，影响的是验收证据的归因。

纯复现直接导入当前 `createDriver`，注入一个 30ms MACHINE token 和延迟 70ms 的 introspection；不使用 socket、数据库、服务、文件或时间替换。实际输出：

```json
{"beforeUnexpired":true,"responseInactive":true,"afterExpired":true,"currentBrowserChecksWouldPass":true}
```

建议修复：对同一载体记录准确原截止和权威时钟，拒绝响应后再次检查截止；需要覆盖多次请求时逐次检查，不在一次门禁后合并计算。若截止已过，输出固定“证据窗口耗尽”失败，不能算撤销通过。Access 截止可以由私有 protocol 层把原始 token digest 与当前环境 owner 的受控查询关联，仅返回截止，不输出 token；若继续使用 SDK 推算期限，应明确保守界限并在响应后检查。授权码已有原库准确期限，直接补响应后的权威时钟核对即可。BFF、业务 bearer 和 SSO 同样补响应后检查。

## 本轮纯组件验证

在 Windows Node 上执行：

```powershell
node --test `
  saas-plateform/backend/tests/unified-auth-https/phase23-protocol.mjs `
  saas-plateform/backend/tests/unified-auth-https/phase23-key-cases.mjs `
  saas-plateform/backend/tests/unified-auth-https/phase23-invitation-cases.mjs `
  saas-plateform/backend/tests/unified-auth-https/phase23-policy-cases.mjs
```

44 项通过、0 失败、0 跳过。覆盖当前进程内原始 carrier 与旧凭据跨 reload 保留、C/D 缓存预发布编排、真实 SDK 空 GET 适配、公开 subject 对比、密钥窗口、高水位、邀请 receipt 和额外客户端收缩的组件行为。这些纯测试不证明实际 HTTPS、真实保留窗口、PostgreSQL 恢复或最终资源清理。

## 其余审查结论

- browser 的 protocol 只启动一次；恢复后通过 `reload` 替换当前配置，原始 flow/session/credential 仍保存在同一进程。恢复拒绝验证结束前没有显式重启 RP 或 collector。
- 正常退役依据数据库 `last_signed_exp + 60` 的严格截止，预发布检查 SDK 的公开 cache age，未发现清空缓存或修改时间以规避 SDK 行为的路径。
- 恢复前先 capture-deny、隔离源库；目标依次 restore/reconcile/rearm/gate，应用池启动前完成新旧密码 SCRAM 对照。当前静态检查未发现上述顺序中的新增确定运行阻断。
- 当前动态主任务仍为 `IN_PROGRESS`。不得根据本文将四条 Phase 23 验收勾选通过。

## 资源边界

本审查创建/复用容器均为 0，没有临时数据库、角色、服务进程、私钥或备份文件，因此没有本审查需要清理的 Docker 或运行资源。未测量或声称主任务磁盘回收；主任务的环境与最终清理由唯一环境 owner 记录。

## 同轮修复与独立复核

上述 P2 证据缺口已在当前源码修复：新增 `phase23-carrier-window.mjs`，每次提交旧载体前后使用同一个原始截止和真实时钟观测，拒绝请求期间自然到期或时钟回拨；原请求异常保持原样。根代理在 browser 以协议进程保留 token 的 SHA256 关联源库 `auth_access_tokens.token_digest`，在 checkpoint 前保存精确 `expires_at`，恢复后逐次请求使用目标 PG 的 `clock_timestamp()` 进行前后核验。USER current 与 C/D/MACHINE original 的版本绑定均已独立检查；MACHINE 的 introspection 与资源请求分别受门禁保护。授权码、SSO、两 BFF 和业务 bearer 也已包装。

独立复核时发现 Windows browser 的新增 helper import 最初未加入 PREFLIGHT 冻结哈希白名单，根代理已补齐，并已复查确认。四个 helper 固定错误代码统一使用 `PHASE23_` 前缀，直接兼容现有 `safeFailureCode`；4/4 直接适配检查通过，不会退化为泛化失败或输出原始 deadline、token 或 digest。

修复后的 helper 9 项与 protocol 16 项合计 25 项纯测试通过，browser `node --check` 通过。新 helper SHA256 为 `7f37a022ab90e76cf1f972104e8b6311a5b291965c72b856a56433fd348fc011`，其测试文件 SHA256 为 `fa69d1e37274f36024f01d31b97f3fbd557db4b1bea5393d2b74388b84d4103e`。未发现新增的 token 版本错误、未绑定 import 或固定诊断泄漏；没有新增确定后半程运行阻断。

这些检查仍为当前源码、纯组件和静态集成证据。必须用包含本次修复的新冻结工具快照完成真实 HTTPS／备份恢复／旧载体拒绝和最终清理，才能作为 Phase 23 动态验收；本报告不改变台账 `IN_PROGRESS` 状态。
