# UUID-U05 SDK 发布与消费者准备记录

阶段整体状态：**PENDING**。本记录仅证明 SDK 新发行登记、活跃消费者固定副本和无数据库代码验证通过，不代表 UUID-U05 整体验收完成。新目标三库运行、实际部署、配置引用映射、旧凭据失效和跨系统端到端验证尚待 U04 后完成。

六个运行服务的环境字段、既有配置合同 UUID 路径、轮换/丢弃规则见[只读运行字段目录](uuid-u05-runtime-field-catalog.json)。该目录没有配置值或秘密，状态明确为运行尚未验收，不能用它替代真实新 epoch 的部署证明。

执行日期：2026-10-24。范围为本地工作区中的六个独立仓库；没有连接数据库、启动真实业务服务或发布外部 npm 包。前端测试使用独立临时目录及可清理的静态 HTTP fixture，没有操作冻结预览服务。

## 新发行及协议边界

SDK 登记版本为 `3.0.0`，状态为 `REPOSITORY_CANDIDATE`。根工具包、根导出和 `releases/3.0.0/` 一致。SDK 来源修订为 `67031f14b522d49b3de63e0c1fc8bf7fe5fc7aa9`，消费者明确记录 `workingTree: true`，没有将未提交源码冒充已发布版本。

3.0.0 使用固定 `uuid@13.0.0` 和浏览器 `crypto.getRandomValues` 生成 UUID v7，拒绝系统身份中的 v4、nil 和错误 RFC variant；旧请求标识不沿用。SDK 主版本变化反映运行时输入兼容性变化。以下独立 wire 版本及现有字段保持不变：

| 合同 | 版本 |
|---|---|
| MicroProps | `2.0` |
| AppAuthorizationContext | `1.0` |
| Notification | `1.0` |

当前 SDK 支持集合仅含 `3.0.0`。省略 sdkVersion 的既有 MicroProps 基线仍保留；显式旧版 SDK 被拒绝。没有增加员工、客户、SSO 等后续身份方案字段。

当前 Schema 对现有系统 UUID 字段增加 v7/variant pattern；生成 Rust DTO 的显式上下文、组织范围、角色、数据授权、菜单和通知校验同步更新。公共 fixture 使用固定、无凭据的 v7 示例，平台 `public_contract.rs` 与独立消费 fixture 保持逐字节一致。直接 serde 反序列化不自动执行 validate 的既有边界没有改变。

## 活跃消费者

| 消费者 | 固定副本及来源检查 |
|---|---|
| 平台前端 | `frontend/vendor/micro-app-sdk/`，SOURCE、manifest、包版本与完整发行一致 |
| 门店前端 | `frontend/vendor/micro-sdk/`，SOURCE artifacts、发布门禁、validContext 和 backend/dependencies.lock.json 同步 |
| 工单前端 | `frontend/vendor/micro-sdk/`，SOURCE files、实际 UMD SDK 允许集合、发布门禁和 backend/dependencies.lock.json 同步 |
| Vue / jQuery 示例 | 各 public/vendor 中的 classic sdk.js 固定副本及 SOURCE |
| 报表 / OAuth 前端示例 | 完整 vendor/micro-sdk 发行及 SOURCE |
| WebSocket 示例 | 新增 vendor/micro-app-sdk/3.0.0 并切换活跃 import；原 2.0.0 目录保留 |

五个示例统一的 `dependencies.lock.json` 固定 46 个文件和 5 个 SOURCE。门店、工单各自的仓库输入检查验证 24 个 pins。历史业务发行版本号没有因 SDK 升级被批量替换。

## 已执行验证

Windows 使用 Node.js `24.22.1`；SDK、门店和工单的相关 npm 门禁使用 `11.16.0`，通过 npm exec 的固定版本工具执行。平台和示例的部分命令使用本机 npm `11.19.1`，满足这些包的 `>=11.16.0 <12` 引擎约束，不把它记为 npm 11.16.0。Rust 使用仓库现有工具链及锁定依赖。

| 范围 | 命令或门禁 | 实际结果 |
|---|---|---|
| SDK | npm run build / npm run check | 20 个生成产物通过确定性与逐字节检查 |
| SDK | npm test | 30/30，通过同毫秒/回拨、安全随机、旧 ID 拒绝、独立声明消费及 classic 实际 bundle v7 请求 ID 验证 |
| 平台前端 | npm test | 398/398 |
| 平台前端 | sdk:check / typecheck / build | 通过，实际 dist 的 qiankun 门禁确认 sdkVersion=3.0.0、buildChecked=true |
| 工单前端 | npm test | 90/90，含构建、实际 UMD 执行及发布门禁 |
| 门店前端 | npm test / typecheck | 149/149；TypeScript 通过，含构建和发布门禁 |
| Vue 示例 | npm test | 12/12 |
| jQuery 示例 | npm test | 11/11 |
| 报表示例 | npm test | 8/8，含实际 UMD 构建消费 |
| WebSocket 示例 | npm test | 9/9，含 typecheck 和构建 |
| OAuth 前端示例 | npm test | 18/18，含隔离构建和生命周期 |
| 平台后端 | cargo test --bin saas-platform-backend public_contract_ | 11/11，包含新增 nested ID/旧 SDK 拒绝、独立消费和来源字节检查 |
| 平台后端 | rustfmt --check src/public_contract.rs src/public_contract_tests.rs | 通过 |
| 来源与摘要 | 平台 SDK 检查、两个业务仓 check-inputs、示例 check-inputs | 全部通过 |
| 变更完整性 | 相关 git diff --check | 通过 |

前端第一次全量测试暴露两处仍使用短字符串身份的 SDK fixture，已改为 v7 后 398/398 通过；工单首次测试暴露旧门禁错误文案正则，修复后 90/90 通过。没有放宽运行时检查。曾执行 `cargo test --lib public_contract_` 得到零匹配测试，未将它作为验收证据；上表使用实际包含公共协议测试的主 binary target。

## 新发行摘要与历史保原

| 文件 | SHA-256 |
|---|---|
| releases/3.0.0/artifact-manifest.json | `02fe52794596edf08e8220c60aff3640a1401f0416ebec71ca0d37b154c40b47` |
| releases/3.0.0/index.mjs | `48922ad23bfddc9d65b8d18b61769940b987405ab4e749a7003c86552734f067` |
| releases/3.0.0/sdk.js | `cd4143e916c4aba29cccbf1974b7b355a6aed6141a4ed45b657c37b2c94bfb71` |
| releases/3.0.0/protocol.rs | `082aae25785550ebed9461cb108c70bb6d671122ed63af6134677f6c2ba51fd8` |
| releases/3.0.0/schema.json | `51d0e9a49701e3aa85dae1c3d1b83fe80f33a5b9c79f281b5ae14988cbb87dae` |

SDK 1.0.0 和 2.0.0 的 manifest 摘要分别仍为 `d3137ecd7588e59df7151b5c533adfdc0a72eedf36138f038928538cf914b1dc`、`2e6007209f63dba709157eb6cd37ef7c4961c679e10b2dbbfad6690836143c3c`；SDK 测试同时逐文件核对历史 manifest 中的大小和摘要。升级前后保存的 70 个冻结文件摘要全部一致，覆盖上述两版 SDK、旧 WebSocket 2.0.0 副本及登记的历史部署文件。门店/工单历史 artifacts、旧已签文件和不可变发行目录未被重写。

## 尚未验收

- 新目标三库对应的六个实际预览服务尚未由本记录启动或测试。
- runtime env、policy、manifest、应用登记和外部引用尚需通过 U03 权威实体映射生成并核对；签名/哈希须由对应 owner 重建。
- 旧 session、access、refresh、code、device、app token、JWT jti 的失效和新身份重新认证尚未由本记录验证。
- 平台、门店、工单、通知、OAuth 和外部 RP 的真实新 epoch 端到端创建/读取/关联/回调/拒绝旧 ID 尚待执行。
- 没有将源码或 vendor 准备完成写成最终部署完成，也没有以本记录把 U05 台账改为 COMPLETED。
