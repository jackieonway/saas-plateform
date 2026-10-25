# Project Task Ledger

> UUID v7 专项实施台账，版本 v0.8，2026-10-24。UUID-U01～U06 均已完成。U06本地final切换、真实新写和recovery保数据恢复、技术验收及最终九库只读审计通过；恢复核心读写累计909.035秒超原600秒目标，用户于2026-10-24明确接受本轮偏差并要求完成U06，当前状态COMPLETED。原超时测量、失败和计时保留，后续演练目标不因此改为909秒。当前预览使用recovery，T1的666,815条保留行及53个delta变化完整核对，旧凭据两代拒绝、SDK3／253实际HTTP字节及147UUID列／67FK通过。原v4、U05源、final和认证包保留。范围、失败及限制见[U06结果](validation/uuid-u06.md)和[机器摘要](validation/uuid-u06-summary.json)。身份Phase1由另一个用户聊天完成、独立身份指针仍为Phase2，本专项不修改其台账或工作树。

> 范围澄清：用户另行明确员工／客户与 SSO 新功能使用无历史数据的新库。该身份新库以空库 UUID v7 基线作为 Phase 1 首项，不依赖本专项的历史映射、保数据导入或 U06。下文“专项完成后才能实施身份”的要求仅适用于本专项正在重编号的预览源／目标库；不得在其 UUID epoch 切换期间叠加身份迁移。独立身份新库不读取或借用这些主体，不取消本专项，也不把其未完成阶段计为完成。

## 1. Project Overview

### Project Name

系统自有 UUID 全面采用 v7，并通过保数据重编号与新库重建完成存量切换。

### Project Goal

审查所有自有运行时 UUID 生成点、数据库 schema、跨库软引用和对外协议；未来新生成的持久 UUID、请求 ID 统一采用 UUID v7。对需要保留的既有业务数据按实体建立一次性 old→new 映射，初始化全新独立目标库，转换并导入数据，再协调切换服务。UUID 以外的业务编号、金额、日期、密码哈希和安全随机凭据维持各自契约。

### Background

- 工作区是多个独立仓库，不是统一 Cargo workspace。重点数据库边界包括平台、门店和工单三库；实际环境、实例、主机、库名和其他持久存储尚需 UUID-U01 清点，不能把“三库”当成已覆盖全部生产数据。
- 静态审查已发现：平台 11 个历史迁移中有 39 个 `CREATE TABLE`、107 个 UUID 列、23 个 `gen_random_uuid()` v4 默认、64 行包含 `REFERENCES`，未发现 UUID v7 CHECK 或 `ON UPDATE CASCADE`。这些是源码扫描结果，不是实际生产 catalog 统计。
- 排除 vendor/构建/历史发行的 Rust `.rs` 扫描有 229 处 `new_v4` 符号引用，含测试：225 处带括号调用和 4 处函数值传递，后者也是真实生产生成入口，不是 vendor 命中。生产清单为 19 个文件中的 24 处，其中 jitter 的 1 处应改随机 `u8`，3 处发布 attempt nonce 可用 v7 并保留栅栏复验，JWT jti 可用 v7 但 token 必须重签。
- 五个 UUID Cargo 包配置当前启用 v4 feature，锁定依赖已支持 v7；另有 1 处生产内嵌 SQL 用 `gen_random_uuid()` 生成 cleanup `request_id`。历史迁移不能直接改写。
- 前端自有源码有 9 处 `crypto.randomUUID()` v4；Node 兼容 `.mjs` 工具另列清单。SDK 固定 release／vendor 中有 19 处，必须通过新版本发布更新消费者，历史发布字节不能修改。
- `installation.rs:783` 当前仅接受 v4 stage 目录，OAuth `jti` 测试存在版本 4 断言，均需按新契约调整。nil 拒绝哨兵、明确的 v4 负面 fixture、冻结历史产物属于有意例外，不能当作活业务 ID。
- 已对本地三套预览库执行 `REPEATABLE READ READ ONLY`、10 秒 statement timeout 的聚合 SELECT；[只读清单](uuid-v7-inventory.json)保存 150 条数据库／列聚合记录，不含 UUID 原值。本地 PostgreSQL 为 18.6，但 CI／恢复仍为 PG17，不据此宣布系统已统一升级 PG18。
- `qiankun_platform_preview_20261024` 为 107 个 UUID 列、23 个 v4 默认、64 个 FK；非空 UUID 单元格 2706 个，全部 v4，v7 为 0。同日门店预览库为 23 个 UUID 列、2 个 FK、非空单元格 28 个，全部 v4；工单预览库为 17 个 UUID 列、1 个 FK、非空单元格 0 个。
- 上述数字统计非空 UUID 单元格，包含 FK 重复，不能当实体数。审计仅覆盖本地 preview，`backend.env` 中配置的远程开发库未连接，生产库未审计；不能声称全部实际存量 UUID 已核验。
- 用户“已有数据调整为 v7 后数据库重新初始化”按保留业务数据解释：导出、重编号、初始化全新目标库、导入与切流；不是清空现库后重新 seed。

### Success Criteria

- 所有系统自有运行 UUID 生成点采用 v7，或被明确认定为安全随机值而使用相应 CSPRNG；所有新增 typed UUID ID 输入核验版本与 variant。
- 目标 schema 在现有 PostgreSQL 17 基线上运行，使用应用显式写入 UUID v7、可兼容的约束和追加迁移，不为此强制升级 PostgreSQL 18，不自制 SQL UUID 生成算法。
- 保留范围内每一实体主键仅分配一次 v7，跨平台／门店／工单引用复用同一权威映射；UUID 列、数组以及登记的 JSON／文本路径不产生悬空引用。
- 新目标库由真实迁移初始化，seed 与旧数据导入互斥；约束、触发器、schema fingerprint 和迁移记录真实一致，业务计数、金额与关键不变量与源快照对账。
- 所有旧登录／session／access／refresh／code／device／派生凭据失效，旧 JWT 不通过改 `sub`／`jti` 或字符串替换继续使用；新凭据重新认证签发。
- SDK、vendor、部署配置、深链接及外部 RP 引用通过新版本和协同映射切换；历史签名证据与冻结产物保持原字节并保留映射说明。
- 切流有停写边界、最终快照、独立目标库、完整恢复证据。旧库留存只读；切流后若已发生新写，必须迁回／重放，不能直接切旧库丢掉新数据。

---

## 2. Global Constraints

### Technical Stack

- 沿用各仓库 Rust／Axum／SQLx／PostgreSQL、浏览器／Node、SDK 工具链及锁文件，不把工作区强行改成单一构建工程。
- Rust 新 UUID 采用锁定 `uuid` crate 的 v7 能力与 `Uuid::now_v7()`，由统一生成入口管理。离线历史时间构造采用受支持的 v7 库 API 和 CSPRNG，不直接修改版本字符或拼 SQL 位段。
- PostgreSQL 17 作为当前兼容基线：应用为持久 UUID 和请求 ID 显式提供值，增量迁移移除 v4 默认并补 v7／variant 约束。即使将来使用 PostgreSQL 18，也保持统一应用生成策略。
- 浏览器与 Node 使用经过审查的 UUID v7 实现与安全随机源；SDK 从可维护源码产生新版本，不能把 `crypto.randomUUID()` 当成 v7。

### Architecture Constraints

- 本专项建立一次 UUID 身份 epoch 切换，优先于原方案“原 `users.id` 保持不变”的假设。重建后新 `users.id`／OIDC `sub` 再作为稳定主体；原主体到新主体的映射长期受控保留，并协同所有外部 RP／集成。
- 映射键为 `authority.entity + old_primary_key`，不是全库无类型字符串替换。同一权威实体只分配一次新 UUID；其他库的软引用按权威实体引用复用，不能各库独立生成。
- 映射类型覆盖持久 UUID 主键、外键、软引用、UUID 数组、登记的 JSON／文本路径、环境与配置引用；不同实体偶然相同的旧 UUID 不得误合并。
- 可信 `created_at` 可用于历史 UUID v7 时间部分；缺失、不可信或异常时间使用已冻结的迁移时间规则。为每个值提供足够安全随机位，不能只把 v4 的版本位改为 7。
- 映射、导出清单、schema fingerprint、版本、快照时间和对账摘要形成同一可验证迁移包。变换器离线处理受控导出，不边读活库边生成不一致快照。
- 目标数据库必须是明确登记、解析检查后的全新独立目标库。平台／门店／工单迁移各自真实执行，不修改 `_sqlx_migrations` 等记录伪造 schema 一致性。
- 自有代码、SDK 源码和消费配置可以升级；固定历史 release、已签证据、旧审计归档按保留策略保原并附映射，不追求源码文本“零 v4”。

### Business Constraints

- 先确认保留哪些账号、业务记录、业务计数器、审计及归档；默认保留业务数据和非 UUID 业务值，不默认丢弃历史订单、门店或工单。
- `bigint`、业务编号、金额、日期、散列、客户端业务名称和非 UUID `client_id` 不强改成 UUID。若既有编号生成器依赖 UUID，登记并验证其实际业务合同。
- UUID 数组和 JSON／文本仅处理有类型登记的 ID 路径，不能对用户自由文本执行全局正则替换。
- 外部已签发或第三方拥有的 ID 不擅自重编号；作为外部关联保原。自有主体映射影响既有 RP／webhook／深链接时，由相应 owner 协同切换。
- 本专项不实现员工／客户或 SSO 新能力；如后续在本专项的预览源／目标库实施身份功能，须先完成本专项，避免同一库的身份模型和 UUID epoch 同时迁移。独立身份新库按其台账验收空库 v7 基线，不依赖本专项历史搬迁。

### Security / Compliance Constraints

- 密码、secret、授权 code、refresh、PKCE verifier、OIDC nonce 等是安全随机凭据，保留合适长度和熵的 CSPRNG，不能统一替换成带时间信息的 UUID v7。
- 通知 jitter 原 UUID 首字节用法改为专用随机字节；UUID v7 首字节主要是时间，不能充当随机退避。发布 attempt nonce 仍须验证栅栏与原子性。
- 输入的 typed UUID ID 验证 version 7 和 RFC variant；nil、v4、错误 variant 和非 UUID 拒绝。外来 request ID 采用拒绝或重发 v7 的明确合同，原非 UUID 关联值只能保存为外部 correlation 字段。
- ID 重编号是认证主体变更，全部旧会话和派生凭据失效；不能仅更新 JWT 数据库行或签名字符串。新 `jti`、新 token 必须重新签发与签名，密码哈希是否含主体派生因素需要审查。
- 签名、哈希和 manifest 的完整性不能通过字符串替换伪装。自有可重建产物按 owner 重新生成／签名；不可重签历史证据保原字节并建立映射，登记其外部验证方式。
- 导出和映射含敏感身份关联，具备最小读写权限、加密存储／传输、审计和保留期限；日志与证据不输出密码、token、私钥或全量个人资料。
- 停写包含 HTTP／CLI、后台 worker、scheduler、replay、outbox、通知投递与外部写入，不只是停止网页入口；冻结期间不允许旧服务向旧／新库继续补写。
- 任意恢复/回退都在开放入口前重新推进不会随备份倒退的认证失效边界，撤销旧 session/refresh/code/device 并验证旧 JWT 拒绝。恢复还须冻结 agent、对账库外效果，用迁移 epoch 或等效任务栅栏拒绝旧 nonce/lease/receipt 重放；“新库无写入”不能替代这些条件。

### Code Style / Engineering Rules

- 遵守现有工程规则，采用小范围公共 UUID 生成／解析入口，逐调用核实用途，不机械替换全部 `new_v4`。
- 原迁移不改，追加 DROP v4 默认和校验迁移，逐一适配 seed、测试 fixture、所有省略 UUID 的 INSERT、cleanup SQL、CLI、导入和工具脚本。
- UUID version 验证必须与类型／用途校验分开；外部业务字符串不是 UUID 时按原合同处理，不以 v7 验证扩大拒绝范围。
- 每个阶段在所属仓库执行最小充分检查与必要隔离库／协议／端到端验证；数据库 `#[ignore]` 用例须显式运行，不能用普通单测结果替代。
- 每阶段证据路径为 `validation/uuid-uNN.md`。已完成阶段的实际证据见状态摘要；未完成阶段的证据须在真实执行后生成，记录环境范围、命令、结果、限制和产物摘要，不以未来路径宣称已经验收。
- 测试包含版本与 variant、并发唯一性、类型化引用、数据对账、恢复和拒绝旧凭据等真实边界，不以重复实现的断言替代验收。

### Forbidden Actions

- 按用户后续全量开发授权逐阶段实施；实际数据库操作仅限明确登记的本地预览源与全新独立目标，远程开发库和生产库不在范围内。历史 SDK 和历史 migration 字节保持不变。
- 不执行 `DROP DATABASE`／`TRUNCATE`／现库清空，不根据“所有数据库”这种泛称连接并重建；实际重建仅限之后明确登记的独立目标库。
- 不在每个库各自生成同一权威主体的新 ID，不使用无类型全局 old UUID→new UUID 替换，不将 UUID v4 版本字符改成 7。
- 不通过伪写迁移记录、关闭后不恢复约束／触发器、忽略 append-only 不变量或跳过 schema fingerprint 来让导入“通过”。
- 不改冻结历史发布字节，不将已签 JWT／manifest／审计证据进行字符串替换后继续宣称签名有效。
- 不把所有随机值换成 UUID，不要求 PostgreSQL 18 升级或自制 SQL v7 算法，不因保留 v4 负面 fixture 而声称仍有活业务 v4。
- 不在新库有写入后直接回旧库，不把数据丢失的切换称为回滚成功；反向迁移／重放方案必须先验证。

---

## 3. Phase Status Summary

| Phase | Name | Status | Depends On | Output Summary |
|---|---|---|---|---|
| UUID-U01 | 目标环境、全量 catalog 与导出合同 | COMPLETED | - | 可重复只读 CLI、147 列／53 表／67 FK 精确校验、注册表和导出合同；见 validation/uuid-u01.md |
| UUID-U02 | 统一生成、输入校验与追加 schema 迁移 | COMPLETED | UUID-U01 | v7 生成/输入、147 列约束、显式 INSERT/seed、PG17 回归完成；见 validation/uuid-u02.md |
| UUID-U03 | 离线导出与类型化重编号变换器 | COMPLETED | UUID-U02 | 666,750 源行、562 映射、同 map 两次转换一致，271 文件完整认证回读；见 validation/uuid-u03.md |
| UUID-U04 | 独立目标三库初始化、导入与恢复合同 | COMPLETED | UUID-U03 | 固定目标真实迁移、失败恢复重建、666,610 行认证导入与独立复核；见 validation/uuid-u04.md |
| UUID-U05 | SDK／协议／配置与全系统一致性验收 | COMPLETED | UUID-U04 | SDK 3 实际 Node／浏览器消费、七正式 release、新凭据／协议、业务／通知正向及授权撤回、最终只读复核；见 validation/uuid-u05.md |
| UUID-U06 | 协调停写、最终重建导入与切流恢复 | COMPLETED | UUID-U05 | RPO=0及全部技术审计通过；用户明确接受本轮RTO909.035秒相对600秒的偏差，原失败／计时保留 |

允许阶段状态仅为 `NOT_STARTED`、`IN_PROGRESS`、`BLOCKED`、`COMPLETED`。本专项预览源／目标库只有在专项完成且证据齐备后才能叠加身份迁移；独立身份新库的 Phase 1 另按空库 v7 基线验收。原 13 个身份阶段编号不变。

---

## 4. Phase Details

### Phase UUID-U01: 目标环境、全量 catalog 与导出合同

#### Status
COMPLETED

#### Goal
把静态审查扩展为明确目标环境的全量只读事实清单，冻结存量保留、类型化引用和导出合同。

#### Scope
仓库／运行产物／SDK／脚本生成点、实际数据库 catalog 和 UUID 版本／variant 计数、跨库软引用、外部 RP、配置／对象存储引用、保留范围与迁移包格式。

#### Out of Scope
业务生成代码修改、生成实际新 ID、删除数据、重建数据库、直接连接未登记的开发／生产环境。允许开发本阶段只读清点工具、注册表及其验证。

#### Inputs
本台账静态审查事实、`uuid-v7-audit.md`、`uuid-v7-inventory.json` 与完整方案；实际环境连接信息由目标环境清单提供。已审计只读预览库的 2706／28／0 个非空 UUID 单元格不等于实体数，也不能代替远程开发／生产审计。

#### Execution Requirements
- 用只读 catalog 枚举每个已登记库的 UUID 列、默认、约束、FK、索引、触发器、视图、函数、数组及 migration fingerprint；版本／variant 和 nil 计数按实体列输出摘要。
- 逐项登记 JSON／文本／数组／配置／深链接中的类型化 ID 路径，映射到 `authority.entity`，包括平台主体在门店／工单的软引用和外部 RP 的 `sub`。
- 把生产生成点、测试负例、冻结历史、第三方／vendored 源码、Node 工具与安全随机值分类；不把 229 处 Rust 文本命中说成 229 个生产发号入口。
- 冻结保留范围、计数与业务计数器、快照一致性方式、历史时间规则、导出／映射格式、签名重建责任和 schema 指纹交接合同；未回答项按对应阶段处理。

#### Expected Outputs
目标环境／主机／库名解析清单、全量 catalog／版本摘要、跨库引用与生成点矩阵、导出与保留合同、`validation/uuid-u01.md`。

#### Acceptance Criteria
- [x] 每个声明已审计的实际环境有明确连接目标、只读证据、catalog 和计数时间；未访问的环境显式标注未审计。
- [x] 每个自有持久 UUID 主键／引用、运行生成点和外部入口有类型、owner 与处理规则；已识别 JSON／文本路径和外部 RP 关联。
- [x] 导出合同能表达主数据、排除／撤销凭据、业务不变量、schema 指纹和受控恢复；不需要的 PostgreSQL 18 升级未被引入。

#### Risks
只有列类型清单却遗漏软引用；测试／历史命中被误算生产；错误连接目标；活库导出跨库时点不一致。

#### Dependencies
无前置专项阶段。Q1／Q2 在实际环境审计与保留合同确定前处理；其他未知信息可先登记，不要求一次回答全部。

#### Notes
此阶段结束应能回答“哪些实际库已审、还有哪些未审”，不能以预览库替代全部生产环境。

---

### Phase UUID-U02: 统一生成、输入校验与追加 schema 迁移

#### Status
COMPLETED

#### Goal
让新代码和全新 schema 在 PostgreSQL 17 上显式生成／接收 v7，清除自有活业务 v4 发号路径并保留有意例外。

#### Scope
五个 Rust UUID 依赖 feature、统一 `now_v7` 入口、浏览器／Node／SDK 源码生成器、typed ID 校验、追加迁移、所有 INSERT／seed／CLI／cleanup 和版本相关目录／测试适配。

#### Out of Scope
真实存量主键重编号、生产迁移、新库导入、改历史 migration 或 SDK 固定 release／vendor 字节。

#### Inputs
UUID-U01 生成点与 schema 清单、锁文件、原迁移、`installation.rs` v4 stage 校验、OAuth `jti` 断言、通知 jitter 与发布 attempt 栅栏调用链。本地预览 PG18.6 与 CI／恢复 PG17 差异必须保留在验证矩阵。

#### Execution Requirements
- Rust 自有持久／请求 UUID 使用统一 v7 生成入口；按用途逐一替换 24 个生产调用及自有 vendored telemetry 来源，新增或更新已审查 v7 feature，不做无关依赖升级。
- 浏览器／Node 的新 UUID 采用安全 v7 实现；SDK 修改可维护源码，固定副本留待 UUID-U05 新版本消费。所有输入 typed UUID 同时验证版本和 variant，外部 request ID 合同按 U01 处理。
- 追加迁移 DROP 23 类 v4 默认及其他实际 catalog 发现的 v4 默认，采用 PG17 兼容的 v7／variant 约束；原迁移不改，不依赖 PG18 专有函数。旧数据不原地通过该约束，实际迁移先在新空目标 schema 验证。
- 所有省略 ID 的 INSERT、seed、CLI、导入入口和 cleanup `request_id` 显式提供 v7；从最早历史迁移到最新迁移的全新库路径必须真实可执行。
- jitter 改随机 `u8`，安全 secret／code／refresh／PKCE／OIDC nonce 保留 CSPRNG；发布 attempt nonce 可 v7，但保留并发栅栏。stage 目录接受新 v7 合同，OAuth `jti` 断言改验 v7／variant，旧凭据不重写 JWT。

#### Expected Outputs
统一生成／输入模块、源代码与依赖 feature 改动、追加迁移和所有 INSERT 适配、例外清单及回归用例、`validation/uuid-u02.md`。

#### Acceptance Criteria
- [x] 新自有运行 UUID 均为 v7／合法 variant，并发唯一性与输入拒绝覆盖 nil、v4、坏 variant；安全随机值与业务编号未被误改。
- [x] PG17 全新隔离库从历史迁移到最新迁移成功，所有 seed／INSERT／cleanup 有显式 UUID；约束阻止活业务 v4，新旧 migration 记录未伪造。
- [x] 通知 jitter 不再取 v7 时间前缀；发布 attempt 栅栏、stage 目录与 `jti` 回归通过，保留的历史／负例有明确理由。
- [x] 所属仓库构建和必要静态检查／单测／显式数据库测试通过；未发布固定 SDK 副本没有被原地改写（发行登记比较留 U05）。

#### Risks
新增 CHECK 导致源存量库启动自动迁移失败；省略 ID 的隐藏 INSERT；用 v7 时间字段代替随机熵；改 SDK 源码却遗漏消费发布。

#### Dependencies
UUID-U01 完成；追加迁移不得自动作用于仍含旧 UUID 的源库，实际发布／连接策略在 U04／U06 受控执行。

#### Notes
本阶段负责新代码与目标 schema，不通过给源库旧行改版本字符来满足新约束。

---

### Phase UUID-U03: 离线导出与类型化重编号变换器

#### Status
COMPLETED

#### Goal
生成可审计、可复跑且跨库一致的保数据转换包，同一权威实体的新 UUID 只分配一次。

#### Scope
受控快照导出、实体映射注册表、时间规则、UUID／数组／JSON／文本路径转换、引用闭包校验、业务不变量对账及迁移包完整性。

#### Out of Scope
直接改现库 PK／FK、全局正则替换、重写签名证据、将导出的活动凭据作为新库有效登录状态恢复。

#### Inputs
UUID-U01 导出合同与引用类型、UUID-U02 v7 实现、已批准的保留快照和时间规则、实际 schema fingerprint。

#### Execution Requirements
- 先为 `authority.entity + old_primary_key` 分配唯一新 v7 并固化注册表；重跑复用已存在映射，跨平台／门店／工单引用调用权威实体映射，不各自随机生成。
- 使用可信 `created_at` 或已冻结的迁移时间构造 v7，保持足够 CSPRNG；记录时间来源、不可信／缺失处理和碰撞保护，禁止只替换版本位。
- 按已登记类型改 UUID 主键、FK、软引用、UUID 数组、JSON／文本路径与配置清单；发现未登记引用或缺映射时失败，不猜测字段意义。
- 对签名／哈希派生字段分流到 owner 的重建合同；历史外部证据保原并附映射。活动 token／session／code／device 不作为可用凭据导入；可保留的审计行按撤销／归档规则处理。
- 输出每实体行数、主外键闭包、业务编号／金额／日期／哈希等不应变化字段的对账与摘要，保护映射文件和个人数据。

#### Expected Outputs
离线导出／转换工具、版本化映射注册表与 schema 描述、转换数据包、引用与业务对账摘要、`validation/uuid-u03.md`。

#### Acceptance Criteria
- [x] 同一迁移包重跑产生相同映射；同一权威主体在三库引用完全相同，不同类型同旧 UUID 不误合并。
- [x] 全部映射目标为合法 v7／variant；缺时间、异常时间、UUID 数组和嵌套 JSON／文本路径均有可控验证。
- [x] 缺失／多义引用导致明确失败；引用闭包、行数和业务不变量对账通过，原账号主体映射可长期追溯。
- [x] 没有自由文本全局替换，没有已签 JWT 字符串修改，没有活动旧凭据被静默恢复；源库保持只读。

#### Risks
扁平 old UUID 字典混淆实体；多次生成导致跨库主体分裂；不可信时间造成不正确历史排序；隐含文本引用未登记。

#### Dependencies
UUID-U02 完成；Q2 保留范围、Q3 快照规则、Q5 历史时间规则在生成真实转换包前确定，测试可使用标注清楚的隔离 fixture。

#### Notes
UUID v7 时间不是业务创建时间的替代字段；业务 `created_at` 自身保留原值，异常时间仍按合同处理。真实源包 666,750 行，经 562 个类型化映射生成 666,610 目标包行；119 凭据行和 21 metadata 行按合同排除，原证据留存。两次转换及 53 张 target／proof 摘要完全相同，完整认证回读 271 文件／1,628,018,517 明文字节。原三库仍只读且原 v4 单元格计数保持；目标创建、导入和切流未在 U03 执行。见 [验收](validation/uuid-u03.md) 与[脱敏摘要](validation/uuid-u03-summary.json)。

---

### Phase UUID-U04: 独立目标三库初始化、导入与恢复合同

#### Status
COMPLETED

#### Goal
在全新独立目标库真实完成 schema 初始化与保数据导入，并证明约束、不可变记录和恢复合同成立。

#### Scope
平台／门店／工单独立目标库、真实迁移与 schema fingerprint、seed／import 互斥、受控导入通道、触发器／append-only 不变量、服务冻结及隔离恢复演练。

#### Out of Scope
DROP／清空源库、伪写 migration 记录、对未明确目标的“所有 DB”批量重建、正式全量流量切换。

#### Inputs
UUID-U03 数据包和映射、UUID-U02 代码／迁移、UUID-U01 目标环境合同、owner 提供的受控导入和 schema fingerprint 交接规则。

#### Execution Requirements
- 为每个目标解析并记录环境、主机、端口、实际库名与角色，检查是全新目标且与源库不同；实际初始化前依据明确目标合同执行，不能把生产现库当空白目标。
- 从真实历史 migration 起执行到最新追加迁移，再校验结构与 migration fingerprint；无数据初始化 seed 与旧数据 import 是互斥模式，导入模式不能先生成冲突演示／业务数据。
- 不因源 schema 缺 `ON UPDATE CASCADE` 而原地更新主键；按依赖顺序导入映射后的新行。约束、不可变／append-only 触发器需要专用受控导入设计，不能静默禁用后遗留失效或伪造成功。
- 初始化／导入期间停全部 worker、replay、outbox 和外部写入，只允许受控导入角色；导入后恢复正常约束／角色并检查 FK、UUID 版本、业务不变量与 schema fingerprint。
- 演练快照／映射包备份、导入失败清理独立目标、重新创建目标和恢复；旧库保持只读，不删除原数据。

#### Expected Outputs
明确目标库初始化／导入工具与配置、真实迁移及指纹记录、seed／import 互斥、受控导入与恢复手册、`validation/uuid-u04.md`。

#### Acceptance Criteria
- [x] 独立目标三库从真实 migration 初始化并成功导入，目标／源解析核对无误，没有源库 DROP／清空或伪迁移记录。
- [x] seed 与 import 同时选择被拒绝；约束、append-only／不可变触发器、FK 和角色权限导入后有效，schema fingerprint 与真实结构一致。
- [x] 源快照与目标在保留范围行数、业务计数器、金额及关键不变量上对账通过，UUID 目标列／类型化引用为 v7 且无悬空引用。
- [x] 隔离恢复演练可复现，冻结期间所有后台写入确实停止，源数据和映射包仍可用于恢复。

#### Risks
连接串指向现库；启动 seed 冲突；触发器阻止导入或绕过不变量；伪指纹掩盖结构偏差；后台 worker 抢写。

#### Dependencies
UUID-U03 完成；实际目标环境、Q6 schema 指纹／受控导入合同及 Q3 恢复目标在相关操作前确认。

#### Notes
隔离／预演目标已完成，实际证据见 `validation/uuid-u04.md`、`validation/uuid-u04-summary.json` 和 `validation/resume-audit-20261024.md`。U04 恢复只覆盖失败且未开放的固定目标组；U06 仍需新鲜源等价证明及独立最终目标，不能把预演快照或 U04 重建冒充最终切流、新写后的恢复。

---

### Phase UUID-U05: SDK／协议／配置与全系统一致性验收

#### Status
COMPLETED

#### Goal
让所有实际消费者使用支持 v7 的新版本，校正身份与配置引用，证明新 UUID epoch 下全系统协议与凭据失效成立。

#### Scope
SDK 新版本及 vendor 消费、浏览器／Node 工具、应用与 OAuth 协议、policy／env／manifest／runtimefiles／深链接、外部 RP 协同和旧凭据全失效端到端验证。

#### Out of Scope
原地修改历史 SDK release、给旧 JWT 改 ID 后继续使用、未经协调改第三方标识、正式最终停写切流。

#### Inputs
UUID-U04 可运行新目标、UUID-U03 映射、U01 外部引用与签名清单、SDK 发布流水线、应用部署和 runtimefiles 格式。

#### Execution Requirements
- 从升级源码发布 SDK 新版本并更新活动消费者/vendor，历史发行字节保留。同步 supportedSDKVersions、门店 validContext 硬编码版本、生成 DTO、来源记录/摘要、检查脚本和消费测试，不能只复制新包就交付；按计划阻止仍发送旧活业务 v4 的消费者混入新 epoch。
- policy、env、manifest、runtimefiles、应用登记、深链接和外部 RP／集成引用使用同一类型化映射；非 UUID `client_id`、业务编号和外部第三方 ID 保原。
- 自有产物需要重建签名／哈希时由真实 owner 重新生成；旧已签证据保原附映射，不能字符串替换伪造有效签名。
- 全部旧 session／access／refresh／code／device／app token／JWT `jti` 失效；新账号以新 `users.id`／`sub` 重新认证，验证外部 RP 不把新主体错认成无关账号或自动合并。
- 跨平台、门店、工单、通知、微应用、OAuth 和新登录链验证创建／读取／关联／回调／拒绝旧 ID；版本化协议不静默改写冻结历史合同。

#### Expected Outputs
SDK 新发布与消费清单、配置／引用迁移包、签名／哈希重建记录、外部 owner 确认和端到端证据、`validation/uuid-u05.md`。

#### Acceptance Criteria
- [x] 实际加载的 SDK/vendor 与新发行匹配，新前端/Node 生成 v7，版本允许列表、生成 DTO、来源摘要和检查脚本通过；冻结历史产物哈希保持不变。
- [x] 三库与应用／通知／OAuth 全链路引用一致，policy／env／manifest／runtimefiles／深链接及 RP 主体映射无旧活引用。
- [x] 旧 token、refresh、code、device、session 无法访问／兑换，新凭据签名有效；没有通过字符串替换继续使用旧 JWT 或已签产物。
- [x] 残留 v4 命中仅为登记的历史／负例／外部证据；新的错误版本／variant 请求被拒绝或按已冻结 request ID 合同重发。

#### Risks
新 SDK 未真正被活动页面消费；外部 RP 依赖旧 `sub`；运行配置漏改；签名产物被重写但未重签；旧凭据仍有效。

#### Dependencies
UUID-U04 完成；Q4 外部 owner 与迁移窗口、Q7 SDK 发布时间和消费者升级前置在实际发布前处理。

#### Notes
证据见 `validation/uuid-u05.md` 和 `validation/uuid-u05-summary.json`。仅在本地已授权范围验收：14 个 opaque client_id 保原，一个未选用但登记外部 callback 的 seed client 保留原字段并 DISABLED，实测 invalid_client，未访问外部 RP 或冒称 owner 确认。临时 notification/publish SELF 授权经用户明确选择，仅作用于一名测试成员，正向发布／v7 深链接／Socket 事件和撤回后 live／新 token 拒绝均通过，随后恢复原 policy。旧普通 OAuth raw 不可用、旧 code/device 源空、旧 JWT 已过期的覆盖限制均按原始证据记录，不冒称全量 raw 重放。宿主登录路由真实故障已最小修复并发布独立 host-u05-r2，初始产物及 216 个历史文件保原。冻结协议不静默改版；chat 的非持久 chat-sequence 保持 opaque。

---

### Phase UUID-U06: 协调停写、最终重建导入与切流恢复

#### Status
COMPLETED

#### Goal
在明确目标环境完成最终停写快照、统一映射、新库重建导入和切流，并验证零误清空与可保数据恢复。

#### Scope
最终环境／主机／库名核对、所有写入方冻结、最终导出与映射、独立目标初始化／导入、发布／外部协同切流、观察与只读旧库留存／恢复演练。

#### Out of Scope
未明确环境的批量重建、清空生产源库、新库产生写入后直接切回旧库、借本阶段实施员工／客户或 SSO 新功能。

#### Inputs
UUID-U01～U05 全部证据、最终源快照窗口、具体独立目标库清单、已验收生成器／迁移包／映射工具、切流与恢复 SLO、外部／SDK owner 协同状态。

#### Execution Requirements
- 实际执行前解析并逐项核对服务环境、主机、数据库名和角色，确认目标为登记的新库，源库只读留存；不能把方案中的泛称当作执行目标。
- 协调冻结全部 API／CLI／worker／replay／outbox／scheduler／外部写入，记录停止边界；最终导出、使用固化规则／映射重编号，再按真实迁移初始化独立目标、导入并全量对账。
- 按新 SDK／配置／签名产物与外部 RP 窗口同时切流，清除全部旧凭据状态，登录重新认证；观察成功率、拒绝旧 ID、引用失败、后台任务和业务不变量。
- 源备份继续只读保留。仅在没有新写入和未对账库外效果时按合同恢复至受控副本；开放入口前再次提升凭证失效边界、撤销并验证全部旧 JWT/refresh/code/device，不允许原样复活旧会话。已写入时完成反向迁移/可验证重放和再对账，不能直接切旧库丢失新写。
- 停写与恢复覆盖外部 agent、安装、通知发送和在途回执；对账实际效果，以迁移 epoch/轮换任务凭据/等效栅栏拒绝旧 nonce、lease、receipt，不能从快照恢复重复任务资格。回退至旧身份基线时专项不能标完成。
- 记录最终 schema fingerprint、迁移包／映射摘要、切换 epoch、恢复结果与实际限制；完成后才解除本专项预览源／目标库后续身份迁移的前置门禁，独立身份新库不依赖此历史搬迁交接。

#### Expected Outputs
最终目标核对与停写记录、最终转换／导入／对账包、部署与外部协同切流记录、旧库只读留存及保数据恢复证据、`validation/uuid-u06.md`。

#### Acceptance Criteria
- [x] 最终库名／主机／环境核对与停写边界可追溯，源库未清空，目标由真实迁移创建并通过最终业务／引用／UUID 版本对账。
- [x] 全系统新写 UUID 为 v7，旧凭据不可用，实际 SDK／应用／RP／通知／配置均使用同一映射与新 epoch，没有后台旧写或悬空引用。
- [x] 完成切流／恢复演练，按用户2026-10-24明确接受的本轮RTO偏差验收（原目标600秒、实测909.035秒，测量仍未达标）；新写后恢复包含反向迁移/重放且无数据丢失，源备份只读；任何恢复后旧凭据与旧 agent 任务栅栏都不会复活，库外效果已对账。
- [x] 六阶段交付与限制完整可追溯，身份台账编号仍保留；本专项预览源／目标库在完成前不叠加身份迁移，独立身份新库的范围和证据单独记录，不冒领专项完成状态。

#### Risks
遗漏写入方导致快照失真；目标连接错指源库；新旧配置交错；活动消费者仍发 v4；恢复丢失新写；外部 RP 关联未同步。

#### Dependencies
UUID-U05 完成；Q1 实际环境、Q2 保留范围、Q3 最终窗口／SLO、Q4 外部 owner、Q6 schema 合同、Q7 发布窗口在实际切流前满足。

#### Notes
实际证据见validation/uuid-u06.md、uuid-u06-summary.json、uuid-u06-data.json和uuid-u06-final-audit.json。已完成六独立目标真实12／3／4迁移、新鲜A导入、T0暖副本、final真实新写、T1的53行delta、部分提交只读续跑、666,815保留行全量匹配及两代新epoch。最终九库每组54表／147UUID列／67FK，非法UUID和孤儿均0；recovery150 HTTP、两代24旧secret／14旧app请求拒绝、业务完整读取和新v7工单、253静态文件回读通过。源／final只读NOLOGIN、原v4围栏、唯一外部seed禁用、publish授权0及任务表0通过。真实窗口暴露并修复epoch搜索路径、门店动态lease探针及连续两更新版本回放；原失败和原计时保留。recovery核心读写累计909.035秒超600秒的测量保留；用户于2026-10-24明确回复“接受本轮909秒偏差，按实际结果完成U06”，据此完成本轮验收。closure见validation/uuid-u06-closure.json；该接受不放宽后续演练目标，历史审计完成投影保持原字节。T1源可读的本地逻辑恢复不等于介质丢失RPO0／PITR；过期code／app／device样本限制如实记录。

---

## 5. Current Execution Pointer

Current Phase: UUID-U06（COMPLETED）；UUID-U01～U06专项全部完成

Current Objective:
本专项六阶段全部完成。用户已明确接受本轮909.035秒RTO偏差；原600秒目标、超时与失败证据保留，不作为后续通用SLO变更。当前预览沿用已验收recovery，原v4、U05源、含新写final和A／T0／T1／delta继续保留。解除本专项UUID前置门禁，后续身份工作按独立身份台账及既有阶段编号接续；该台账与Phase2指针未修改，本次未执行后续身份阶段。

Current Restrictions:
- UUID-U01～U06已完成，用户只接受本次RTO偏差，原测量未改为达标；本专项前置门禁解除，后续身份迁移仍须按其台账核对目标库／范围，不改阶段编号或自动复活历史凭据。
- U06 最终目标必须重新施加外部 seed client 禁用、fresh epoch 和旧凭据失效围栏；本地调试 client 的 DISABLED 不等于外部 owner 已完成主体绑定，重新启用必须另行协调。
- 真正执行重建必须使用已经解析核对的独立目标数据库清单，不能静默清空现库或把“重新初始化”解释为丢弃业务数据。
- 证据只记录实际执行结果；本地预览范围与未审计的远程开发／生产环境分开记录。
- 本专项完成后，如在这些预览目标库继续身份开发，沿用身份台账原阶段编号并单独记录范围；独立身份新库先验收其空库 v7 基线。

---

## 6. Decision Log

| Date | Decision | Reason | Impact |
|---|---|---|---|
| 2026-10-24 | 完成目录全部任务开发，实际 UUID 重建仅本地预览三库 | 用户全量开发指令及环境选择“本地预览三库” | U01 开始；三个源库 qiankun_platform_preview_20261024／qiankun_stores_preview_20261024／qiankun_orders_preview_20261024 保数据留存，远程和生产排除 |
| 2026-10-24 | 系统自有运行 UUID、持久主键和请求 ID 统一采用 v7，安全随机值按用途保留 CSPRNG | 用户要求未来全面采用 UUID v7，同时 UUID 不是通用 secret | Rust／浏览器／Node／SQL／SDK 与输入校验均需处理，jitter 不使用时间前缀 |
| 2026-10-24 | 使用应用显式发号及 PG17 兼容追加迁移 | 现有锁依赖支持 v7，数据库升级不是必要前提 | 不强制 PG18，不自制 SQL 算法，历史 migration 保留 |
| 2026-10-24 | 存量保数据重编号后初始化全新独立目标库并导入 | 落实用户“已有数据也调整后重新初始化”且避免隐含丢数 | 现库只读保留，不执行 DROP／清空，需最终停写与恢复合同 |
| 2026-10-24 | 映射按 authority.entity＋旧 PK 唯一分配，跨库复用 | 同一主体分库生成会破坏平台／门店／工单关联 | 类型化处理 UUID 列／数组／JSON／文本路径，禁止全局正则替换 |
| 2026-10-24 | 可信历史时间或固定迁移时间＋CSPRNG 生成 v7 | 正确构造 v7 且不伪造版本位 | 缺时间规则需冻结，业务 created_at 自身保持原值 |
| 2026-10-24 | 本专项作为一次主体 epoch 切换，之后新 users.id／sub 稳定 | 用户要求既有 UUID 也重新编号 | 原主体映射长期留存，外部 RP 协同，优先于原“旧 users.id 不变”的假设 |
| 2026-10-24 | 所有旧 session／token／code／device／refresh 作废并重新认证 | 认证主体和 jti 重编号不能保留旧签名语义 | 不直接改 JWT，不恢复有效旧凭据，真实产物重新签发 |
| 2026-10-24 | 历史 SDK／签名证据／明确负例保原，新版本升级消费者 | 历史字节是兼容与审计依据 | 允许登记的 v4 源码命中，不宣称源码零 v4 |
| 2026-10-24 | UUID-U01～U06 先于同库身份 Phase 1，原阶段不重编号 | 控制同时迁移 UUID epoch 与身份模型的风险 | 决策时六阶段全未开始；现阶段状态以本台账状态表为准，独立身份新库范围另见主台账 |
| 2026-10-24 | U03 采用独立源／转换包和完整认证回读，LSN 仅作初末观察 | 集群 WAL 不能单独证明源业务写入；新鲜维护库准入观察与源 RR／schema／计数／序列共同约束一致性 | 六份准入证明、同 map 重跑及 271 文件回读已完成；进入 U04，目标导入与最终切流仍分别验收 |
| 2026-10-24 | 本轮仅对一名测试成员临时授予 notification/publish SELF，正向验收后正式撤回并恢复原 policy | 用户明确选择临时最小授权；既有 policy 无通知发布能力，不能用原 403 冒充正向链路 | 1 publication／1 delivery、v7／深链接／Socket 与幂等通过，原 5 项 ticket grant 保持；live／新 token 撤回后拒绝，最终 grant 剩余零 |
| 2026-10-24 | 本地未选用的外部 callback seed client 施加 DISABLED 围栏，opaque 标识及其它字段保原 | 缺少外部 owner 迁移确认；本轮不访问第三方或擅自绑定新主体 | 本地有效 authorize 参数实测 invalid_client，无 Location／外部请求；U06 继续保持围栏，外部重新启用另行协调 |
| 2026-10-24 | 立即执行本地 U06 窗口，RPO=0、每次切换／恢复 RTO≤10 分钟 | 用户本轮异步问题明确选择推荐项 | 准备完成后从真实停写点计时，原库与包保留，实际耗时和零丢失须实测；不得把目标值写成通过结果 |
| 2026-10-24 | U06 从当前 U05 v7 库新鲜导出并保持合法 v7 身份；以认证 T0／T1 行 delta 验证新写后恢复 | U05 后已有新写；旧重编号工具会拒绝 PASSED release、v3 metadata 和非空幂等 receipt，不能删数据迎合旧合同 | 独立 U06 数据路径保原业务行／历史证据，原 U01～U04 合同不放宽；冻结源可读的本地逻辑恢复范围明确，不冒称介质丢失 RPO0/PITR |
| 2026-10-24 | U06实际恢复与技术审计通过，但RTO909.035秒超600秒，保留BLOCKED等待明确决定 | 用户原SLO未变；不以修补后子步骤成功重置真实窗口或覆盖失败 | 当前recovery可用，原库／包保留，预览库身份迁移门禁不解除；独立身份台账不修改 |
| 2026-10-24 | 用户明确接受本轮909秒RTO偏差，按实际结果完成U06 | 用户回复“接受本轮909秒偏差，按实际结果完成U06” | U06 COMPLETED、Q3关闭、本专项前置门禁解除；原600秒目标和909.035秒未达标测量、失败及计时保留，后续SLO不变，身份主台账未改 |

---

## 7. Open Questions

| ID | Question | Impact | Owner | Status |
|---|---|---|---|---|
| Q1 | 本次覆盖本地 WSL Ubuntu PostgreSQL 18.6 的三套 preview 源库及相应新目标；运行产物／配置引用随之核对 | 用户选择本地预览三库，远程开发及生产明确排除；U04 再登记不存在的独立新目标名 | User／执行代理 | COMPLETED |
| Q2 | 按已审查表策略保留业务／账号／审计及计数器，排除活动旧凭据和由真实迁移重建的 metadata | U03 已真实执行：119 凭据行、21 metadata 行不进入目标包，原始源证据和逐行 proof 保留；其余保留行完成 owner 投影 | User／业务负责人 | COMPLETED |
| Q3 | 本地窗口RPO=0、恢复核心读写909.035秒超600秒；用户已明确接受本轮偏差并要求完成U06 | 本轮按用户例外验收关闭，原SLO和失败记录保留；未来目标未放宽 | User／运维负责人 | COMPLETED（用户接受本轮RTO偏差） |
| Q4 | 本地 RP／配置引用与外部 callback 的处理边界 | 本地 7 应用／3 OAuth 流程和主体一致性已验收；14 opaque client_id 保原，1 外部 callback seed client 已 DISABLED 且真实拒绝。外部 owner 未确认、账号绑定未测试，本地未触达外部；U06 保持禁用，重新启用另行协调 | User／集成负责人 | COMPLETED（本地范围；外部禁用） |
| Q5 | 可信 created_at 使用毫秒时间；缺失、负值或晚于冻结点统一采用 source frozen_ms，已有合法 v7 保留原值 | U03 已将时间理由／来源写入映射；同源恢复映射精确比对并真实复跑一致，原业务时间不改 | User／数据负责人 | COMPLETED |
| Q6 | schema fingerprint、不可变／append-only 触发器和受控导入的权威 owner／交接合同是什么？ | U04 已完成真实迁移、受控导入、触发器／FK／序列／完整 raw schema 与角色围栏复核及失败目标恢复；本代摘要重新审查，未放宽生产校验 | User／数据库与工程负责人 | COMPLETED |
| Q7 | 本地 SDK 与消费者发布、request ID 合同及冻结历史字节 | SDK 3、host-u05-r2 与 7 微应用已实际发布／浏览器消费；Node 2,000 请求 ID 全 v7，253 静态文件回读通过，216 历史文件保原；更广环境发布时间不在本轮范围 | User／SDK 与应用负责人 | COMPLETED（本地范围） |

问题仅在影响其执行阶段时处理，不要求用户现在为方案追加授权或一次性回答全部。静态源码审查事实、只读预览范围和未来真实环境审计必须各自明确。

---

## 8. Change Log

| Version | Date | Change |
|---|---|---|
| v0.8 | 2026-10-24 | 用户明确接受本轮909秒RTO偏差并要求完成U06；全部四项验收完成（RTO按本轮用户例外接受，原测量仍未达标），U01～U06均COMPLETED、Q3关闭。新增closure决策绑定原证据SHA，保留历史审计原字节／失败／计时及600秒目标，身份主台账不改 |
| v0.7 | 2026-10-24 | U06实际执行六目标正式迁移、666,765保留行A导入、T0后真实新写、53行delta和666,815行T1精确恢复；修复raw指纹搜索路径／动态lease探针／版本3→5部分提交续跑，29项真实回归与最终九库审计通过，recovery150HTTP／业务新写／253静态字节通过。原窗口909.035秒超600秒，U06 BLOCKED等待SLO决定，身份主台账不动 |
| v0.6 | 2026-10-24 | U05 完成：冻结 12／3／4 正常服务 imported 启动，真实 policy 转换与 epoch／12 密钥轮换，SDK 3／七 release／253 静态字节和 216 历史文件保留；修复真实宿主登录路由并发布 r2，七应用真实浏览器 66 项通过；凭据 116 HTTP、OAuth BFF 17 HTTP、业务／通知正向及临时 SELF 授权撤回／原 policy 恢复通过；最终 147 UUID 列／3,058 值、67 FK、完整 schema／源和角色围栏复核通过。指针 U06（NOT_STARTED），身份主台账不修改 |
| v0.5 | 2026-10-24 | 接续核验六旧代理与 import-05 真实失败，修复层级触发器运行路径；U04 固定目标实际重建、真实 12／3／4 迁移、逐代 raw schema 双路审查、666,610 行认证导入、147 UUID 列／67 FK／序列独立复核及重复拒绝通过。U04 COMPLETED，指针进入 U05（NOT_STARTED）；身份工作交由另一用户聊天独占，不修改其主台账 |
| v0.4 | 2026-10-24 | UUID-U03 完成：三库真实冻结／导出、562 映射、666,750→666,610 行的同 map 两次转换和 271 文件认证回读；保留各失败轮次、三种实际二进制及原源 v4 留存事实。指针进入 UUID-U04；Q2／Q5 完成，Q3／Q6 按实际边界推进，身份主台账未修改 |
| v0.3 | 2026-10-24 | 澄清适用范围与证据说明：本预览库保数据专项继续独立执行，同库身份迁移仍等待 epoch 切换完成；无历史数据的独立身份新库改按其空库 v7 基线进入 Phase 1，已完成阶段证据按实际状态引用。本次未修改六阶段状态、实际操作范围或导出／恢复条件 |
| v0.2 | 2026-10-24 | UUID-U02 完成：147 列 PG17 约束和全生成/输入/INSERT/seed 适配，各仓验证完成；平台136 DB用例首次134+2定点修正通过如实记录；推进 UUID-U03 |
| v0.2 | 2026-10-24 | UUID-U01 完成：三库只读 CLI 实跑、147 列／53 表／67 有序 FK 覆盖、生成／引用注册表及导出合同；指针推进 UUID-U02 |
| v0.2 | 2026-10-24 | 接收全量开发授权和本地预览环境选择，UUID-U01 IN_PROGRESS，新增只读工具与机器可读合同开发范围 |
| v0.1 | 2026-10-24 | 创建 UUID v7 专项六阶段台账；仅方案，UUID-U01～U06 全部 NOT_STARTED，指针 UUID-U01，原身份 13 阶段不变 |
