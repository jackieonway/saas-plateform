# Phase Execution Result — Phase 23

状态：**COMPLETED**。选型保持 **SOCKET_IO_COMPATIBLE**，Phase37未触发。按用户要求，本次仅标记Phase23任务完成并同步证据，**整个执行指针区段保持原样**；逐字节边界证明见[台账边界](phase-23-boundaries.json)。

## Executed Phase

Phase 23：迁移 Rust 通知服务并实现有界分区调度。前置 Phase 4、6、10、12、22 已完成；沿用 Phase 22 的 `SOCKET_IO_COMPATIBLE`、socketioxide0.18.7/engineioxide0.17.7，Phase37未触发。共享工作树的工单、OAuth、CLI 等并行结果保留，不将其归为本阶段变更，也不推进其状态或执行指针。

## Execution Summary

`mini-services/notification-ws` 现为独立 Rust crate 与 Cargo.lock，薄 main/窄 lib 将配置、协议、平台 HTTP、只读 DB/LISTEN、调度、传输和服务生命周期分开。旧 Node 后台、入口及旧实现测试已移除；npm 只保留真实平台/PG、Socket.IO 客户端与浏览器测试工具。生产镜像和正式 notification Compose 使用 Rust 二进制，不携带平台 JWT 签名密钥。

实时消息是已持久通知的提示。每次投递都读取对应归属行并向真实平台附 notificationId 复验，不增加长期正向权限缓存；过载、断线或不确定授权丢提示，由 HTTP 恢复完整收件箱，不重新发布、不替换幂等键。

## Deliverables

- [Rust 服务](../../mini-services/notification-ws/Cargo.toml)：独立锁文件、私有实现模块、唯一公共启动出口、固定 Socket.IO 协议、精确 Origin 与路径/SID 准入。
- [配置和容量说明](../../mini-services/notification-ws/README.md)：拒绝 JWT_SECRET、不加载平台.env、正常 TLS/可选额外 CA、只缩短到期、逐投递/周期校验和独立 expiry watch。
- [分区调度](../../mini-services/notification-ws/src/scheduler.rs)：user/tenant/org 分区 FIFO，单键单处理器，全局 ready 公平轮转，pending/分区/worker 分别限额；取消和 panic 回收有回归。
- [可撤销物理传输](../../mini-services/notification-ws/src/transport.rs)：Engine 准入与实际 TCP 分别限额，退役后100ms flush再关闭当前连接；keepalive 所有权可转移且关闭不可撤销，弱引用元数据不保留 TCP 许可；全局关闭锁存并唤醒 idle/不完整HTTP，Engine关闭与刷新后全TCP撤销共同await。
- [正式 Compose](../../deploy/compose.notification.yml)、[CA overlay](../../deploy/compose.notification-ca.yml)、[只读角色 SQL](../../deploy/notification-role.sql)：12列 SELECT/LISTEN，非root/只读/私网，legacy override 仅保留工单；旧通知容器需按原部署清单精确停止。
- [真实联合入口](../../scripts/ci/notification.mjs)：构建当前平台 main、Rust 通知与受控 Rust 延迟代理；完整迁移和665923行地区 seed、真实 session/app token、发布/幂等/权限/撤销/恢复/调度和边界。
- [真实首次认证竞态](../../mini-services/notification-ws/tests/rust-auth-races.mjs)：实际平台 HTTP 已进入 gate 后重复认证、首次等待超时、同tick先后顺序、非法首事件与真实应用Bearer拒绝；gate只延迟真实请求，不伪造认证结果。原session保持有效时旧连接仍不能复活，新连接重新认证后接收真实持久通知。
- [宿主双页测试](../../tests/phase23-notification-browser.mjs)：实际宿主页、真实平台响应、真实组织切换与断网恢复；延迟旧 HTTP 仍使用真实响应验证客户端代次取消。
- 工单与原生恢复的旧 opt-in E2E 通知启动适配显式要求预编译 `NOTIFICATION_E2E_SERVICE_BINARY`，验证绝对普通可执行路径，直接 owned launch/restart Rust，不再导入旧Node模块或从平台发行包推导通知入口；三项纯适配器回归纳入CI，未执行未来完整工单/灾备阶段。
- [镜像验收入口](../../scripts/ci/notification-image.mjs)及 fast/PG/browser/delivery 门禁；集中报告校验拒绝缺项、零测试、skip、错误产品、过时输入和清理失败。CI 源快照包括本服务，上传仅允许脱敏报告、日志与指定截图。

## Identity and Resource Bounds

`authenticate` 只接受严格 `{token}`，首次尝试按收包次序消费；token、user/tenant/org/sessionId 由平台确认，不能二次换身份。公开 authenticated 不含 sessionId/token。消息特定403/404或临时验证失败只丢当前提示；401、身份改变、已知到期、初始/周期无法确认则退役。已知到期由独立 watch 检查，不能等慢 HTTP；后续成功只允许缩短 expiresAt。

默认配置中，queued hints≤1000、active handlers≤8、驻留分区≤128、每分区 pending≤32、平台 HTTP≤16、Engine≤256、TCP≤4×256+16、query pool≤4、LISTEN≤1、每连接两个 watch、Socket.IO buffer≤32、输入/frame/message≤8192字节、平台响应≤4096字节。驻留提示上界是 pending+active；不能把 worker 内在途提示也算入 pending 或宣称进程 RSS 固定。所有校验共用许可，deadline 包含许可等待、header/body；队列满只丢提示，持久收件箱不变。

namespace 断开、物理 TCP 关闭和 Engine 内部元数据回收分开核验。raw peer 忽略41时仍必须关闭实际 IO；退役 SID 的新 TCP 请求不再进入 Engine handler。polling Engine 许可可能等待库的 ping/timeout 回收，初次准入仍受 C 限制；报告会记录真实新 open 从503恢复200的窗口。

## Source Provenance

HEAD 为 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，工作树包含其它阶段未提交内容，不能只用 HEAD 代替实测源码。最终统一 Linux r7 冻结682个文件，完整输入 SHA256 为 `1bc2fbbae7e62c5691e781b33612b707ee085f7aaf3ef7d5c7af63bf70961eee`；完整清单保存在 `target/phase23/linux-r7/source-manifest.json`。阶段相关逐文件来源见[分组清单](phase-23-inputs.json)，相邻阶段文件只用于运行平台/宿主，不归为本次修改。

| 运行输入组 | 文件数 | SHA256 |
| --- | --- | --- |
| Rust通知Cargo/src | 12 | `39b41439bc5c4db910036d7de685a3887c65c2c4a79195990ac464eef8ea901d` |
| Rust真实上游延迟代理 | 9 | `7db75672b5948c352523c3d27663e350dbbb7239bcd7275f5b477afa2d7bbbd1` |
| 宿主/浏览器相关输入 | 186 | `7d2cba4065e4956800869f91b635a6afe66f8fcd076a310bd071d40a552b4e3c` |
| 实际平台实现/迁移 | 67 | `b0fa65c1977f7b3219462c8bed4e4b1901b6f62227e94b588638340102d4601f` |
| 锁定JS/联合驱动 | 13 | `b5cc60a5015cb287a12c1c26a78108626f14125944c907121ee358ec8b514a96` |

通知镜像独立14项Cargo/Rust/Docker输入为 `2127b07f61cb6417da0253bf4b5c331efd1155e25f862c6be09713ad8833148d`，与生产crate的12项逐文件对应。本地image ID为 `sha256:cac73730a08fcca8587e2227aea72c22f67ae384f0f98ee12e23cb322b829541`；它不是registry digest。相关源文件、基镜像identity、安全配置、信号及清理结果见[镜像归档](phase-23-image.json)。

## Executed Commands and Results

最终统一 Linux r7 的8个必需步骤全部PASS：真实平台/PG/认证竞态/调度/边界 **34/34**（30个子项及4个父项）、实际Chromium双页 **1/1**且6个命名检查全过，零失败/跳过/取消/todo。真实工具、命令、时延、源码及清理归档见[运行摘要](phase-23-runtime.json)。通知生产镜像9项实测通过，源码输入对应，SIGTERM退出0/2992ms；详见[镜像结果](phase-23-image.json)。最终源码已通过 Windows Rust fmt/check/Clippy/test/doc，29项单元测试零失败/ignored；日志为 `target/phase23-rust-final-r3.log`，SHA256 `0a0c18d83b1b42261e721575f250893b96b43f8099175276b461c9414bf55433`。CI报告/工作流/通知E2E启动策略、Linux runner/原生构建/通知上下文与共享会话回归合并87项通过，零跳过，见 `target/phase23-policy-combined-final.log`。开发调度和实际 Compose 另验，不将 Windows 的 POSIX-only skip 算作 Linux 网络验收通过。

| 检查组 | 结果 | 原始证据 |
| --- | --- | --- |
| 独立Rust fmt/check/严格Clippy/test/doc | 五门禁PASS；29unit、0failed/ignored | target/phase23-rust-final-r3.log |
| 锁安装、三个Rust --locked构建、四file真实平台PG联合 | 8步骤全部PASS；联合34/34 | target/phase23/linux-r7/reports/integration/notification-result.json |
| 实际宿主双页Chromium156.0.8078.4/Playwright1.64.1 | TAP1/1、6命名检查、两个document不变；迟到真实HTTP200→生产AbortError | target/phase23/linux-r7/reports/integration/two-page-browser-chromium.log |
| CI/runner/native构建/通知上下文/共享会话与旧E2E适配回归 | 87个不同用例通过；最终CI子集29再验通过，无skip | target/phase23-policy-combined-final.log、target/phase23-ci-policy-final-r2.log |
| 部署策略与角色SQL | 4项含实际Compose解析；12列/LISTEN允许、users/通知写入拒绝 | target/phase23-policies-final.log、phase-23-role.txt |
| 修复后生产通知镜像 | 9/9、当前14项源输入匹配、无Node/非root/只读/私网/指标鉴权 | phase-23-image.json |
| 完整输入与资源清理 | 682/682及5组/9共享导入匹配；自有库/角色/容器/网络归零，WSLlease exit0 | phase-23-runtime.json |

可复现的主要入口：

```sh
cargo fmt --manifest-path mini-services/notification-ws/Cargo.toml --all -- --check
cargo check --locked --manifest-path mini-services/notification-ws/Cargo.toml --all-targets --all-features
cargo clippy --locked --manifest-path mini-services/notification-ws/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --locked --manifest-path mini-services/notification-ws/Cargo.toml --all-targets --all-features
cargo doc --locked --manifest-path mini-services/notification-ws/Cargo.toml --no-deps
node scripts/ci/notification.mjs --run-tests --out-dir /absolute/new-report --browser chromium
node scripts/ci/notification-image.mjs --out-dir /absolute/new-image-report --source-revision FULL_GIT_SHA
```

联合入口必需 owned `TEST_DATABASE_URL`/`CI_TEST_POSTGRES_OWNER`，自动构建三个真实 Rust 二进制；缺输入失败，不跳过。Windows 当前 Rust 入站回环受阻，真实网络行为放在自有 Linux Docker 私网，通过快照复制源码，无宿主端口、源码/个人环境或 Docker socket 挂载。Node24.18.0/npm11.16.0、Rust/Cargo1.97.0、Docker29.1.3、PostgreSQL17.6为实际工具基线。

[部署角色SQL实测](phase-23-role.txt)在自有PostgreSQL17.6执行，12列读取与LISTEN成功，users查询及通知INSERT/UPDATE/DELETE拒绝，默认只读事务开启；自有数据库容器已回收。部署策略4项含实际Compose解析通过，配置模板与运行时分别验证。

## Measured Windows and Recovery

以下为最终Linux r7实测，配置用于隔离验收，不作为生产SLA。

| 场景 | 配置与实测 |
| --- | --- |
| 删成员后下一条真实投递 | 周期复验设60000ms，实际带notificationId复验35ms拒绝；未等周期 |
| 无通知提示的真实注销 | recheck1000ms/HTTP2000ms，从注销请求前计时911ms断开；测试上限7000ms |
| 已知到期遇慢校验 | HTTP5000ms、proxy12000ms、DB到期3000ms；到期后12ms退役，无迟到投递 |
| 缩短到期后DB再延长 | 保持已捕获较短期限，迟后19ms断开，即使真实平台仍认可延长后的会话 |
| 首次A校验pending后重复B | A请求1/B请求0；30ms退役，4个新Engine可占满容量，第5个真实拒绝 |
| 首次认证独立等待 | 等待1000ms、HTTP5000ms、proxy12000ms；从发送事件后观察988ms拒绝，原平台session仍有效 |
| polling退役与容量回收 | 声明100msflush，测得150ms后旧SID新TCP GET/POST均ECONNRESET；合法peer保持在线，新open503→200耗时19981ms，上限50000ms（公告ping25000/timeout20000） |
| 队列饱和与慢上游 | active2、pending4、partition3、HTTP high-water2；丢24个提示，HTTP恢复全部24条持久行；5000ms deadline实测4987ms释放worker |
| 不合作HTTP连接停机 | idleTCP1+不完整HTTP头1，EOF触发同一生命周期111ms正常退出，上限7000ms；生产镜像另验SIGTERM2992ms/exit0 |

宿主两页共享真实session；切组织intent同步停止两页旧scope/传输，真实迟到旧HTTP响应被生产请求边界取消，切换后只收新组织。仅中断通知传输后，两个原document在重连中保留，真实HTTP收件箱/未读/偏好恢复，离线历史不弹Toast。实际只执行Chromium；Chrome/Edge保持受管CI必需项，但未冒记为本轮产品实测。

## Dependency and Boundary Audit

[依赖与公开公告归档](phase-23-dependencies.json)记录267个 registry 包及本地 crate，11个运行直接依赖、0个开发直接依赖；未使用的 tower 直接依赖已移除。无平台业务 crate/path/git 依赖，服务只通过 socket-context 取得身份。最高已声明依赖 MSRV1.94.0；SQLx 未声明，未测试最低编译器。当前使用 Rust1.97.0。

OSV 对全部267个精确版本完整查询：锁文件 `rsa0.9.10` 命中 [RUSTSEC-2023-0071](https://osv.dev/vulnerability/RUSTSEC-2023-0071)。它属于未启用的 SQLx MySQL 可选分支；当前选定 feature 的全目标活动树233个 registry 包，rsa/sqlx-mysql均不可达，活动匹配0。报告保留 FINDINGS，不宣称全锁 clean；功能变化须重查可达性。去掉 tower 后版本集合逐项相同，复用同日原查询并明确记录时间。cargo-audit/deny/machete未安装，未冒记这些工具通过；没有覆盖系统镜像漏洞或法律许可证审查。

## Failures and Corrections

- Windows Rust TCP 入站回环超时是环境限制，不作为协议不兼容或跳过必需测试的依据。
- Linux r1 在768MiB PG tmpfs配置下平台于 ready 前退出，未取得能确认根因的完整平台日志；r2改为自有2GiB后完整665923行 seed和平台 ready得到实证，正式 CI 同步2GiB，不绕过初始化。
- r2测试仍断言历史0022迁移，而当前源码只有0001/0002/0003；改为对实际 SQL 文件逐条校验版本、success和checksum，未修改迁移内容。
- r3实际联合24通过/2失败计数（一个业务子项和其父项）：普通租户根上下文的 TENANT grant 被当前平台收敛到 ORG_SUBTREE，测试不应预期 TENANT 发布成功。修正为真实403且零写，再用同token发布允许的子组织并验证隔离，未放宽平台规则。
- 首轮 fmt发现新 guard 代码换行未符合 rustfmt，已格式化；CI证据上传策略首次拒绝新增通知路径，已扩展精确脱敏白名单并补拒绝 source/work 私有目录的反例。历史失败保留，最终结果单独记录。

- r4真实平台14个子项与调度3个子项全部通过；新增 raw polling 测试错误假定 unauthorized/41 必须在同一 HTTP body，修正为按实际分包读取。另一新增测试发现 idle TCP/不完整HTTP请求头使退出超过7000ms，属于实际生命周期缺陷。已新增全局有界弱引用TCP registry与关闭锁存，Engine关闭及100ms后全TCP回收共同await；补两项唤醒、许可回收与千次循环元数据回归，最终五Rust门禁及29unit已通过。r4仍保留FAIL，修复后r7全套真实复验通过。
- r5已实证idle/不完整HTTP在112ms退出，真实平台与调度继续通过；raw polling测试在100ms刷新窗口内发送未被请求的pong后出现非预期HTTP状态，原断言未记录实际status，未将协议错误推断写成已确认根因。仅调整测试，在声明窗口后先GET再POST并记录实际status，仍拒绝500与测试自身timeout，不放宽服务契约。r5保持FAIL；修订测试单独9/9通过，r6旧三file统一28/28+浏览器通过；新增认证race先6/6通过后纳入r7四file34/34统一验收，均独立保留原报告。

## Risks / Notes

HTTP 补拉是持久数据恢复，`[websocket,polling]` 也不保证初次 WS 失败自动降级 polling。没有把补拉宣称为该传输能力。周期失效窗口由配置、抖动、HTTP deadline与实际调度共同决定，默认值不是生产 SLA；本地小容量测试不替代正式负载容量规划。

本次执行独立通知门禁及所列受影响回归，未将整仓其他阶段门禁或远端GitHub执行宣称为通过。临时数据库/只读角色均归零，精确owner容器/网络标签查询无剩余，WSL保活正常退出；本地候选镜像保留供复核。

本次只构建本地候选镜像、隔离 PG/平台/浏览器及受控运行；没有推送 registry、操作生产部署、真实业务数据或发出外部消息。候选 image ID 不写成 registry digest。正式采用须核对不可变镜像、真实HTTPS/CA/Origin/网关，并精确停止旧 Node 服务；宿主旧静态资源保留升级窗口，不破坏既有 wire。停止实时或回退兼容 Rust 镜像不改变持久收件箱。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 删成员/踢出/过期后下一次投递拒绝，实测周期窗口内断开，无跨租户组织消息。 | PASS | 真实平台、持久行及Socket.IO验证；独立逐投递35ms、无hint注销911ms、expiry交叉12ms；明确配置与上限。 |
| 慢复验不阻塞其他收件人；并发/队列/内存有界，LISTEN和HTTP完整恢复。 | PASS | FIFO与公平调度、独立worker/HTTP预算、24条过载持久行完整；真实断LISTEN后readyDOWN/重连UP及新提示恢复。 |
| 所选Socket.IO polling/Upgrade、到期、live/ready和优雅退出实际验收。 | PASS | 强制polling/Upgrade、rawWS/旧SID新TCP拒绝与真实EIO许可回收；HTTP半头111ms和生产镜像SIGTERM2992ms。 |
| 双页切组织停止旧身份；生产通知入口无Node，不冒称自动polling回退。 | PASS | 实际两页6检查；Rust镜像9检查；旧Node入口/Compose/活跃测试导入退役；HTTP补拉单独定义。 |

全部选定阶段必需验收完成，无待办或新增业务问题；下一阶段不自动执行。完整结构化台账更新见[更新建议](phase-23-ledger-update.md)。
