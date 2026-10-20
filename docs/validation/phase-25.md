# Phase 25 验证记录

执行日期：2026-10-17。终态：**COMPLETED**。按用户指定执行 Phase25；Phase4/9/11/13/14 均已 COMPLETED。本次只标记该任务完成，整个 Current Execution Pointer 区段以写入前字节为准保留，不推进或自动执行下一阶段。

## 交付与行为

- 独立 crate [tools/platform-admin](../../tools/platform-admin/README.md)，二进制 `saas-platform-admin`，edition2024、独立 Cargo.lock、薄 main 和窄公共库出口，无平台实现或数据库依赖。
- 默认离线 plan；bootstrap、grants-apply、register、preflight、activate、rollback 均须显式 `--apply`。inspect/releases/deployments 仅查询。支持 grants-plan，拒绝未知参数、重复 JSON 属性和闭合 schema 以外字段。
- 固定工单应用和七个菜单 key；创建与详情 MENU 使用不重叠路径，四个动作 BUTTON 保持独立。客户端必须是平台实际生成的 ID，CLI 不读取/创建/重置 client secret，不抢占其它应用的关联或菜单。
- 管理请求使用精确 HTTPS origin、rustls、无代理、无 redirect/自动 retry；整条命令共享默认35秒预算（显式10..35000毫秒），JSON 最多1 MiB。可选 `PLATFORM_CA_FILE` 最多64 KiB，仅证书 PEM，正常证书/主机名校验不关闭。
- 响应只投影允许字段；当前 Bearer 若出现在合法成功/错误进度字段也返回 `SENSITIVE_OUTPUT_REJECTED`。未知写结果保留 `outcomeUnknown/reconciliationRequired`，先 inspect 对账。
- Register 只提交现有 `environment/version/entryType/entryUrl/healthUrl/resources{url,kind}`；已有版本拒绝，不提交镜像、digest、target 或 ZIP 安装字段。
- Grant 使用正式功能授权及 Phase14 GET→preview→同一 opaque version PATCH API；实际根上下文、独立 read/write 权限及策略由平台执行。只做批准的正向增量；已有同 scope no-op，不同 scope 固定冲突。角色集合先读、并集 PUT、再读，membership 只校验；部分/未知结果不自动重试或删除回滚。
- 退役5个旧 Node 管理实现/测试，保留两份公开 manifest；现有联合 fixture 改为显式预构建 Rust 子进程，无 JS 管理 fallback。前端打包/测试 Node 用途保持。

## 实测门禁

最终 **linux-r3：17/17 步 PASS，必需0失败/0跳过**；Rust CLI **21/21**（13库+8安全集成）；JS契约/CI **428/428**；真实HTTPS **9/9**（TAP另计1个文件测试）；真实CLI→平台→PG **13/13检查+1个Rust fixture**；无Node runtime **9/9**。这些集合不同，不把重复项累计成一个验收总数。

| 检查 | 实际命令（隔离 /workspace） | 结果 |
| --- | --- | --- |
| install-ci | `npm --prefix scripts/ci ci --ignore-scripts --no-audit --no-fund` | PASS |
| install-frontend | `npm --prefix frontend ci --ignore-scripts --no-audit --no-fund` | PASS |
| install-micro-vue | `npm --prefix mini-services/micro-vue ci --ignore-scripts --no-audit --no-fund` | PASS |
| install-micro-jquery | `npm --prefix mini-services/micro-jquery ci --ignore-scripts --no-audit --no-fund` | PASS |
| build-micro-vue | `npm --prefix mini-services/micro-vue run build` | PASS |
| build-micro-jquery | `npm --prefix mini-services/micro-jquery run build` | PASS |
| sync-micro-vue | `node scripts/sync-microapp.mjs vue mini-services/micro-vue/dist` | PASS |
| sync-micro-jquery | `node scripts/sync-microapp.mjs jquery mini-services/micro-jquery/dist` | PASS |
| cli-fmt | `cargo fmt --manifest-path tools/platform-admin/Cargo.toml --all -- --check` | PASS |
| cli-check | `cargo check --manifest-path tools/platform-admin/Cargo.toml --locked --all-targets --all-features` | PASS |
| cli-clippy | `cargo clippy --manifest-path tools/platform-admin/Cargo.toml --locked --all-targets --all-features -- -D warnings` | PASS |
| cli-tests | `cargo test --manifest-path tools/platform-admin/Cargo.toml --locked --all-targets --all-features` | PASS |
| cli-doc | `cargo doc --manifest-path tools/platform-admin/Cargo.toml --locked --no-deps` | PASS |
| cli-release | `cargo build --manifest-path tools/platform-admin/Cargo.toml --locked --release --bin saas-platform-admin` | PASS |
| js-contracts | `node --test --test-reporter=tap tests/platform-admin-contract.mjs tests/data-grants.mjs tests/data-grants-transport.mjs tests/auth-queries.mjs tests/route-policy.mjs tests/route-policy-guard.mjs tests/application-navigation.mjs tests/micro-context-runtime.mjs tests/micro-context-updates.mjs tests/micro-sandbox-lifecycle.mjs tests/micro-runtime-faults.mjs tests/micro-iframe-protocol.mjs tests/micro-sdk-client.mjs tests/micro-sdk-generation.mjs tests/notification-context.mjs tests/session-refresh.mjs tests/session-coordinator.mjs tests/session-integration.mjs tests/map-location-regions.mjs mini-services/micro-vue/smoke-test.cjs mini-services/micro-jquery/smoke-test.cjs business-apps/work-orders/frontend/tests/client.mjs business-apps/work-orders/frontend/tests/app.mjs business-apps/work-orders/backend/tests/cors.test.mjs examples/app-backend/authorize.test.mjs tests/package-microapp.mjs tests/ci-runner.mjs tests/ci-workflow.mjs scripts/ci/postgres.test.mjs scripts/ci/platform-admin.test.mjs scripts/ci/work-orders.test.mjs scripts/ci/linux-runner.test.mjs scripts/ci/release.test.mjs tests/browser/results.test.mjs tests/browser/wait.test.mjs tests/zip-release-e2e-policy.mjs` | PASS |
| cli-https | `node --test --test-reporter=tap tests/platform-admin-tls.mjs` | PASS |
| cli-platform-postgres | `node scripts/ci/platform-admin.mjs --out-dir /reports/postgres/live --source-revision 32956c4532bbd6eff53584fd0f5e9bb6e188437c` | PASS |

文档检查使用 `RUSTDOCFLAGS=-D warnings`。最终 runner 执行与正式 fast 相同的 TLS 专用 evaluator，同时核对单文件 TAP、完整九项 JSON、零失败/跳过、TLS私有文件清理和二进制摘要。真实平台 runner 核对精确13个ID、1个Cargo通过、实际SQL计数及临时数据库清理；parser另有7项缺失/重复/坏状态反例，CI evaluator 另有坏TLS报告回归。未运行整套 fast/browser/delivery，也未把已有平台 strict Clippy 债标为修复；本次新 CLI 严格 Clippy 通过，新增平台fixture定向 rustfmt/check 通过，先前平台tests Clippy的38条既有警告无新增fixture诊断。

Windows 固定 Node24.18.0 的 native 六步 PASS：13库+7离线安全测试；1个真实TCP测试明确过滤，Linux完整21项中实际执行且PASS。Windows 全契约355项为354通过/1跳过，因操作系统 EPERM 无法创建符号链接，故该轮整体FAIL；Linux最终契约已零skip。原始状态见JSON，不将过滤或跳过计为执行。

## 真实状态与安全证据

实际SQL：bootstrap应用 **1**、菜单 **7**，重复执行ID及计数保持；实际clientId只关联本应用。外来客户端、冲突菜单及已有发布版本拒绝。URL release **2**、ZIP artifact **0**，实际预检、两次激活、deployment读取和rollback完成。

CLI 子进程 env_clear，仅接收必要环境、平台URL/Bearer/实际clientId，无数据库URL。授权所有写入经HTTP；测试驱动SQL仅用于准备/断言隔离数据。实际功能授权四类PUT、旧角色集合保留及重复零写已验证。数据grant审计 **3** 条，并核对请求ID、actor及目标；错误根/独立read-write权限/错误组织或跨应用拒绝，同token下一次context/member请求消费新scope。临时数据库 `assertions_ok/database_removed/pool_closed` 均true。

HTTPS fixture 使用临时CA/IP证书和实际Rust二进制；验证受信认证请求、拒不受信CA/错误主机名/私钥CA、redirect、超大body、持续滴流共享deadline，以及合法成功字段Bearer回显和写后输出拒绝的未知结果提示。stdout/stderr 对私有canary逐项检查，私钥/证书目录最终移除。

无Node实证从已有不可变Debian镜像只复制同一release ELF，UID/GID10001、无node/npm/bun/cargo/rustc、动态库解析完整；只读根、cap-drop ALL、internal网络、无host端口/mount。help、默认plan和未apply bootstrap在私密ambient配置下运行，固定错误也不回显canary。临时容器/网络/镜像全部移除。

## 输入和产物绑定

- Git HEAD `32956c4532bbd6eff53584fd0f5e9bb6e188437c` 仅标识历史基点；实际未提交工作树由逐文件SHA标识。初始615项摘要 `c48d3a045081d8e2166c641af9c3e1f2c2f56deb5b19a0e5d57ea15172428d2d`。
- 最终Linux实际输入 **680项**，摘要 `226a131b47dc2e2a1e5a35959b68f63ee0d07fef7c3fb5ada0b64a4dd593770f`；容器中执行前后源码一致。最终Phase25及全部平台输入 **126项** 与当前字节核对一致；其它并行阶段差异在inputs报告独立列出，不还原或归入本阶段。
- Release ELF amd64 **5,714,704 bytes**，SHA256 `13d63b45fef71e1a17577e0e1af818aa431a2649faac56f50fa145a02d52dc44`；TLS、live及无Node运行均绑定同一摘要。仅保留本地候选产物，没有外部发行。
- 工具镜像 `sha256:7ad323a47d6d5202da34beb0f315d24dd10102d9bebd0726260c189768d0094c`；PostgreSQL postgres (PostgreSQL) 17.6 (Debian 17.6-2.pgdg12+1)，镜像 `sha256:8745eeb23be1055bd80c88e509a5247110cce0f2496c7492d165f3d0fc1975cd`，自有tmpfs/无host ports。Debian runtime `sha256:97049525bbb2470c8694cd9cab09d92d5784c937a9de8d2dbe1aa28baed2074d`。
- 平台Cargo.toml/Cargo.lock及旧迁移与本阶段起点逐项一致；无新增平台path依赖/迁移。两份保留的公开manifest与CLI内嵌版本契约一致。

逐文件清单见 [phase-25-inputs.json](phase-25-inputs.json)，命令、检查、镜像/DB/二进制及原始报告摘要见 [phase-25-runtime.json](phase-25-runtime.json)。本地原始日志在 `target/phase-25/linux-r3`、`native-r1`、`js-r1` 与 `runtime-r1`。

## 失败记录与边界

1. Linux r1 如实FAIL：新契约读取的工单联合fixture没有进入快照，且额外CI测试缺干净安装yaml；已补明确sourceFiles与依赖安装。真实平台首次bootstrap为403，fixture缺OAuth实际MENU授权；补齐与平台seed相同的 `/system/oauth/clients`，未削弱平台/CLI权限。
2. Linux r2真实CLI/平台/PG、21Rust与9TLS全部通过；全量JS另有两个示例public镜像文件未生成，426通过/2失败，整体FAIL。最终轮先实际构建两个Vite示例并用既有sync脚本生成被快照规则排除的public产物，再执行完整契约，不改测试预期或源码排除规则。
3. 复核发现fast只看TLS文件级TAP、忽略JSON，已补专用闭合报告门禁并加入反例；最终Linux实际执行该evaluator。
4. 无Node首次构建因本机legacy Docker不支持COPY --chmod而FAIL，未触发运行资源且清理完成；成功版采用兼容COPY+chmod，没有下载/安装。
5. Windows TCP loopback受限（SynSent）及符号链接EPERM分别记录；由Linux执行必需网络/符号链接验收，没有把环境失败改成通过。先前调试PID均已退出。

真实平台fixture走显式development loopback HTTP；生产TLS规则由独立实际HTTPS fixture验证。现有菜单DTO不暴露status，CLI核对identity/shape，最终授权有效性仍由平台策略判定。现有角色集合PUT及bootstrap无跨请求CAS/事务；在单管理员维护窗口执行，失败后先查询对账。没有生产DB、registry push、外部部署、ZIP上传或Docker生命周期管理功能。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| plan无外部写；重复bootstrap幂等且不抢占他应用客户端/菜单 | PASS | 本地计划不读取网络配置；实际DB全表快照/零HTTP写、重复ID计数及跨应用/菜单冲突拒绝 |
| 错误origin/redirect/超时/冲突固定类别，无Bearer/secret/正文泄漏 | PASS | 8安全集成+9真实HTTPS；成功字段回显拒绝；未知写保留对账；无Node ambient秘密验证 |
| register仅允许字段；data grant经管理API，无直接表写 | PASS | 严格schema/请求body字段闭集、实际URL发布生命周期、正式grant HTTP/权限/版本/审计及下一请求scope |

## 清理与回退

linux-r1/r2/r3的runner/PG容器及网络已由owner label核对后删除，cleanupErrors均空、WSL lease stopped/exit0；临时DB/pool与TLS私有目录已清理。无Node运行资源剩余0。Cargo缓存volume在最终归档前核验本阶段owner后删除，最终空查见runtime JSON的cleanupAudit补充。

回退应整体恢复本阶段CLI、调用入口、CI清单和旧5文件的同版本组合；不要留下指向已退役入口的脚本。停用CLI不会撤销已完成的管理写入，先inspect并通过正式平台管理API审阅修复，不删现存应用、菜单、角色或发布历史，不以SQL自动逆向撤销。

完整结构化台账更新见 [phase-25-ledger-update.md](phase-25-ledger-update.md)。
