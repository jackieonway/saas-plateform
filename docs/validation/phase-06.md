# Phase 6 执行与验证证据

日期：2026-10-15（Asia/Shanghai）。执行阶段：Phase 6，版本化 SDK 与公开授权契约。状态：**COMPLETED**。四项验收均PASS；正式fast23步、PostgreSQL27及三产品各146组完整通过、零skip，自有资源与lease已核验清理。所有初轮失败原样保留。

可复核的最终汇总为 [phase-06-runtime.json](phase-06-runtime.json)，逐文件源码摘要为 [phase-06-inputs.json](phase-06-inputs.json)。本阶段建立首个登记SDK及公开生成契约，接入宿主和三个前端消费者，并修复独立审查复现的旧401跨epoch竞态。

## 1. 边界与起点

用户明确授权 Phase 6；依赖 Phase 1、4 均已完成。Git HEAD 保持 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，保留所有 Phase 1–5 及用户已有未提交工作。Phase 5 完整输入摘要为 `99bdd76a4762aff60e4097b36d3b14140c401b861fe938c9f02b68ff95eef78c`；它是历史起点，不充作本阶段证据。

起点共享 SDK bridge 与工单 vendor 原本字节一致，SHA-256 都是 `fb041923bbcff457b4d8b559f42637c5ee74ebc9c34824a901ba87d60e74b42a`。本阶段将真实旧字节保留在 `examples/micro-app-sdk/compat/legacy-iframe-2.0.js`，没有虚构之前存在已发布 SDK 版本，也没有将原本一致描述为修复漂移。

范围只有 SDK 版本、公开 wire/schema/TS/Rust、classic/ESM、必要消费者、构建/漂移与兼容验证。没有改微前端内核、实现 Phase 7 永久挂起隔离或 Phase 8 更新语义、共享数据库模型、迁移工单/通知/OAuth后台、建立根 workspace、外部发布或生产部署。工单参考 API 的 CORS 只增加本次 SDK 所需的 requestId 头，不扩展 Node 后台业务。

工具沿用 Node24.18.0/npm11.16.0、Rust/Cargo1.97.0、Playwright1.64.1、WSL Ubuntu Docker29.1.3。依赖继续使用已有锁文件，无新增 Rust crate 或 npm 运行依赖；esbuild 使用宿主已锁定构建依赖。隔离 PostgreSQL17.6 使用独占私网/tmpfs、随机凭据且无宿主端口；没有读取私有 `.env`。

## 2. 交付与保持的行为

### 单一登记、生成与消费者

`examples/micro-app-sdk/release.json` 首次登记候选 SDK1.0.0，保持 context1.0、MicroProps/iframe2.0、notification1.0。SDK版本和wire版本独立。`contracts/schema.json` 是公开结构源，版本/公共限额由登记解析；同源生成标准JSON Schema、TS公开类型和纯serde/serde_json Rust wire DTO。signal/回调等不可序列化运行时字段使用明确小模板，不伪装成JSON schema字段。

统一入口 `npm run sdk:sync` / `npm run sdk:check` 生成或检查30个目标，包括登记常量、root兼容入口、`releases/1.0.0` classic/ESM/类型/schema/Rust/逐文件摘要，以及平台Rust、工单bridge/ESM、Vue/jQuery classic vendor。CHECK不会覆盖差异；测试在临时独占目录分别破坏vendor、schema、types、版本常量、release ESM和Rust副本，验证拒绝且原差异未被修复。当前release目录存在未登记文件时拒绝，并且不删除它。

宿主窄导出生成的公开Micro/context/lifecycle及通知类型，本地保留管理UI/入口状态类型。runtime、props、iframe bootstrap/READY和通知校验消费登记常量；显式未知SDK或wire拒绝，当前保留的versionless2.0基线继续可挂载。Vue/jQuery实际生命周期创建同版context client，工单实际调用同版vendored HTTP层；版本字段同时在真实挂载DOM可观测，不能仅靠测试额外注入全局冒充消费。

平台 `routes/app_context.rs` 在真实输出边界构造生成 DTO；保留原授权查询、dataGrant内容、授权hash输入、菜单额外字段及日期输出。Rust保留组织上限缺失/null/list三态，缺失回退当前组织，显式null保持平台实际根语义；未知版本和超限拒绝。Rust模块本身不依赖SQLx/Axum或平台业务类型。独立wire消费者测试不代表未来Rust业务认证、SQL隔离或Socket.IO迁移已完成。

### 请求、错误和能力

通用请求强制 `credentials:omit` 并覆盖Bearer为捕获的应用token，默认JSON1MiB、通用上限4MiB；验证声明长度和实际流字节，不回退为无限`json()`/`text()`。fetch、流读取与上层状态处理均复核取消/快照，旧响应不能写回新授权。

`MicroSdkError` 区分UNAUTHORIZED/FORBIDDEN/CONFLICT/NETWORK/PROTOCOL/HTTP，公开安全status/code/UUID requestId与固定消息；`toJSON()`仅输出公开错误DTO白名单，不保存URL、远端body、token或原cause。取消保持AbortError。工单自有401失效、403视图、409草稿、CSV BOM/4MiB与提醒幂等语义保留；参考后台仅在原批准origin下允许并暴露X-Request-Id，未知origin/header/cookie/PUT仍拒绝。

client/iframe/host能力检测涵盖实际需要的有限响应流、取消、安全UUID、MessageChannel、sandbox及下载object URL。缺能力在申请应用token/context之前停止，没有TRUSTED、同源或无界读取降级。SDK不读共享存储、不自行刷新，跨页会话仍属Phase10。nonce、精确parent/origin、独占MessagePort、epoch、到期取消、组织硬上限和下载broker原回归保留。

### 干净构建边界

fast/browser在锁定安装之后、build之前执行全量SDK CHECK；native白名单加入SDK与必要vendor，CI另包含实际工单消费者和最小CORS测试依赖。frontend Docker保持`/app/frontend`与`/app/examples/micro-app-sdk`相对布局，纯Node Alpine执行`--check --scope sdk`25项目标；完整CI/native仍检查所有30目标，Docker不复制整个平台/工单后台或安装Rust。

旧协议文件中的optional真实浏览器用例独立为`tests/micro-iframe-browser.mjs`并使用项目锁定launcher；纯Node协议组没有条件skip，也没有个人浏览器缓存回退。框架接入、支持清单、90天候选窗口和旧版保留细节见 [SDK README](../../examples/micro-app-sdk/README.md)。

## 3. 实际验证记录

| 检查 | 结果与范围 | 原始证据 |
| --- | --- | --- |
| SDK生成/客户端专项 | PASS，6+11=17，零skip；含独立TS声明编译、漂移拒绝、限额/UTF8/取消/安全错误 | `target/phase-06/generation-client-tests-final.log` |
| Windows宿主/协议/通知/沙箱 | PASS，56/56，零skip；typecheck与相关eslint通过，3条既有eslint warning | `target/phase-06/host/final/` |
| 独立真实opaque iframe | PASS，1/1，Chromium156.0.8078.4/Playwright1.64.1，实际CORS/no-cookie/DOM保留/取消握手 | `target/phase-06/host/final/` |
| Rust公开wire消费者 | PASS，9/9，零ignored；生成Rust及新增测试rustfmt通过 | `target/phase-06/rust-contracts/` |
| Windows Rust工程 | check/doc PASS，分别有1条既有dead-code、2条rustdoc warning；严格Clippy与全仓fmt仍FAIL，既有生产15/测试目标38个Clippy诊断单列 | `target/phase-06/windows-{rust-check,clippy,rust-doc,fmt}.log` |
| 正式Linux PostgreSQL | PASS，5套件27/27，零skip；包含实际root/child context与撤权；输入472项bce504… | `target/phase-06/linux-postgres-r1/` |
| 无缓存frontend Docker | PASS，Node-only生成检查；UID101/RO/networknone，无Node、拒绝写入、HTTP入口及97个assets字节/hash/MIME、缺资源404；native输入零漂移 | `target/phase-06/host-image-r1/` |
| CI/部署策略 | PASS，85/85零skip（CI49+部署36） | `target/phase-06/ci/policy-result.json` |
| Linux fast r1 | 原样FAIL：22步PASS、dev/Vite18 PASS/1 FAIL；合同168、Rust3/16/3/9/8通过 | `target/phase-06/linux-fast-r1/` |
| 三产品browser r1 | 原样FAIL：各127 PASS/1 FAIL/0SKIP，micro-host第6组测试locator错误，后续未执行组不算PASS | `target/phase-06/linux-browser-{product}-r1/` |
| Linux fast r2 | 原样FAIL：锁定安装返回成功但遗漏rolldown Linux原生optional包，build失败，后续步骤NOT_RUN；不改锁文件和依赖进行干净重验 | `target/phase-06/linux-fast-r2/` |
| 最终Linux fast r3 | PASS，23/23步；构建、lint/typecheck、契约169、部署36、CI49、dev/Vite19、Rust全目标check及config3/archive16/maintenance3/public9/transport8均通过，零skip | `target/phase-06/linux-fast-r3/` |
| 最终三产品browser r2 | PASS，各146/146、零skip；8套件逐项核对，三种能力模式均由真实浏览器观察 | `target/phase-06/browser-matrix-result.json` |

不同快照、重复子集或不同操作系统不能相加成唯一总数。Windows部分证明当前安装环境；正式Linux来自全新容器，无宿主源码/home/依赖缓存/Docker socket挂载。浏览器使用真实Vite/React/SDK及受控API/Socket.IO fixture；PostgreSQL另走真实平台路由，不能将UI fixture冒充生产鉴权或完整业务迁移。宿主镜像HTTP探测在network-none容器中进行，未宣称外部HTTPS部署、OCI导出或registry发布。

### 实际浏览器矩阵

| Linux产品 | 实际版本 | Playwright | 必测检查组 | 能力观察 |
| --- | --- | --- | --- | --- |
| Chromium | 156.0.8078.4 | 1.64.1 | 146 PASS / 0 FAIL / 0 SKIP | client/iframe/host均PASS |
| Chrome | 155.0.8059.79 | 1.64.1 | 146 PASS / 0 FAIL / 0 SKIP | client/iframe/host均PASS |
| Edge | 155.0.4283.45 | 1.64.1 | 146 PASS / 0 FAIL / 0 SKIP | client/iframe/host均PASS |

每产品的实际套件数为micro-host24、notifications10、platform-settings11、regions12、entities13、route-permissions64、departments11、chat1。438是三个产品的执行总数，不能描述为438个唯一行为。三份正式报告、UA与能力、原始日志摘要、每owner资源空查询和lease退出均由严格collector核对；另有Windows Chromium完整micro-host24/24诊断，不与正式Linux矩阵混算。

## 4. 命令与证据入口

统一生成/校验入口为 `npm run sdk:sync`、`npm run sdk:check`、`npm run test:sdk`；CI只CHECK，不先同步。实际隔离运行由 `runLinux` 调用以下原有门禁，完整参数、工具与逐步命令以各轮summary及最终runtime JSON为准：

```sh
node scripts/ci/run.mjs --gate fast --out-dir /reports/fast --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c
node scripts/ci/run.mjs --gate postgres --out-dir /reports/postgres --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c
node scripts/ci/run.mjs --gate browser --out-dir /reports/browser --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --browser chromium
node scripts/ci/run.mjs --gate browser --out-dir /reports/browser --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --browser chrome
node scripts/ci/run.mjs --gate browser --out-dir /reports/browser --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c --browser msedge
```

fast/PG工具镜像为 `sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7`；浏览器镜像为 `sha256:66ac2ee1834bf8ccec2cfce67f5bd9f72a3382488ae170134fafc4cb493f3048`。Windows额外执行了锁定Rust全目标/全feature check、严格Clippy、doc及全仓fmt，结果按前表分别分类。

本轮外部driver位于 `target/phase-06/`：fast r3、PG r1、browser r2与宿主镜像driver；最终runtime保存其SHA。driver管理自有WSL stdin lease，在正式命令结束后核对前后输入，并移除仅属于本次容器的reports/work缓存以避免复制npm cache；不改变正式门禁命令、步骤或通过条件。浏览器附加UA/能力观察在正式146组之后执行。重跑应使用全新输出目录，不能覆盖本次记录；项目受管的通用流程见 [CI README](../../scripts/ci/README.md)。

## 5. 失败、修正与快照关联

- 早期SDK生成测试r1在canonical正在收尾而产物尚未同步时准确报告drift，后统一生成17项通过；没有在CHECK中写回掩盖差异。
- Windows独立iframe r1真实失败来自测试CORS fixture没有允许新X-Request-Id头；仅补fixture后r2和最终1/1通过。业务参考CORS另外实际loopback验证，不以fixture代替真实origin限制。
- Browser早期r1因尚未生成protocol模块导致Vite导入失败，随后SDK3项诊断通过。正式三产品r1的失败是测试在`.micro-mount-zone`错误位置即时读取SDK属性；真实生命周期写在内层容器，且jQuery路由守卫可能正在替换挂载。最终断言针对实际版本属性并等待授权挂载就绪，仍要求精确1.0.0，不放宽生产行为。
- Fast r1唯一失败来自历史Vite交付测试硬编码两个script。现在真实产物是framework、SDK、app三个classic脚本；修正断言同时增加版本目录内SDK字节必须等于登记产物，不仅放宽数量。
- Fast r2的原生optional包遗漏没有完整npm debug日志，不能断言具体下载或npm处理原因；锁文件已登记linux/x64/glibc包及integrity。r3保持源码、锁及门禁不变，额外归档脱敏npm日志和安装依赖清单。r2诊断wrapper另将source-verification文件末尾写成字面量反斜杠n，原文件保留为非严格JSON，最终collector不将它冒充可解析JSON或通过证据。
- 独立安全审查确定性复现工单旧401竞态：两层queueMicrotask替换到epoch2后，await helper的旧401可能清掉新上下文。先增加回归使旧代码FAIL，再于catch入口复核snapshot/调用方取消；当前工单26/26通过，独立原探针返回AbortError且epoch2保留。共享SDK/宿主/native字节未因此改变。
- root额外检查保留Clippy15/38及全仓fmt既有失败；生成Rust、新Rust测试格式通过，未关闭全局检查或批量格式化业务代码。

最终快照共472文件，输入摘要为 `9c967cda2b728de9f675f8537ce2c6cb3279bd4026020c77a081b550c9a0e576`；初轮PG的472文件摘要为 `bce504b646f17d7454b6fcb61c0ce04d7b89a2e4d6a3ad6bc37ada58efa1ee0e`。两者差异仅限工单client/回归、browser fixture locator及Vite交付断言，四个路径的前后SHA写入最终JSON。正式fast r3与三browser r2均在最终快照完整通过；PG相关backend/SDK生产输入逐项相同，未将旧报告改名为新结果。宿主56项记录输入、Rust11项记录输入仍逐项当前一致。

无缓存宿主镜像为 `sha256:a857dfbe435228003a67da9ab9c24a694aea2314c4a3467039f684b263a6ac69`，343项native输入摘要为 `d9287100a59038e488e2ccd153312fc48b70ecd7426156b9130bfb72c45c910d`，最终逐项零漂移；当前0001/0002迁移也与Phase5摘要相同。本阶段未新增迁移或改变旧checksum。

最终只读清点 `saas.ci.owner` 与 `saas.phase06.owner` 两种标签的容器/网络/卷，共6次查询均退出0且为空，见 `target/phase-06/final-resource-audit.json`。最终审计自身的WSL lease也正常退出0；没有执行prune、删除镜像、修改全局WSL配置或清理其他资源。各轮自有owner及清理日志继续保留，镜像保留用于复核。

## 6. Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 宿主和至少两个前端消费同一登记版本，CI检测vendor/生成物漂移 | PASS | 宿主+Vue/jQuery/工单实际接入1.0.0；30目标CHECK与负例、完整fast169契约、三产品真实挂载及纯Node镜像构建通过 |
| 401/403/409/网络/协议错误可区分，无凭据泄漏；未知版本拒绝 | PASS | SDK11、生成6、Rust公开wire9、三产品真实SDK组通过；旧401跨epoch回归先失败后修复，最终工单26及正式契约通过 |
| nonce/来源/epoch/取消/组织上限/下载broker及旧静态激活/回滚 | PASS | Node56、真实iframe1、PG27与三产品完整146通过；实际旧bridge字节→1.0.0→旧字节回滚、独立静态版本及受控release API/UI均实测，不宣称生产部署 |
| 浏览器能力检测与SDK支持清单明确，兼容窗口不放宽安全边界 | PASS | 三种真实产品的client/iframe/host能力共9次观察通过，缺能力负例拒绝；支持清单、versionless基线与90天候选窗明确，opaque隔离保持 |

保留Chrome/Edge最近两稳定版为产品目标，本次每产品实际安装版本另外列出；未安装的另一稳定版、Firefox/Safari不宣称完成。候选90天没有推导出实际下线日期。Phase6结束后仅将指针移至Phase7 NOT_STARTED，不自动实施后续阶段。

## Suggested Task Ledger Update

### Phase Status Update

Phase 6: COMPLETED。已依据台账自身“每Phase完成时更新台账”的执行要求同步摘要、详情、证据、决策、变更日志与指针。

### Output Summary

- SDK1.0.0仓库候选，classic/ESM/TS/schema/Rust及30项目标由统一入口生成/检查；原versionless2.0真实基线保留。
- 宿主与三个前端实际消费同版契约，安全HTTP/错误/能力检查和最小CORS接入，旧401竞态回归及修复。
- 完整fast、PG、三真实浏览器、无缓存宿主镜像和只读清理核验，原始失败与不同快照分别归档。

### Completed Items

四项Acceptance Criteria全部PASS；详见Self-Check与逐文件/运行JSON。

### Remaining Items

Phase6内无剩余必需工作。全仓既有fmt/Clippy债务、未测的额外浏览器版本、外部发行及生产环境限制按前述记录；不得转写为已通过或本阶段已实施。

### New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| SDK版本与wire版本独立，首次登记1.0.0候选 | 不虚构历史发行，保持既有安全契约 | context1.0/iframe2.0/notification1.0保持；未知声明拒绝 |
| schema+登记生成公开TS/Rust与版本产物，CI只CHECK | 消费者和副本可持续核对，避免同步掩盖漂移 | 无新增运行依赖或crate；平台仅在输出边界消费DTO |
| 只支持捕获且验收的旧versionless2.0基线，候选90天无下线日 | 旧静态回退须有真实字节和隔离证据 | 不自动兼容任意旧SDK或降低opaque sandbox |

### New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| Q05/Q08剩余的另一稳定版、扩展浏览器和正式兼容下线/保留政策 | 限制后续外部支持承诺，不阻塞已完成的隔离候选验收 | 产品/实施负责人 |

### Next Recommended Phase

Phase 7：隔离可信微应用永久挂起的故障，NOT_STARTED；需新的明确执行请求。本次没有实施Phase7/8或其他后续阶段。
