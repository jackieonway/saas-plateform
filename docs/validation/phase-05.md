# Phase 5 执行与验证证据

日期：2026-10-15（Asia/Shanghai）。阶段：完善 ZIP 兼容静态包的原子安装与恢复对账。最终状态：COMPLETED。三项台账验收均PASS；源码、实际测试计数、失败历史及清理结果已核对归档。

## 1. 执行边界与起点

用户明确要求按台账执行 Phase 5，前置 Phase 3、4 已 COMPLETED。Git HEAD 保持 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，保留全部已有未提交工作，不创建提交、外部镜像推送或生产部署。前阶段最终白名单源码摘要为 `00b536afd8b8baaa2406d074af952a7d560758c2ba8bbeabf366a1bcaa6fca27`；本阶段各运行另外记录实际工作树摘要，HEAD 不代替未提交输入。

范围仅为同机共享目录中的 ZIP/tar.gz：受限解包、完整私有暂存、无覆盖原子安装、不可变摘要、跨环境登记与对账、暂存维护、真实平台/数据库/Nginx 发布链。不实现跨机分发、不构建或启动微应用镜像、不迁移业务服务或开启未来阶段。测试使用自有临时 PostgreSQL、网络、版本卷、证书和随机凭据，未读取私有 `.env` 或借用生产数据库。

工具沿用 Rust/Cargo 1.97.0、Node 24.18.0/npm 11.16.0、WSL Ubuntu Docker 29.1.3/Compose 2.39.4。Windows 安装测试位于实际 NTFS；Linux 测试在全新无宿主挂载容器中运行，真实 overlayfs/tmpfs 跨设备行为分别验收。正常 TLS 校验始终开启，测试 CA 仅用于本次隔离客户端与平台配置。

## 2. 实现

### 私有安装与产物身份

`backend/src/release_archives.rs` 保留原压缩 20 MiB、单文件 4 MiB、文件总量 16 MiB、49 文件、128 条目以及路径/链接/特殊文件限制。路径还拒绝 Windows 尾点/尾空格、完整 COM/LPT 保留名、文件/目录前缀冲突及保留 metadata 的大小写别名。tar.gz 在库解析之前增加 **17 MiB 全解压流预算**，涵盖 tar 内部消耗的 GNU/PAX 元数据、头和填充，防止文件检查之前的内部读取绕过限制；不提高 16 MiB 实际文件配额。

安装器在 `MICRO_APPS_DIR/.staging/<UUID-v4>/payload` 完整写入并同步文件、隐藏清单和目录，再一次发布到 `<app>/versions/<version>`。暂存父目录 0700；正式目录 0755、文件 0644。Linux 使用 `renameat2(RENAME_NOREPLACE)`，Windows 使用无 REPLACE/COPY 标志的 `MoveFileExW`，不回退为普通 rename 或跨设备复制；已有空目录、文件、symlink/junction 均不能被覆盖。Windows 碰撞路径和成功安装后核对实际版本拼写，大小写别名竞争不能双双成功。

`.artifact-manifest.json` 的规范摘要绑定 schema、entryType、entry 和按路径排序的 `{path,size,sha256}` 文件清单，不含摘要自身、压缩封装、时间、版本或环境。不同 ZIP 压缩方式/顺序、tar.gz 及同一外包装前缀中的相同文件产生相同摘要。资源定义也稳定排序。检查已安装产物时会重新核对实际文件集合和字节，不能只相信 manifest 声称的摘要；旧目录只能在完整字节吻合后受控采用。

### 数据库、重试与恢复

仅新增 `0002_release_artifacts.sql`，不改 `0001_init.sql`。原迁移 SHA-256 为 `723e6b957a2ba1dc65df0355aa0d22e34d50a020dc97cd6d193d53c69d47eed7`。新增应用/版本共享 artifact 定义、可空 release 摘要绑定、外键及不可变约束；原 URL 登记保留未绑定状态。

ZIP 安装成功后，登记使用短应用事务锁，核对所有环境的完整定义与规范摘要。同环境同产物返回稳定 release ID，不重复登记审计；跨环境共享同一产物，但各有 release ID 和部署指针；异摘要或已有定义不一致为 409。普通 URL 登记不能旁路已绑定 ZIP 的版本命名空间，提示使用同一包提升环境。文件/网络操作不占用数据库登记锁。

COMMIT 结果或后续回读未知时，保留已安装文件并查询主库核对；查不到、数据库不可用或响应丢失都不授权删除公开目录。`GET /api/applications/{id}/releases/storage?version=...` 受平台管理权限保护，报告 `MISSING / INSTALLED_UNREGISTERED / REGISTERED / INCONSISTENT` 和全部环境引用。绑定 ZIP 的预检、激活及回滚会先核对实际本地摘要，缺失或损坏保存 `FAILED / ARTIFACT` 诊断，即使 HTTP 字节仍有合法 MIME/200 也不能启用。

### 暂存维护与静态边界

Rust 二进制支持 `--micro-staging --root <绝对目录> [--apply]`，在环境加载和数据库初始化之前离线分派；默认 SCAN，仅显式 apply 清理。写入持有永久维护文件的共享内核锁；维护只在成功取得非阻塞独占锁后处理 UUID 和 ownership record 均匹配、无链接和未知内容的私有 stage。不按 mtime/PID 强行解锁、不 unlink 锁文件、不删除任何正式版本，不明目录保留并报告。后台写入任务自己拥有锁和暂存生命周期，HTTP future 取消不会提前释放它们。

Nginx 在通用版本匹配之前拒绝点 metadata 和任意大小写 `release.json` 路径段；与 native renderer 共享策略。操作、升级停写、未知提交重试、旧 binary/新迁移回退边界见 [zip-recovery.md](../../deploy/zip-recovery.md)。旧写入进程不遵守新锁，升级必须先停止全部旧 writer 并排空在途上传，不能同卷混跑。

## 3. 实际结果与证据

不同快照、重复子集和不同操作系统的检查不能相加为唯一总数。最终 Linux、Windows 专项与 HTTPS 结果分别保留，机器可读汇总为 [phase-05-runtime.json](phase-05-runtime.json)，最终白名单输入为 [phase-05-inputs.json](phase-05-inputs.json)。汇总复核实际日志计数、必需组完整性、源码摘要及清理记录，不只读取顶层 PASS。

| 检查 | 当前实际结果 | 原始证据 |
| --- | --- | --- |
| Windows/NTFS 最终安装回归 | PASS，17/17，0 fail/ignored；含实际进程终止、并发、junction、大小写竞争、GNU/PAX 预算及暂存清理；r4绑定当前lock及源码摘要，运行前后输入相同 | `target/phase-05/archive-windows-r4/{result.json,windows-archives.log}` |
| Windows 离线维护入口 | PASS，3/3，0 ignored | `target/phase-05/windows-maintenance-tests.log` |
| Windows Rust all-targets check | PASS；仅既有 Config::from_env dead_code warning | `target/phase-05/windows-check-final.log` |
| Rust doc | PASS；既有两处 rustdoc warning 保留 | `target/phase-05/windows-doc.log` |
| 全仓严格 Clippy / fmt | FAIL，Clippy 生产15处、测试编译目标38处既有诊断，fmt 仍有全仓格式差异；新 archive/maintenance 模块无该类诊断，未将失败当成通过 | `target/phase-05/windows-{clippy,fmt}-final.log` |
| Windows CI 工具回归 | PASS，48/48，0 fail/skip | `target/phase-05/ci-tool-tests-final.log` |
| Windows 打包器补充 | 28 PASS、1 symlink 权限 skip；不是 Linux 必需零 skip 证明 | `target/phase-05/package-tests.log` |
| Nginx/native 配置策略 | PASS，13/13、0 skip，含真实 Compose parser；真实 HTTP 编码验证由 E2E 另计 | `target/phase-05-workflow/`及本会话输出 |
| Workflow 政策/actionlint | PASS；政策3/3、20种错误修改拒绝，actionlint1.7.7 exit0 | `target/phase-05-workflow/{policy-tests.log,actionlint.log}` |
| Linux r1 | archive15、maintenance3、打包器29、CI工具47通过；PG **26 PASS/1 FAIL/0 SKIP**，新ZIP6/6通过；外层正确FAIL且清理无错 | `target/phase-05/linux-db-r1/` |
| 最终 Linux 安装/维护/CI工具 | PASS，archive16、maintenance3、CI48，全部0 fail/skip；同一源码快照 | `target/phase-05/linux-db-r5/reports/extras/` |
| 最终正式 PostgreSQL gate | PASS，5个模块27/27、0 fail/skip，含ZIP6及实际NULL父菜单重启断言 | `target/phase-05/linux-db-r5/reports/postgres/{summary.json,postgres/postgres-result.json}` |
| 最终 Linux 完整契约 | PASS，99/99、0 fail/skip；锁定安装→两示例build→仓库sync→完整测试，源码前后不变 | `target/phase-05/linux-db-r5/reports/contract-supplement/` |
| 真实 production Rust/PG/Nginx HTTPS | PASS，9/9、0 fail/skip/NOT_RUN，exit0；完整地区初始化与真实登录成功 | `target/phase-05/zip-e2e-r4/{result.json,logs/}` |
| 独立资源清理核对 | PASS，两种owner标签的容器/网络/卷均为空；私有证书/env删除，WSL lease exit0，镜像与日志保留 | `target/phase-05/resource-audit.json`，HTTPS最终报告 |

CI 已接入全部 archive 嵌套单元、维护3项、打包器29项，以及明确的5个 PG 模块27项（原21 + ZIP登记6）。当前 fast 合约清单99项已在最终 Linux 同快照完整实测。Delivery 新增9组真实 ZIP 回归，复用已校验候选三 image ID，源码/架构/镜像身份不符时拒绝。缺组、失败、skip、NOT_RUN 或清理失败均使该层失败；只上传 result.json 和脱敏 logs，不上传 TLS 私钥/运行环境。Phase4整轮fast、浏览器与三镜像候选结果继续作为独立历史，未宣称本阶段重新完整执行这些层或远端workflow。

### 最终输入及九组 HTTPS

完整 CI 白名单403项输入摘要为 `99bdd76a4762aff60e4097b36d3b14140c401b861fe938c9f02b68ff95eef78c`。最终 PG、Linux安装/维护/工具、完整99项契约及HTTPS driver使用相同输入；补验和原测试均检查运行前后源码不变。最终原生构建范围294项输入摘要为 `cdff2bf186bc1af371ec9fd75a4d51c058768e05128c408b7d3d1e1f33864d1d`，与当前各记录文件逐项相同；完整白名单数量与各摘要、日志SHA见JSON。文档与台账不在该构建白名单中，另由Git差异检查。

HTTPS于2026-10-15 08:45:18–08:55:05 UTC执行。新平台镜像为 `sha256:4e10f2b7915cc8de5d365aa68be6c504e4460eeeee4d79e297808e648ce5a71a`，宿主和入口复用Phase4固定image ID。宿主container ID在v1、v2、回滚期间也相同。实际版本卷`stat -f %T`输出为`ext2/ext3`，保留原值；独立Linux安装单元使用overlayfs，跨设备测试指向`/dev/shm` tmpfs，不将两类环境泛称裸ext4。

| 必需组 | 实际证明 |
| --- | --- |
| production-boundaries | production bootstrap→登录ticket→平台根上下文；正常CA/SNI校验，只有edge发布loopback端口；平台版本卷RW、UID101静态服务RO；无效运行配置下离线SCAN也不启动平台 |
| response-recovery | 实际收到HTTP响应头后丢弃body，通过受保护列表/storage确认REGISTERED，重试保持同ID/摘要 |
| v1-preflight-activate | Nginx实际HTML/module/CSS，显式预检与激活成功 |
| parallel-idempotent | 三个同包并发、stored/deflated ZIP字节不同而规范摘要相同，稳定release ID |
| cross-environment-immutable | 跨环境异字节409；普通URL在qa登记已绑定版本409且无row；同包staging提升成功、production指针不变 |
| activate-rollback | v2激活及回滚v1成功，v1/v2旧URL均200且SHA一致，宿主镜像和容器不变 |
| parallel-conflict | 异内容同版本竞争分别200/409；连续公开观测只出现404或完整胜者的HTML/JS/CSS/binary，无混合字节 |
| interrupted-private-staging | HTTP body中断后无公开版本/数据库行，旧v1/v2仍完整；真正文件写进程中断由Rust专项另证 |
| private-metadata | 实际manifest/maintenance lock及release.json等22次GET/HEAD/编码请求404/no-store，无害查询参数仍可用 |

关键实际命令在上述原始报告中保留。专项Rust使用`cargo test --locked --manifest-path backend/Cargo.toml --bin saas-platform-backend release_archives:: -- --test-threads=1`及`release_archive_maintenance::`；正式PG驱动为`node scripts/ci/run.mjs --gate postgres ...`，明确`--ignored`运行选定27项；契约通过manifest完整列表交给`node --test --test-reporter=tap`。HTTPS独立模式用`node tests/zip-release-e2e.mjs --out-dir <新绝对目录> --frontend-image <固定ID> --edge-image <固定ID>`，仅构建当前平台。CI的`--release-report`模式另由5项driver政策测试与48项工具回归验证；本轮实测使用独立模式，不冒称远端候选发行已执行。

## 4. 失败与修复

- Linux r1 唯一 DB 失败是旧设置测试硬编码只有迁移 `vec![1]`。已改为精确比较当前嵌入迁移版本清单、首项仍为1，并核对重启前后旧迁移 checksum、原 branding 和种子行为；不删除历史 checksum 或跳过测试。最终完整27项重验，不改 r1 原始失败报告。
- Linux r3 的新增 URL 命名空间负例使用 production 环境和本地 HTTP，先被既有 HTTPS 要求以 400 拒绝，未到预期的 ZIP 冲突 409。仅将负例环境改为 staging，再验证普通 URL 登记没有遗留行及相同包可提升；r3 的 26 PASS/1 FAIL 原样保留。r4 完整 PG27 已通过；最终 r5 还覆盖后述真实首启修复。
- 第一轮 Windows/安装与早期 Linux15通过后，独立审查发现 Windows 大小写竞争以及 tar GNU/PAX 内部元数据预算问题。两项均增加回归后修复；旧15项不是最终17/16项的同源码验收。
- HTTPS r1 的真实生产平台已连接隔离 PG、执行迁移及 bootstrap；地区导入期间 Docker journal 记录 systemd 停止 docker.service，PG 和其他容器同刻正常停止。保留日志，不把“WSL idle”写成唯一未经证明的因果。Windows driver 新增拥有独立 stdin 管道的前台 WSL lease，EOF/进程退出可结束，不改 WSL/daemon 全局设置。
- HTTPS r2 的地区全量初始化实测耗尽 PG 768 MiB tmpfs，日志为 FileFallocate / No space left on device。最终仅提高本次自有临时 PG 配额，不改生产配置或删减地区数据；两次环境失败均不计通过。
- HTTPS r3 成功 COPY 全部 665,923 条地区记录，随后 `region_seed::initialize` 查询菜单的 nullable `parent_id` 时按非空 UUID 解码，真实生产首启因此失败，9 组 ZIP 尚未运行。最小修复为 `query_scalar::<_, Option<Uuid>>(...).fetch_optional(...).await?.flatten()`。现有 PG 品牌/重启测试增加真实 SQL NULL 父节点、连续两次初始化菜单 ID 稳定且不重复的断言；该窄回归预置 imported 标记以免反复导入全量目录，完整 COPY 仍由新 HTTPS 轮次验证。此修复是完成本阶段真实平台启动所必需，不扩展地区功能。
- Linux r5 的扩展包装器首次契约为97 PASS/2 FAIL，原因是干净快照没有预先生成`frontend/public/sub-apps/{vue,jquery}/app.js`，实际为ENOENT。补齐锁定依赖安装、示例build和仓库sync后，在同一源码容器完整99/99通过；补验于08:52:06 UTC完成，正式PG于08:52:44完成，全部证据随后归档并清理。**原始extras及outer仍为FAIL/exit1**，没有改成统一绿灯；最终正式PG27/27、专项16/3/48和完整99补验分别读取真实结果。汇总JSON保留诊断FAIL、前置准备和补验关联，不能只截取outer或只看97项就宣称阶段通过。
- 严格 Clippy 的既有全仓问题继续单列。首次检查还发现一处新测试字节数组写法，已改为等价 byte string，并在最终 Windows17项重新验证；不批量重构其他业务源码或关闭全局规则。

## 5. Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 写入中断无公开半成品，并发无混合字节 | PASS | Windows17/Linux16专项含进程中断、无覆盖/并发；HTTPS竞争连续观测只有缺失或完整产物，HTTP中断无公开版本 |
| 同摘要幂等、异摘要冲突，DB提交后丢响应可对账 | PASS | 最终真实PG ZIP6覆盖实际提交后注入ACK/回读丢失；HTTPS实际body丢失→查询/稳定重试，跨环境/普通URL409与同包提升均通过 |
| 真实Nginx上传→预检→激活→回滚，宿主不重建且旧资源可达 | PASS | 最终production Rust/PG/正常TLS/Nginx九组通过，宿主image/container ID相同，v1/v2旧资源SHA一致 |

未宣称Windows突然断电后的持久性或所有文件系统都支持安装；不支持原语、EXDEV或无法安全辨认时拒绝。当前仍有全仓fmt/Clippy和rustdoc warning，Windows打包symlink补充项受权限跳过，其能力已由Linux29项包含在99项中零skip验证。没有使用生产数据、实际registry或外部部署，也没有执行未来阶段。

## 6. Suggested Task Ledger Update

### Phase Status Update

Phase 5: COMPLETED。汇总、详情、自检及证据同步；Current Execution Pointer移至Phase 6 NOT_STARTED，后续实施需要新的执行指令。

### Output Summary / Completed Items

- 私有stage、Linux/Windows禁止覆盖原子安装、规范artifact digest、并发幂等/冲突与保守旧目录采用。
- 只新增0002迁移、跨环境不可变登记、未知提交保留与四态对账、实际文件损坏阻止预检/激活/回滚。
- 离线SCAN/APPLY与共享/独占内核锁、Nginx metadata隔离、CI必需计数/源码/清理门禁和升级恢复说明。
- Windows17、Linux16/维护3/PG27/契约99/CI48与真实HTTPS9通过；原始失败与同快照补验分别保存。

### Remaining Items

本阶段无剩余必需项。既有全仓检查债务、未提供生产环境及未执行远端发行继续明示，不扩大本次范围。

### New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 应用+版本绑定一种规范产物，普通URL不能旁路ZIP绑定 | 跨环境不得借不同来源改变同版本内容 | 相同包提升；异产物409；旧未绑定URL仍按既有流程 |
| 未知DB结果保留正式目录，维护仅处理可确认私有stage | 不能从错误响应推导提交未成功 | 查询storage/全部环境后重试；不自动删除公开版本 |
| 新旧writer升级先停写/排空；锁文件长期保留 | 旧进程不遵守新锁协议 | 禁止新旧writer同卷混跑，旧0001-only二进制不能直接降级运行 |
| 修复真实首启NULL父菜单解码 | 完整production启动阻断本阶段必需HTTPS验收 | 最小nullable修复及现有PG重启断言，不扩地区功能 |

### New Open Questions

无新增阻断问题；真实生产卷、域名、registry及正式运维政策仍沿用台账Q02–Q04等未收敛项。

### Next Recommended Phase

Phase 6: 版本化 SDK 与公开授权契约。未执行。
