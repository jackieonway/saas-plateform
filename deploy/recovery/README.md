# 三库协调备份与隔离恢复

`saas-recovery` 是独立 Rust 离线 CLI，调用 PostgreSQL 17 的 `pg_dump`、`pg_restore`、`psql` 和 Linux `findmnt`。没有生产 Node 服务，也没有数据库 down migration。平台、工单和门店必须作为同一集合恢复。构建与门禁：

```sh
cargo build --locked --manifest-path deploy/recovery/Cargo.toml
cargo test --locked --manifest-path deploy/recovery/Cargo.toml
cargo clippy --locked --manifest-path deploy/recovery/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path deploy/recovery/Cargo.toml -- --check
cargo doc --locked --manifest-path deploy/recovery/Cargo.toml --no-deps
```

新依赖仅复用仓库已有系列：serde/JSON 用于有限集合格式；ring 用于系统随机数、SHA256、AES256-GCM；base64 用于内存集合中的二进制字段；reqwest/rustls 用于带认证、私有 CA、拒绝跳转的 HTTPS 对象存储和运维事件。配置、凭据和本地运行文件不进入静态镜像。

## Profile 与部署前提

| Profile | 候选 RPO | 候选完整 RTO | 当前实现 |
| --- | --- | --- | --- |
| pilot | 24h | 4h | 协调停写 custom dump + 完整恢复演练 |
| production-pitr | 15min | 1h | 未实现、未验收，CLI 拒绝该 profile |

试点工具在隔离容器故障域中验证远端取回。容器隔离不证明真实异机、异地故障独立性；正式物理远端、密钥托管、链路和运维响应须在目标主机重新演练。本地报告不构成生产 SLA。

操作者按 `config.example.json` 创建自己的绝对路径配置，配置/pg service 文件/32 字节随机二进制密钥/两个独立 token 文件均须属于当前用户且 mode0600，父目录0700。示例名全部为占位。PG service 分别指定三个源库和维护库的连接，密码仅在私有文件中；目标 owner 由 DBA 预建最小权限角色。维护连接只供离线建新库，不给业务服务。生产数据库连接应通过 pg service 的 `sslmode=verify-full` 和自己的 `sslrootcert` 正常验证。

配置文件、其所有父目录和可执行工具路径必须由可信运维控制，不允许其他用户替换。运行环境清理非必要 PG 变量，PATH 使用批准的固定目录。工具对配置/集合拒绝链接，任意远端 URL/命令不能来自用户业务请求。单操作锁保存在 sequence 文件旁；并发运行拒绝。异常断电留下锁时，先确认所有对应进程停止，再由操作者处理锁；工具不会擅自接管。

使用专用 tmpfs 作为 scratch，预估 dump 容量并禁用 swap 或使用批准的加密 swap。tmpfs 不可用或空间不足即失败，禁止降级磁盘明文。普通数据库 stderr 留在0700暂存目录，正常或可捕获错误退出时删除；SIGKILL 后手动核查私有 tmpfs。内存集合上限512MiB（包括 base64 后的 JSON），单集合最多20000文件；这限制了试点规模。更大库需后续经过验收的分块流式格式或生产 PITR，不能未经测试提高上限。

## 准备完整版本材料

`artifact_root` 是本轮停写后固定的材料树，必须包含：

```text
host/                         # 实际宿主完整 dist，包括保留的旧 hash
stores/v1/ stores/v2/          # 两份不可变完整前端构建
work-orders/v1/ work-orders/v2/
images/lock.json               # 四个镜像身份与 archive SHA256/bytes
images/stores-v1.tar ...       # 四个 docker save 归档
routes/nginx.conf              # 网关、额外 host/micro 配置也放 routes/
runtime/releases/             # 平台引用的全部不可变版本与 ZIP 正式目录
runtime/bin/                  # 原生部署时的固定 Rust 可执行文件及动态库
runtime/native-lock.json       # 原生文件身份、构建来源和转换记录
```

镜像锁为四项数组，每项含 `app`（stores/work-orders）、`version`（v1/v2）、`imageId`（sha256:配置digest）、`archiveSha256`、`archiveBytes`。imageId 是本地镜像 config 身份；registry manifest digest 应另存部署材料，二者不能混称。试点从真实构建导出四个镜像并校验归档；部署时还应对恢复出的 archive 执行 `docker load`、检查对应身份和静态/后端就绪，再切流。不得从当前浮动标签替代历史镜像。

运行密钥、TLS 私钥和数据库角色密码由独立托管链恢复，不放这棵公开版本材料树。工具对全部文件做加密及哈希认证，但不能替运维发现遗漏的外部配置或所有发布引用。先用 Phase31 库存与实际 active/previous/环境引用核对材料，保留全部仍被引用的版本；无删减备份和保留回收动作。

## 备份与远端校验

1. 先在网关进入维护，停止平台、工单、门店、通知、发布/清理/同步任务以及任何外部写入者，排空未完成事务，保持静态材料不变。三个库没有天然跨库原子快照；这个共同停写窗口就是恢复点协调协议。
2. 单独计时停写/排空。确认后设置 `BACKUP_WRITES_QUIESCED=true`；该标志是运维断言，工具不能发现所有直连 DBA/外部写入者。
3. `saas-recovery backup-new /absolute/private/config.json` 每次生成随机唯一远端对象和本地密文名。直接 `backup` 使用精确配置名称，适合一次演练；已有名称拒绝覆盖。
4. 本地 custom dumps 仅在 tmpfs 暂存。集合含恢复点、三库版本、SQLx 迁移 checksum、dump 和完整版本文件。集合整体 AES256-GCM 认证，随机96位 nonce；密文上传后重新通过 HTTPS 下载、比较 SHA256 并解密认证，成功后才记录可用。
5. HTTPS 存储须支持 Bearer、`PUT` + `If-None-Match:*` 返回201以及 `GET` 返回200；PUT 冲突不得覆盖。存储端须自行实现持久化、原子发布、不可变策略和异机耐久性。CLI 不提供存储服务。
6. `backup-new` 仅在远端取回验证成功后原子更新 `sequence_file` 同目录的 `events.latest.json`（basename按sequence文件而定）。这是本地选择索引，所有 verify/age/restore 仍从远端取回；本地索引损失时依据远端对象目录找回精确集合和另存的恢复配置。失败不更新索引、不删除已发密文或失败目标。上传后验证失败应隔离该对象并调查，不自动删除。
7. 恢复备份前活动的服务，记录停写总时长。`backup-window.sh` 为可审阅 systemd 包装示例，必须补齐目标环境所有写入者及维护/排空流程，不能盲装。备份失败仍尝试恢复原活动服务，重启失败返回非零。

```sh
saas-recovery verify /var/lib/saas-recovery/events.latest.json
saas-recovery age /var/lib/saas-recovery/events.latest.json
```

age 取回并认证选择的远端集合，超过86400秒产生失败事件；缺失、错密钥和损坏也失败。RPO 按故障时最近**可取得且已验证**的异机恢复点计算；若仅选到较旧集合，应按该集合保守报告，不能用尚未复制的本地 dump 时间。每天成功一次也可能临近/超过24h，必须监控年龄并给上传/失败重试留裕量。production15min不能靠每日 dump 证明。

## 隔离恢复与开放流量

1. 从独立托管处取得密钥、PG角色/TLS/运行配置，RTO 从故障/恢复请求开始，包括取密钥、取副本、建库、服务就绪、重新登录和全部业务验证。密钥与密文不得同一故障域保存。
2. 创建新的0600恢复配置：三目标各不相同、名称以 `recovery_` 开头、不等于任意源库且目标不存在；设置从未存在的 `restored_artifacts`。网关保持隔离，不让定时/旧副本连接目标。源端连接为只读标识检查；源临时不可达时先由 DBA 建立可验证的源身份连接/恢复配置，工具不会跳过保护。
3. `saas-recovery restore /absolute/private/restore.json` 总集合认证、必需产物/镜像锁和三目标检查通过后才建库。template0创建、无clean/no-owner/no-ACL、指定目标owner、每库单事务恢复、比对SQLx迁移指纹。
4. 平台库执行 `invalidate.sql`：所有用户 session_version增加、平台session撤销、psr消费、所有用户/机器/应用OAuth token撤销、code/consent消费、device过期，恢复审计同事务。保持失败目标隔离；工具从不自动DROP，跨库失败不能伪称全局回滚。
5. CLI 成功只返回 `QUARANTINED`。恢复原生文件/动态库或从归档加载固定镜像，核对原始配置引用、权限和密钥；更换JWT签名密钥作为额外防御。签名密钥轮换不能替代第4步。
6. 在隔离HTTPS启动平台/两个业务/通知，实际重新登录，拒绝旧session、refresh、应用Bearer、普通OAuth access/refresh/code/device和pending consent；核对恢复审计。验证已有记录、普通授权/跨组织拒绝、增改删、通知发布/收件箱/已读和两个应用的两版本页面及旧资源。全部通过后才由运维切流。
7. 记录完整RTO、远端RPO、停写、dump/upload、取密钥、restore、ready、业务/浏览器各段，另记录失败和重试；明确当前采纳的profile。恢复配置会携带敏感连接文件路径，报告只保留摘要和脱敏结果。

## 告警、定时和生产 PITR 后续验收

CLI 给 Phase32 Rust monitor 的 `/internal/events` 发送固定 `backup_run`、`backup_verify`、`backup_age`，分别带 healthy 和本地持久递增 sequence。监控按自身连续样本阈值输出 DOWN/RECOVERED；阈值2时一次失败不会立即转态，生产值需按备份周期配置。成功的初始健康不发事件。监控不可达/拒绝会令命令失败；CLI 错误、journal/stdout也需运维收集。没有外部告警发送器或送达承诺。sequence 文件/锁须由备份与校验操作者共同拥有，不可重置以规避监控重放保护。

service/timer 仅为模板，未安装或启用。安装前核对执行用户、服务全量清单、维护/排空、独立密钥注入、tmpfs容量、路径权限、对象存储、最新索引权限和监控阈值；安排 OnFailure 到**已授权**的运维处理器。两个模板默认由同一运维用户root执行以保持0600索引/sequence所有者一致；如改为专用用户，须单独配置停止服务的最小授权及统一所有权。外部告警和生产定时安装须目标环境明确授权。

生产 PITR 设计必须有：三库所属PG cluster的持续WAL归档、独立加密对象存储与保留目录、定期基础备份、timeline/history可取得性；多cluster须共同停写标记恢复点或业务对账，再各自按同一恢复目标还原。验证断档/迟到WAL、错误timeline、归档加密/校验、全量+增量重放、角色/产物恢复点协调，监控archive延迟。故障时选择最新远端可用点，实际完成旧凭据失效与全业务验收，才能声称15min/1h。Q11中真实目标、密钥/接收人和停写政策仍由产品/运维提供。

## PostgreSQL17约束表达式的逻辑恢复兼容

实际dump/restore发现：varchar IN列表的数组整体转text[]，可能在重解析后变成每元素转text；数据语义等价，但旧严格catalog指纹会拒绝两业务启动。恢复工具只在受控新库中处理五个已知IN约束，分别在临时表解析原始DDL及其deparse/reparse形式；目标必须精确匹配其中一种，才按原始DDL重建并重新验证数据。未知表达式立即失败。随后完整比较加密清单记录的源业务catalog（列/约束/index/function/trigger/relation/type），门店原存储fingerprint也必须相等。没有刷新fingerprint、修改历史迁移、关闭runtime验证或接受任意SQL。

pg_restore本身仍单事务；受限约束重建为后续独立事务，失败目标继续隔离，不声称整个多库流程原子回滚。原生回归tests/phase33-native-restore.mjs要求受管Linux/PG17与显式STORES_BINARY/WORK_ORDERS_BINARY，实际dump/restore并重新启动原Rust服务，同时测试未知约束拒绝和全源catalog保持；它与完整授权/CRUD/通知/浏览器验收分别执行。
