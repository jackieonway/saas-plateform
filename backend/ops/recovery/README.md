# 三库备份与隔离恢复指南

saas-recovery 当前支持 pilot 协议：平台、工单、门店三库共同停写，数据库备份与指定版本材料组成认证加密集合，上传远端再取回校验；恢复只允许新的隔离目标。production-pitr 尚未实现，不能将此工具视为通用任意数据库备份服务。

## 1. 工具和私有配置

在本 recovery 目录执行 cargo build --release --locked、cargo test --locked。实际执行环境为 Linux，需 PostgreSQL 17 的 pg_dump、pg_restore、psql 和 findmnt，数据库与 HTTPS 存储可达。

按 [config.example.json](config.example.json) 创建源码之外的私有配置，绝对路径传给 CLI。主要字段：

| 字段 | 配置内容 |
| --- | --- |
| profile | 当前只能使用 pilot |
| service_file、admin_service | PostgreSQL service 连接文件及恢复管理员连接 |
| databases | 固定 platform、work_orders、stores 三项源 service、目标库、owner |
| key_file | 独立托管的 32 字节密钥 |
| scratch | 私有可验证 tmpfs，明文 dump 不落普通磁盘 |
| artifact_root | 本次冻结的完整版本材料树 |
| local_bundle、remote_url | 本地密文与远端 HTTPS 对象位置 |
| remote_token_file、ca_file | 对象存储 Bearer 与信任证书 |
| monitor_url、monitor_token_file | 监控事件入口与专用凭据 |
| sequence_file | 持久递增事件序号及同目录索引 |
| restored_artifacts | 新的恢复材料目标，不得覆盖已有目录 |

凭据文件权限、所有者、符号链接、tmpfs 与锁都受检查。集合规模上限 512 MiB、文件最多 20000，不适合直接承诺大型生产库的 RPO/RTO。密钥与备份密文分开托管，备份中不放实际运行密钥或 TLS 私钥。

## 2. 准备版本材料

当前校验器要求以下固定材料布局，各前端目录至少包含 index.html：

```text
host/                          宿主完整静态产物及保留资源
stores/v1/、stores/v2/          门店两份完整前端
work-orders/v1/、work-orders/v2/ 工单两份完整前端
images/lock.json                四个业务静态镜像的身份及归档摘要
images/*.tar                   对应四个镜像归档
routes/nginx.conf               实际网关入口配置
```

此外，应把数据库实际引用的不可变发布版本、ZIP 材料、完整静态依赖和其他运行材料放入 artifact_root，一并冻结和保存。工具会校验已收集文件的路径与摘要，但尚未逐项核对数据库发布引用或 HTML 资源引用是否齐全；runtime/releases、runtime/bin、runtime/native-lock.json 并非当前校验器的必填项。备份成功不代表这些依赖已完整，应通过隔离恢复中的版本页面与旧资源验收确认。

材料路径按实际文件名逐组件保存，拒绝不能准确还原的非 UTF-8 名称和字面反斜线，不通过替换字符或分隔符悄悄改名。认证包校验还会拒绝 NUL、重复路径和父文件/子路径冲突，在创建恢复数据库或写出材料之前报告这些确定失败的输入。

images/lock.json 记录四项 app、version、imageId、archiveSha256、archiveBytes。Docker imageId 与 registry manifest digest 是不同身份，不能互换。所有实际引用和旧资源都要完整保存。

## 3. 协调备份

1. 进入维护状态，停止所有平台、业务、通知、发布任务及外部写入者，排空事务，冻结版本材料。
2. 明确确认停写后，设置 BACKUP_WRITES_QUIESCED=true。这个标志是操作者断言，工具不能发现所有外部写入者。
3. 执行新集合备份：

```sh
BACKUP_WRITES_QUIESCED=true saas-recovery backup-new /srv/saas/private/recovery.json
```

工具执行三库 custom dump，记录迁移/材料指纹，以 AES-256-GCM 认证加密。远端须支持 Bearer、PUT 配合 If-None-Match:* 返回 201，以及 GET 返回 200；已有对象不得覆盖。上传后重新下载、检查摘要并认证，通过后才更新本地最新集合索引。

4. 无论成功或失败，都核对维护前活动服务和写入者，明确恢复运行并记录停写时间。上传失败对象不自动删除。

## 4. 校验和年龄

verify、age 均从远端取回并认证；可传原配置或实际生成的最新集合配置，例如 sequence_file 同目录的 events.latest.json：

```sh
saas-recovery verify /var/lib/saas-recovery/events.latest.json
saas-recovery age /var/lib/saas-recovery/events.latest.json
```

实际索引名取决于配置的 sequence_file。age 超过 86400 秒失败，不能用尚未远端校验的本地 dump 时间计算恢复点。维护周期、重试和告警阈值应留出裕量。

## 5. 恢复到隔离环境

准备新的私有恢复配置：三目标不同、以 recovery_ 开头、均不存在且不等于源库；restored_artifacts 也必须是新路径。恢复管理员应具备受控建库及所需权限，目标不接业务流量或旧定时任务。

```sh
saas-recovery restore /srv/saas/private/restore.json
```

工具认证全集合、检查来源和目标，再建库、逐库恢复并比较迁移和业务 catalog。只对固定已知约束做受限兼容处理，未知表达式失败，不改历史校验值绕过验证。

平台恢复后使旧 session、refresh、应用/普通 OAuth 凭据、授权码和设备流程失效，并记录审计。成功仅返回 QUARANTINED；跨库失败不保证全局回滚，失败目标继续隔离，工具不自动 DROP。

加载并核对归档镜像与材料，在隔离 HTTPS 下重新登录，验证旧凭据被拒、三库记录、业务授权/写入、通知、两个业务的两版本页面和旧资源。全部验收后再明确切流，记录从恢复请求到业务可用的完整 RTO。

## 6. 调度与边界

备份、校验和年龄命令通过固定事件上报监控，监控失败也可能使命令失败。序号与锁保持统一所有者，不重置规避重放检查。当前没有自动安装的定时器或外部告警发送器。

PITR 需要持续 WAL、基础备份、timeline 与共同恢复目标等另一套协议，当前未实现。先完成当前部署格式的真实演练，再设定可证明的恢复目标。
