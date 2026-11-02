# Phase 18：宿主、SDK 与通知上下文变化

日期：2026-10-28。状态：COMPLETED。Phase13、14、16、17依赖已完成。首轮组件通过后，主代理及独立复核确认并修复宿主切换消息与私有通知收件人恢复两类P2；修复后的动态门禁、独立复核和当前源码摘要均已收敛。当前结果见 [components JSON](phase-18-components.json)；初轮37文件清单仅作 [历史来源记录](phase-18-initial-source-manifest.json)，不作为修复后通过证据。

## 宿主切换提示 P2 修复

已先复现两类失败：异会话／无绑定终止提示可解冻当前屏障（4条 RED）；只比较会话和版本仍不能区分同版本更早的切换或新标签页初始化消息（3条 RED，其中包含真实 `restore`／`notify` 发送点）。修复对每次切换生成128位 CSPRNG `switchId`，屏障和终止提示以原 `sessionId/revision/switchId` 三元组匹配，不能用新 view 的三元组替代原操作。普通初始化、关联值缺失／错误、同版本旧操作结束均只触发权威读取，不能提前解除当前屏障。服务端 GET 确认的新会话或新版本仍可通过完整 bridge 证明后更新。

完整身份跳转仅在 POST 的受控 `returnPath` 注入单个非授权相关查询参数 `__platform_context_switch`。返回宿主时模块初始化检查格式、单值和90字符上限，立即 `replaceState` 移除后才允许业务页面及请求观察 URL。该标记不是凭据或权限，不持久化 token／CSRF，不进入业务缓存，不允许外站返回；即使正确格式或关联值被伪造，也不能替代服务端 Cookie／来源／上下文证明。未匹配返回标记的普通新标签页不会宣告另一页切换完成。

外来提示不能中断本标签页进行中的切换 POST，但并行外来切换仍须记录。待定集合最多32项，超限不淘汰未结算项而保守暂停；已结算记忆最多128项，淘汰只能额外冻结、不能授权。不同版本逐项比较：A/r1 与 B/r2 并行时，GET 到 r2 仍须等待 B，不能因 A 已过时清整组。明确4xx拒绝后，认证来源 GET 可以只结算自身三元组；若还有其他待定，继续冻结且不运行 bearer bridge，因此两个发起页的明确失败不会互相遗失或因等彼此结算而误发令牌。恢复可用状态仍须完整在线 GET／bridge。401和失效来源进入匿名。

超时、网络故障或5xx结果未知时保留本地屏障和原关联值，后续旧 GET 或复制的终止提示仍不能自动解冻；等待权威新会话／版本，或用户通过完整页面重新登录，不额外制造后端授权状态。所有远端 mutation 在恢复后、任何 action 前统一拒绝仍有屏障的原来源，覆盖已排队续期、直接成员选择、再次切换和登出；本页纯内存 forget 不宣告未决远端操作完成。续期旁路已先用移除该统一 guard 的两项 RED 验证，再恢复 guard 后通过。

最终宿主修复门禁：67项 BFF/API/auth-store 定向回归由完整普通测试覆盖，完整普通测试476/476 PASS；typecheck、当前模块0 warning严格 lint、生产构建、407交付文件及固定SDK检查全部 PASS。完整通知浏览器12/12 PASS；新 Chromium 专项覆盖外会话及同版本缺／错关联值、并行另一操作结算不解除原操作、取消 live=true/false、新会话同步、返回 URL 移除、坏格式／超长／重复 marker 拒绝，以及伪造合法 marker 时服务端401仍无 token、微应用或员工连接。主动并行POST、逐revision、32/128边界、UNKNOWN所有mutation和已排队renew由普通回归覆盖。独立只读复核确认最终生产 SHA256 `09a43307cb0b7e0630b458e9684346c15e836ed6d3502a81ae4e362f05681a14`，无剩余P1/P2。主代理已核实宿主与后端修复输入，并将最终源码清单及阶段结论合并至组件 JSON。

## 私有通知收件人恢复 P2 修复

公开删除收件箱后，原实现只查询可删除的投递行，可能漏掉已经失效的原收件成员。实际 PG/LOGIN/HTTP 回归先取得 RED（旧 `result` 错误返回200），随后将服务端生成的完整原成员 tuple 随 publication 写入不可变 `request_body.authority.recipientProofs`；恢复与提交前按原 tenant/application 和全部原成员事实复验。原请求/source 哈希保持原语义，客户端不能注入 authority；缺失、畸形、重复或数量不符返回503，不能变成可触发补发的404。

修后同一实际矩阵1/1 PASS（4.88秒）：USER全部删除、ORGANIZATION部分删除均经过公开 DELETE。原成员仍有效时两条私有路径只恢复原结果、不重建收件箱；失效或换代后 `result` 与 `publish` 均403。普通通知3/3、严格all-target Clippy、fmt和独立审查通过，测试库/角色/会话均0残留。在线account-proof/UserInfo仍是明确的受控transport fixture，本项不冒充Phase25全服务HTTPS。

## 实现

宿主的 BFF 协调器在发起业务身份切换前同步冻结本标签页，取消旧 lifetime，向同源其他标签页广播 `sessionId`、`revision`、`invalid`、`phase` 和每次独立的 `switchId`。消息不携带 token、CSRF 或成员证明。收到切换提示的其他标签页立即取消请求和通知连接；与原版本相同的、可能抢在服务端处理前返回的 GET 不能使旧来源恢复。匹配完整原操作的结束提示或新服务端版本后仍须重新读取服务端会话并取得来源证明。迟到的旧账号切换提示、旧版本／旧关联值不能冻结或解冻新来源；账号替换的结束提示会触发在线核实。

现有宿主 `contextEpoch`、完整身份 cache namespace、qiankun 串行卸载 barrier 和 SDK3 signal 检查共同销毁旧表单、缓存、微应用能力及通知回调。切换提交后先卸载旧实例，再取得新能力／上下文挂载；主题和同一授权内的展示更新仍按原合同处理。取消时也重新读取 Cookie 会话并证明原来源有效；原会话已失效则不恢复凭据、微应用或员工通知。

平台新增 [私有通知合同](../phase-18-private-notification-contract.md)：

- `POST /internal/platform/apps/{appCode}/notifications/publish`。
- `POST /internal/platform/apps/{appCode}/notifications/result`，只查结果，不创建或补发。
- 独立 `APP_RESOURCE` Basic 服务认证，严格有界 Form 携带原 token、`notification/publish` 和完整原通知 JSON。公开 SDK 发布路径继续只接受内部 app_session。
- 外部 opaque USER 通过 canonical 私有 proof 和原 COMMITTED selection 获取私有成员绑定；不解析公开 sub、不签发虚构 JWT。MACHINE 和 CUSTOMER 明确拒绝。
- 精确 publish scope、当前 notification:publish permission、直接成员独立 dataGrant 和员工收件人均必须通过。工单 scope 为显式审核映射 `work-orders:notification:publish`，不是全局开放第三方发布。
- 私有幂等域绑定原 token 摘要、精确来源和原 operation；改变内容冲突，替换来源不能恢复旧结果。原结果恢复同样复验原来源、发布权限、数据范围和原收件人。
- 两次在线来源检查在打开发布事务前完成；事务等待后及提交前再次检查本地 selection、成员／账号代际、服务版本、应用／资源策略、permission、dataGrant 和收件人。wrapper 最长三秒，任何不确定结果保持错误，不据此用新来源补发。

SDK 仍为 3.0.0，MicroProps 为 2.0，授权上下文与通知公共 DTO 为 1.0，qiankun 固定 2.10.16。没有变更生成产物或向浏览器分发私有服务凭据。SDK、通知服务和宿主文档已同步内部发布与外部服务适配的消费者边界。

## 本阶段验证

| 组件 | 命令／层级 | 新执行结果 |
| --- | --- | --- |
| 宿主普通门禁 | `npm test` | 修复后476/476 PASS，含新增跨标签页冻结、切换关联、并行／UNKNOWN／queued renew、取消有效／无效及旧账号／版本提示回归 |
| 宿主联合浏览器 | `node tests/notification-frontend-browser.mjs` | 12/12 PASS；其中新 Phase18 专项有双标签页完整链、取消原来源有效、取消原来源失效三条流程 |
| 宿主构建与交付 | `npm run build`、`npm run sdk:check` | TypeScript/Vite PASS；407 个交付文件检查 PASS，固定 SDK 副本 PASS |
| 宿主 lint | `npm run lint` | PASS；既有 45 个 warning，0 error；当前改动的 BFF 协调器没有新 lint warning |
| SDK | `npm run check`、`npm test` | 20 个生成产物一致，30/30 PASS |
| 平台新私有通知 HTTP／数据库 | `phase18_notification_http... --ignored` | 最终 1/1 PASS，4.88 秒，真实 LOGIN／新库；含原source/权限矩阵、公开DELETE后原收件人全删／部分删恢复与失效403回归 |
| 平台通知普通回归 | `cargo test --locked --bin saas-platform-backend notifications::tests` | 3/3 PASS，含不可变收件事实严格解析与禁止客户端注入 |
| 平台静态门禁 | `cargo clippy --locked --all-targets -- -D warnings`、fmt | 最新 strict Clippy PASS；自身代码 fmt PASS；并发 Phase20 一次临时 fmt 差异已由所属代理修复 |
| 平台文档 | `RUSTDOCFLAGS=-Dwarnings cargo doc --locked --no-deps` | PASS；本轮两处rustdoc文本修正已验证 |
| 工单普通门禁 | ordinary all-targets、doctest、strict Clippy、fmt | 83普通（含77 library）、1 doctest PASS；Clippy/fmt PASS |
| 通知服务普通门禁 | `cargo test --locked --lib`、strict Clippy、fmt | 36/36 PASS，Clippy/fmt PASS |
| 通知服务真实进程／数据库／Socket.IO | `cargo test --locked --test identity_isolation -- --ignored --nocapture --test-threads=1` | Linux 3/3 PASS，8.67 秒；每组数据库／pool 均清理成功 |
| 工单私有通知 consumer | `cargo test --locked --test reminder_contracts -- --ignored --nocapture --test-threads=1` | 最终 Linux 15/15 PASS，7.23 秒；原13组保留、补2组，strict Clippy／fmt PASS，全部隔离数据库／pool 清理成功；生产实现由主代理持有，本报告执行者仅适配该测试文件 |

浏览器版本：Chromium `156.0.8078.4`，Playwright `1.64.1`。浏览器真实运行当前完整 React 宿主、qiankun 2.10.16、固定 vendor SDK3 和 Socket.IO；显式模拟 BFF／身份 HTTP，不连接 PostgreSQL，不声称实际身份服务 HTTPS 互通。新专项证明：

1. 两标签页 A 均挂载微应用和通知；开始统一身份切换后，另一页同步冻结、A 微应用销毁、A 通知连接全部断开；同旧版本 GET 不能解冻。
2. 在刻意忽略 AbortSignal 的 transport 中挂起 A 响应，提交 B 后旧结果被拒绝；保留的 A SDK 写回调在发送前返回 AbortError，无旧写请求；B 用新能力挂载并写入。
3. B 的两个通知连接均绑定新来源；向 B 连接发送 A 归属事件不会展示；B 到不同账号 C 的切换销毁 B 连接及实例；CUSTOMER 投影不能保留员工 token、微应用或订阅。
4. 取消统一身份切换后，只有仍有效的 A 来源可重新取得 token、挂载和订阅；来源已撤销则不恢复。

平台私有 HTTP 组件使用真实 PostgreSQL 规范基线、实际 LOGIN 角色及 HTTP Router，远端 canonical proof 是明确有界的回环协议 responder。验证 publish/result 幂等、原 source／原请求绑定、scope、服务认证、独立 dataGrant、recipient 缩减、USER／MACHINE／CUSTOMER 和权威故障；它不等于实际身份服务与平台 HTTPS 互通。

通知服务 ignored 测试启动当前 Rust 通知二进制、真实数据库投递行和 Socket.IO polling，平台私有证明使用独立协议 authority fixture。旧 Phase4 测试名称仅是可执行测试标识，本报告记录的是当前源码在 Phase18 的新执行结果。

工单实际数据库／HTTP15组保留原 scope、组织上限、严格输入、SQL、intent／commit、连接释放、行锁释放、进程重启、接收人／工单改变、并发和 response-loss 回归。controlled platform fixture 已改为独立 Basic 服务认证、原 token Form、只读 result-first 与严格 `resource-notification/1` wrapper；结果恢复减少 publish POST，不增加重复发布。新增正向 external USER、缺 publish scope 不造 intent、MACHINE403、错误404 code／未知字段不允许 POST、错误／过期 wrapper 不补发、同原结果恢复、换 token／换 canonical client 来源沿旧 key 返回409 `REMINDER_SOURCE_CHANGED`。普通缺 scope 拒绝为401 `INVALID_APP_TOKEN`，不误报17阶段旧的外部 USER 总拒绝仍存在。

## 边界与清理

本阶段实际涉及平台宿主／后端、SDK、通知服务及工单五个工作单元，位于四个独立仓库。没有创建或复用 Docker 容器（0）；静态镜像策略普通测试使用 fakeDocker，没有实际创建镜像或导出容器。统一全服务 TLS／实际消费者矩阵由 Phase25 在固定当前版本环境重新执行。

平台首轮 HTTP fixture 已报告数据库、角色和 pool 全部清理成功。Windows 通知服务三组集成测试遇到既知回环 `/health/ready` 超时，每组均通过 Drop／finally 删除自有数据库、关闭 pool 并结束通知子进程；没有用它们作为通过证据，也未降低生产超时或认证要求。Linux 首次链接曾因 `/tmp` 7.7 GiB tmpfs 满而失败，未进入测试、未创建数据库。主代理已将共享 task cache 移到 disk-backed `/var/tmp`，原路径保留符号链接；本代理未另建独立 notification cache。该迁移释放 tmpfs 容量，不表示 Docker 或 Windows WSL 虚拟磁盘已缩小。

共享 PostgreSQL `55494`、主代理协调的构建缓存和已有 `.local-run` 通知进程不属于本阶段临时 fixture 的清理范围。所有本阶段新启动的 Vite、Chromium、Socket.IO、协议 responder 和临时通知进程由退出处理关闭。独立查询确认本阶段三个 fixture 前缀的数据库为0、平台私有 HTTP 角色前缀为0、对应数据库会话为0；没有本阶段测试或通知子进程残留。唯一既有 `.local-run` 通知服务未被改动。

本代理此次才创建的 Windows `saas-notification/target` 已用 Cargo 的定向 clean 删除，3539个文件、约2.2GiB（核查文件合计2,365,609,426字节）；删除前确认绝对路径位于该仓库、不是 reparse point、含 Rust target 元数据。没有清理平台现有 target、共享 Linux cache 或正式报告。Linux 二进制和依赖仍归主代理保留供后续阶段统一收尾。本阶段没有执行 Docker prune、WSL shutdown、磁盘压缩或任何既有数据删除。

## 自检

| 台账验收项 | 结果 | 当前证据 |
| --- | --- | --- |
| 切换后无旧组织数据或写入覆盖，微应用和通知使用新来源 | PASS | 双标签页真实宿主／qiankun／SDK3／Socket.IO 专项，旧写未发送，B 新来源挂载与双订阅 |
| 延迟响应和旧连接被丢弃或断开，无跨账号／员工客户泄漏 | PASS | transport 忽略取消的迟到响应、旧 socket 全断、B→C 与 CUSTOMER 反向投影拒绝 |
| 取消切换只恢复仍有效的原会话 | PASS | 浏览器 live=true／false 两流程及普通回归 |
| SDK 契约版本、消费者及必要发布说明一致 | PASS | SDK30/30、20 产物、宿主407交付检查、消费者边界文档；工单新私有 adapter 15/15 实际数据库／HTTP |

主代理已完成两类P2的修复证据汇总、独立复核及最终47文件摘要核对，Phase18验收完成。总台账同步为COMPLETED。下一阶段仍依主台账和已批准并行安排推进；本报告不授权未来清库、公开发布或扩大正式数据操作范围。
