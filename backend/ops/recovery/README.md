# 三库协调备份与隔离恢复

`saas-recovery` 当前实现 `pilot` 协议：平台、工单、门店三库共同停写，将数据库与冻结版本材料组成认证加密集合，上传远端后重新取回校验；恢复只面向新的隔离目标。`production-pitr` 未实现。该工具仍有固定试点材料约束，尚未成为任意 qiankun 应用的通用恢复系统。

## 1. 工具和私有配置

在本目录执行：

```sh
cargo test --locked
cargo build --release --locked
```

实际执行环境需要 Linux、PostgreSQL 17 的 `pg_dump`、`pg_restore`、`psql` 和 `findmnt`，数据库与 HTTPS 对象存储可达。按 [config.example.json](config.example.json) 在源码外创建私有 JSON，使用绝对路径交给 CLI。

| 字段 | 内容 |
| --- | --- |
| `profile` | 当前仅 `pilot` |
| `service_file`、`admin_service` | PostgreSQL service 连接文件及恢复管理员 service |
| `databases` | 固定 platform/work_orders/stores 三项源 service、目标库和 owner |
| `key_file` | 独立托管的原始 32 字节密钥 |
| `scratch` | 可验证的私有 tmpfs，明文 dump 不落普通磁盘 |
| `artifact_root` | 本次冻结的完整版本材料目录 |
| `local_bundle`、`remote_url` | 本地密文和远端 HTTPS 对象位置 |
| `remote_token_file`、`ca_file` | 对象存储 Bearer 和 CA |
| `monitor_url`、`monitor_token_file` | 私有监控事件入口及独立凭据 |
| `sequence_file` | 持久递增事件序号，与本次索引和锁同属受控目录 |
| `restored_artifacts` | 新恢复材料路径，不覆盖已有目录 |
| `timeout_seconds` | 当前配置的整体工作期限 |

凭据权限、所有者、符号链接、tmpfs 和锁受检查。认证集合上限 512 MiB、文件最多 20000，不能据此承诺大型生产库恢复目标。密钥和备份密文分开托管，运行密钥和 TLS 私钥不放版本材料。

## 2. 冻结真实发布材料

当前校验器强制以下布局，四个业务前端目录至少含 `index.html`：

```text
host/                          宿主完整产物与保留资源
stores/v1/                     门店试点版本一
stores/v2/                     门店试点版本二
work-orders/v1/                工单试点版本一
work-orders/v2/                工单试点版本二
images/lock.json               四份镜像的身份与归档摘要
images/stores-v1.tar
images/stores-v2.tar
images/work-orders-v1.tar
images/work-orders-v2.tar
routes/nginx.conf              实际入口配置
```

`images/lock.json` 是四项数组，每项有 `app`、`version`、`imageId`、`archiveSha256`、`archiveBytes`；app 固定工单或门店，version 固定 `v1`/`v2`。`imageId` 是 Docker 本地镜像身份，与 registry manifest digest 不同，不能互换。

这一布局是当前 pilot 校验协议，不能因前端已改 qiankun 而省略。它也不自动覆盖当前数据库实际引用的所有版本。还应把实际环境 URL 对应的完整静态材料、不可变发布目录、ZIP 内容、所需镜像、SDK、网关路由和可重建配置收集到 `artifact_root`，一并冻结。

工具校验已收集文件的路径、摘要和认证包，不逐项核对数据库发布引用、外部 URL 内容或 HTML 资源依赖。`runtime/releases`、`runtime/bin`、`runtime/native-lock.json` 不是当前必填布局。不要用补占位 HTML 或只满足固定目录结构代替完整材料验收。

相对文件名按实际路径组件保存，拒绝非 UTF-8 名称、字面反斜线、NUL、重复路径及父文件/子路径冲突。确定无效的包在创建恢复数据库或写出材料前被拒绝。

## 3. 协调备份

1. 进入维护状态，停止平台、业务、通知、发布任务及全部外部写入者，排空事务并冻结版本材料。
2. 记录共同停写点。`BACKUP_WRITES_QUIESCED=true` 是操作者的明确断言，工具不能发现所有写入者。
3. 运行新集合备份：

```sh
BACKUP_WRITES_QUIESCED=true saas-recovery backup-new /srv/saas/private/recovery.json
```

工具执行三库 custom dump，记录迁移与材料指纹，用 AES-256-GCM 认证加密。`backup-new` 为本地路径及远端对象增加唯一后缀，成功后写出 `sequence_file` 同目录的 latest 配置索引；`backup` 使用原配置位置且不覆盖已有集合。

远端存储需支持 Bearer、`PUT` 携带 `If-None-Match: *` 返回 201，以及 `GET` 返回 200。上传后重新下载、检查摘要和认证，通过后才更新最新索引。失败对象不自动删除。

4. 无论成功或失败，核对维护前写入者和活动服务，按明确流程恢复运行，记录停写时间。命令失败先查原结果，不能立即覆盖已有远端对象。

## 4. 远端校验与恢复点年龄

`verify`、`age` 都从远端取回并认证。使用原配置或实际生成的最新索引，例如当 sequence 文件为 `events.sequence` 时：

```sh
saas-recovery verify /var/lib/saas-recovery/events.latest.json
saas-recovery age /var/lib/saas-recovery/events.latest.json
```

`age` 超过 86400 秒失败，恢复点以已远端取回认证的集合为依据，不能用本地 dump 时间替代。备份周期、重试和告警阈值需给验证留下余量。

备份操作成功后，监控事件投递失败也会使命令失败。核对集合、索引、远端对象和事件状态，区分数据操作成功与告警链路失败。不要重置 sequence 绕过重放保护。

## 5. 恢复到新隔离目标

创建新的私有恢复配置：三目标名称各不相同、以 `recovery_` 开头、均不存在且不等于源库；`restored_artifacts` 也必须是新路径。恢复管理员具备受控建库和恢复权限，目标不接业务流量或旧定时任务。

```sh
saas-recovery restore /srv/saas/private/restore.json
```

工具先认证全集合并检查来源和目标，再建库、逐库恢复，比较迁移与业务 catalog。只对固定已知约束作受限兼容处理，未知表达式失败。跨库失败不保证全局回滚，失败目标继续隔离，工具不自动 DROP。

平台恢复会使旧 session、refresh、应用/普通 OAuth 凭据、授权码和设备流程失效并记录审计。成功状态仅为 `QUARANTINED`，尚未完成切流。

## 6. 隔离验收和切流

加载并核对镜像归档、网关配置和静态版本，在隔离 HTTPS 下重新登录，验证：

- 旧凭据拒绝、新登录可用，三库业务记录和迁移一致。
- 当前直接成员、组织切换、功能权限、数据范围和业务写入符合预期。
- 通知、工单提醒以及外部调用使用隔离环境配置。
- 宿主 CSP 满足 qiankun，应用加载、卸载、重进和真实 JS/CSS 正常。
- 固定两版本及数据库实际引用的其他版本、旧资源均可访问，缺资源真实 404。

全部验收后再明确切流，记录从恢复请求到业务可用的完整 RTO。尚未建立持续 WAL、基础备份、timeline 和共同恢复目标，因此不能将当前 dump 流程作为 PITR 证明。调度器和外部告警由运维单独建立，见[监控指南](../monitoring/README.md)。
