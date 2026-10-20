# Phase Execution Result — Phase 22

状态：**COMPLETED**。选型结论：**SOCKET_IO_COMPATIBLE**。固定 `socketioxide 0.18.7` / `engineioxide 0.17.7`，条件 Phase 37 未触发。用户明确要求完成后只标记任务完成，**本次不修改或推进执行指针**。

## Executed Phase

Phase 22：验证 Rust 实时协议兼容并冻结选型。前置 Phase 1、4、6 均已 COMPLETED，本次不要求依次完成 16–21。共享工作树包含其它阶段的既有及并行改动；这些改动不归为本阶段交付，验收仅声明下面的输入与能力。

## Execution Summary

用当前宿主安装的 `socket.io-client 4.8.4`、当前通知 DTO 校验器，以及聊天产品的 `createChatSocket`，连接两个真实 Rust peer。分别实测 polling、直接 WebSocket、polling→WebSocket Upgrade、事件认证、拒绝与到期、心跳、断线重连、Origin 和 namespace 错误。没有以原生 WebSocket 替代 Socket.IO，也没有启动 Node 后台冒充 Rust fixture。

最终 Linux r5 的 16 个命令步骤全部退出 0。真实协议 TAP **44/44**：通知 35 个子项、聊天 7 个子项和两个父测试，零失败/跳过/取消/todo。两 crate 共 **7 个单元测试**通过；聊天前端构建与 **5 项**测试、受影响 CI/通知/session 的 **48 项**契约均通过。没有把前轮失败改写成 PASS。

## Deliverables

- [通知协议 Rust fixture](../../tests/fixtures/socketio-rust/README.md)：独立 crate/lock、薄入口、loopback 限制、严格事件对象、合成 token/到期、完整身份房间与 Origin 校验。
- [Rust 聊天参考](../../examples/websocket/backend/Cargo.toml)：独立 crate/lock、配置/纯聊天室/Socket.IO 适配器/薄入口；保留 `/` 和既有聊天事件，默认回环 3008。旧 `examples/websocket/server.ts` 已删除。
- [真实客户端矩阵](../../tests/phase22-socketio.mjs)与[进程测试驱动](../../tests/helpers/phase22-fixtures.mjs)：锁定安装的客户端、实际 TCP、有限超时、事件前挂监听、stdin EOF 清理。
- [依赖选型与支持矩阵](../realtime-protocol.md)：协议版本、许可证、分支决策、阶段边界与回退复验要求。
- 根 `npm run test:realtime` / `npm run dev:chat:backend`；正式 fast 增加两个 Rust unit 步骤及真实兼容矩阵。聊天 Vite 拒绝 `/backend/**`，现有 HTTP 测试验证源码 404。
- 同步根/聊天/通知 README、工程基线及现状架构；[台账更新建议](phase-22-ledger-update.md)完整记录本阶段终态，不推进执行指针。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 当前真实客户端与候选 Rust 的传输/认证/重连矩阵确实执行，结果及失败原因可复核。 | PASS | Linux r5 44/44；当前客户端真实加载，三种传输及 WS/polling 拒绝/到期/心跳/重连均执行。r2–r4 原始失败保留。 |
| SOCKET_IO_COMPATIBLE 所有必需项通过；若 NATIVE_WS_REQUIRED 列出关键不满足与37输入。 | PASS | 选定前一分支。通知35、聊天7及父测试全部通过；无须启动原生WS分支。 |
| 缺环境保持真实未完成/BLOCKED，不以未知失败选择fallback。 | PASS | Windows Rust入站回环ETIMEDOUT记为未执行网络矩阵，改在受控Linux实测后才决策；不据此宣称库不兼容。 |
| 选型与台账条件触发同步，未完成37不宣称WS适配或通知迁移完成。 | PASS | Phase22记录SOCKET_IO_COMPATIBLE；Phase37未触发、状态不变；生产通知仍为Node，Phase23未由本次实施。指针按用户要求不改。 |

## Source Provenance

HEAD：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`。这不是纯 Git HEAD 验收：工作树含未提交内容，实际运行输入按逐文件 SHA256 记录。

| Evidence | Actual value |
| --- | --- |
| Linux r5 完整快照 | [604项清单](../../target/phase22/linux-r5/source-manifest.json)，SHA256 `fa5f14d2d0dd6232b8e473422ddd4afee91a1035cbe81544f80237035ca2a320` |
| Phase22 相关输入 | [63项归档](phase-22-inputs.json)，SHA256 `d1355666f4220cedc83277278ce505d1f3e417397cc7304cb4cf3538ed6782a7` |
| r5刚结束时的跟踪字节 | [63/63 匹配证明](../../target/phase22/linux-r5/current-phase22-proof.json)，当时changed=[] |
| 最终收尾的阶段边界 | [边界证明](phase-22-boundaries.json)：62项受保护输入匹配，SHA256 `1b3bd761e56cf8f03c65cdcb5d678ac97b2f80f212af0780aca0c4dd540e89d9`；另1项是未在本阶段执行的并行OAuth脚本变化 |
| 原始执行及清理 | [r5结果](../../target/phase22/linux-r5/result.json)、[实际命令](../../target/phase22/linux-r5/executed-commands.json)、[退出码](../../target/phase22/linux-r5/reports/exit-codes.txt) |
| 工具 | [实际版本](../../target/phase22/linux-r5/reports/tool-versions.txt)：Node24.18.0、npm11.16.0、Rust/Cargo1.97.0、Docker29.1.3 |
| Linux 工具镜像 | `sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7`，linux/amd64 |

完整清单也含共享工作树的其它阶段输入，因此不把604项与上阶段的全量差异当成本阶段改动。63项跟踪清单选择库、测试、当前客户端、锁、生成协议与CI文件。最终收尾时只有 `scripts/ci/oauth-playground-delivery.mjs` 被其它工作修改；它未被本阶段矩阵或选定契约导入/执行，保留原快照并单独分类，不覆盖文件、不归因于Phase22。其余62项与实测字节一致；文档和台账补记不改变已实测程序。归档索引见 [运行摘要](phase-22-runtime.json)。

## Executed Commands and Results

所有网络行为在自有 Docker 私网容器中运行；没有宿主目录、个人环境文件、Docker socket 挂载或公开端口。源码由干净快照 `docker cp` 进入；npm/Cargo 仅访问公开依赖仓库，业务 token 都是 fixture 合成值。

| Command / group | Result | Evidence |
| --- | --- | --- |
| 三个 `npm ci --ignore-scripts --no-audit --no-fund`：frontend、examples/websocket、scripts/ci | PASS，锁安装退出0 | reports/npm-frontend.log、npm-chat.log、npm-ci.log；scripts/ci是实际单独docker exec准备步骤，准确命令另列 |
| `node --test --test-reporter=tap tests/phase22-socketio.mjs` | PASS，44/44，零skip | [原始协议日志](../../target/phase22/linux-r5/reports/tests.log) |
| 两个manifest的 `cargo fmt -- --check` / `cargo check --locked --all-targets` | PASS | reports/notification-{fmt,check}.log、chat-{fmt,check}.log |
| 两个manifest的 `cargo clippy --locked --all-targets -- -D warnings` | PASS | [fixture Clippy](../../target/phase22/linux-r5/reports/notification-clippy.log)、[chat Clippy](../../target/phase22/linux-r5/reports/chat-clippy.log) |
| 两个manifest的 `cargo test --locked --all-targets` | PASS，fixture2 / chat5，零ignored | [fixture units](../../target/phase22/linux-r5/reports/notification-test.log)、[chat units](../../target/phase22/linux-r5/reports/chat-test.log) |
| 两个manifest的 `cargo doc --locked --no-deps` | PASS | reports/notification-doc.log、chat-doc.log |
| `npm --prefix examples/websocket test` | PASS，typecheck/build及5项静态交付测试 | [前端日志](../../target/phase22/linux-r5/reports/chat-frontend.log) |
| `node --test --test-reporter=tap tests/ci-runner.mjs tests/ci-workflow.mjs tests/notification-context.mjs tests/session-integration.mjs` | PASS，48/48 | [契约日志](../../target/phase22/linux-r5/reports/affected-contracts.log) |
| Windows两个crate fmt/check/clippy/test/doc，check/clippy/test带all-targets/all-features | PASS，十个命令退出0 | [最终Windows检查清单](../../target/phase22/windows-final-rust-checks.json) |
| 锁定依赖许可证/MSRV与OSV公开漏洞查询 | PASS，101个registry package，无返回漏洞；chat无额外未覆盖包 | [依赖记录](../../target/phase22/dependency-selection.json)、[OSV响应摘要](../../target/phase22/advisory-check.json)、[chat覆盖](../../target/phase22/advisory-coverage.json) |

没有运行全仓 fast/PG/browser/delivery；本次选择的实际程序与契约全部运行，CI步骤接入另经现有政策测试核验。没有修改平台Rust代码，不继承旧阶段全仓fmt/Clippy债务为本阶段通过。

## Compatibility Findings and Repairs

- `serde(deny_unknown_fields)` 不拒绝结构体的位置数组表示。为认证、聊天join/message明确增加object门禁，并用实际客户端及Rust单测证明数组拒绝，未放宽公开DTO。
- socketioxide异步闭包先spawn再执行，原首包布尔锁会被调度顺序反转。改为普通闭包在按包顺序构造future时同步消费一次尝试并完成合成认证；真实并发事件回归保持原断言。外部平台await的延迟/取消/晚到结果仍由Phase23验收。
- WS与polling分别观察3次真实ping/pong后再投递；transport close后生成新socket身份并重新authenticate。unauthorized后的`io server disconnect`使当前真实客户端停止重连。
- 同用户跨租户、同用户同租户跨组织均独立验证；坏Origin的upgrade拒绝后，合法polling连接仍能接收当前身份通知。
- 客户端的`["websocket","polling"]`列表本身不提供初次失败的自动fallback证明，未修改`tryAllTransports`或将HTTP收件箱恢复冒充协议回退。

## Failure History

| Run | Preserved result and repair |
| --- | --- |
| Windows | Rust进程ready后，Node/curl连接Rust回环端口ETIMEDOUT；同驱动Node→Node正常。记为环境阻碍，未从该结果选择fallback；静态/Rust单元检查另行通过。 |
| Rust前验 | 初版同步handler不符合0.18.7异步API，修正后编译；认证数组回归先真实失败再补object门禁。未将这些实现错误归为协议不可兼容。 |
| Linux r1 | 只生成快照；Git dubious ownership阻止driver取得HEAD，未运行协议测试。改为单次git safe.directory参数，不改全局配置。 |
| [Linux r2](../../target/phase22/linux-r2/result.json) | FAIL。测试误用Manager.close、首包并发顺序、聊天广播观察时序和漏装scripts/ci yaml依赖分别定位；保留原日志。仅终止owned容器内本次挂起驱动，继续记录退出和清理。 |
| [Linux r3](../../target/phase22/linux-r3/result.json) | 旧harness快照受控终止，实际exit143；不补写PASS。 |
| [Linux r4](../../target/phase22/linux-r4/result.json) | FAIL。通知35项通过；旧聊天广播同步与缺少yaml准备依赖仍失败，保留原状态。 |
| [Linux r5](../../target/phase22/linux-r5/result.json) | 最终修复程序及正确锁安装依赖，全部16步骤PASS；协议44、unit7、前端5、相关契约48，零skip。 |

r2–r5各自owned容器和网络均已删除，标签查询为空；r5无host ports/mounts。Windows两个Rust测试peer进程检查为空；root持有的WSL lease已结束、退出0。没有操作生产资源。

## Risks / Notes

当前生产通知仍为Node，没有真实平台鉴权、PG LISTEN、逐投递撤权、分区队列、容量、Nginx/TLS或真实浏览器hook/UI cache联合验收。通知fixture的token与expiresAt是合成值；已过期token是合成拒绝分支，不能说已证明真实平台token失效。

Socket.IO客户端矩阵证明当前受测客户端的可观察关闭/恢复语义；未声明对任意恶意raw Engine.IO连接的物理关闭或资源容量保证。此类服务运行边界连同异步平台验证应在Phase23实现并验收。生产切流、镜像发行和其它阶段均未由本次执行。

回退聊天参考不涉及数据库；通知没有切协议，现Node路径继续可用。库或客户端升级需要重跑矩阵。本阶段无剩余验收项；Phase23可消费本结论，但不会自动启动。执行指针整个区段按用户要求保持，若其中保留旧`realtime_protocol_result`缓存字段，本阶段实际决策以Phase22详情及本证据为准。
