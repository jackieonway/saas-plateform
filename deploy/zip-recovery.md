# 同机 ZIP 产物对账与暂存恢复

本页适用于同一 Linux 主机上，Rust 平台读写 `MICRO_APPS_DIR`、独立 Nginx 只读同一持久目录的 ZIP/tar.gz 兼容部署。基础挂载、HTTPS 与入口配置见 [部署说明](README.md) 和 [ZIP override](compose.zip.yml)。纯镜像部署不需要此目录。本页不提供跨机分发、远程容器管理或已发布版本的自动删除。

以下是接口和操作流程，不是目标环境的验收记录。实际执行结果、源码摘要、工具版本及未验收项由 Phase 5 验证记录保存；不能把命令模板或预期响应当作 PASS。

## 存储布局与权限

`MICRO_APPS_DIR` 必须由操作者预先创建，为规范的 UTF-8 绝对目录，路径及各祖先不能含 symlink、junction 或其他重解析点。后端使用 UID/GID `10001:10001`，需要读写该目录；静态 Nginx 使用 UID/GID `101:101`，只读正式产物。按实际用户、组或 ACL 配置权限，不把整个存储目录改为所有人可写。

```text
MICRO_APPS_DIR/
├── .staging/                         # 0700，只有发布进程身份可访问
│   ├── .maintenance.lock             # 永久维护锁，普通文件
│   └── <UUID-v4>/                    # 0700，每次写入独占的新目录
│       ├── owner.json
│       └── payload/                  # 尚未发布的完整或部分产物
└── <application-code>/versions/<version>/
    ├── index.html
    ├── assets/...
    └── .artifact-manifest.json       # 服务端生成的隐藏清单
```

正式 payload 目录按 `0755`、文件按 `0644` 写入，以便独立静态用户读取。私有 `.staging` 父目录保持 `0700`；不能为解决权限问题将其开放给 Nginx。入口只允许 `/sub-apps/<application-code>/versions/<version>/...`，点文件、暂存路径和未知版本应返回 404；隐藏清单不作为公开 HTTP 下载入口。

暂存与正式目标必须能执行同文件系统的无覆盖原子安装。Linux 使用 `renameat2(RENAME_NOREPLACE)` 语义，不支持时失败，不回退为边复制边公开，也不覆盖已有目录。`payload` 的文件、清单和目录同步完毕后才进入正式路径；正式目录一旦安装，不因后续同步、数据库或响应失败而删除。

每个上传写入持有根目录维护锁的共享锁，直到该次随机暂存目录清理结束。维护命令取得非阻塞独占锁，存在写入者或其他维护者时直接拒绝。锁文件长期存在是正常状态；**不要手工删除或替换 `.maintenance.lock` 来“解锁”**。进程退出释放内核锁，文件是否存在不能判断上传是否仍在运行。

## 清单和产物身份

隐藏清单名固定为 `.artifact-manifest.json`，由服务端生成。归档不能自行携带该名称的文件或大小写变体。Schema 1 使用以下字段：

| 字段 | 含义 |
| --- | --- |
| `schemaVersion` | 整数 `1` |
| `entryType` / `entry` | 清单入口类型及规范入口；当前上传 API 只接受 `HTML` + `IFRAME` 应用，入口为 `index.html` |
| `files` | 规范相对路径按原始 UTF-8 字符串顺序排序；每项字段依次为 `path`、`size`、`sha256` |
| `artifactSha256` | 规范清单摘要，小写 64 位十六进制 SHA-256 |

`artifactSha256` 对紧凑 UTF-8 JSON 字节计算 SHA-256，字段顺序固定为 `schemaVersion`、`entryType`、`entry`、`files`；不包含 `artifactSha256` 本身。每个文件摘要对解压后的实际字节计算，`size` 为字节数。压缩格式、时间戳、压缩级别、归档项顺序以及被规范移除的单层共同外包装目录不参与身份。**压缩包文件自身的 SHA-256 不能代替产物摘要。** 使用服务端返回的清单和摘要，不手工重写清单来消除冲突。

解包仍保留容量和路径限制：压缩包不超过 20 MiB，单文件不超过 4 MiB，实际文件内容总字节不超过 16 MiB，最多 49 个文件。tar.gz 的完整解压流另有 17 MiB 上限，包含文件内容、元数据、头和填充；不能用大量归档元数据绕过文件内容预算。不接受空文件、穿越路径、链接、特殊文件、敏感配置、重复或大小写别名路径。一个版本只有一个 HTML 入口，根 `release.json` 为保留名称。文件集合、路径、大小或字节变化均应使用新版本号。

旧正式目录没有隐藏清单时，只有其全部实际文件与本次包完全一致，才允许采用并原子补写隐藏清单；额外文件不会被忽略。旧目录中的 `release.json` 也不能当作可信清单跳过检查。旧数据库发布记录的摘要为空时，还必须核对该应用版本在**所有已有环境**的入口类型、入口 URL、健康地址和规范资源定义；资源按 URL、kind 排序比较。通过后只给本次目标环境的匹配记录补绑定，不批量改写其他环境的旧记录。定义不一致返回冲突，不能通过手改目录或数据库强行采用。

## 先查询，再决定重试

存储对账接口为：

```http
GET /api/applications/{application-id}/releases/storage?version={version}
```

调用者必须同时具备 `system:application` 权限和平台管理上下文。此接口只核对和报告，不修复目录、不登记版本、不切换部署、不删除文件。查询没有 `environment` 筛选，返回所有环境的引用。

| `status` | 实际含义 | 下一步 |
| --- | --- | --- |
| `REGISTERED` | 正式目录及清单一致，且至少一个环境的发布记录绑定该摘要 | 检查 `references` 是否包含本次目标环境；再查该环境版本列表和部署，不把“已登记”当作“已激活” |
| `INSTALLED_UNREGISTERED` | 完整产物已安装，但没有发布记录绑定其摘要 | 保留正式目录，使用原版本和同一产物重试上传以完成登记、预检 |
| `MISSING` | 正式物理目录不存在；即使数据库已有引用也可能返回此状态 | 有引用时，先找到与数据库摘要一致的原始构建包或已验证副本；只恢复同一产物，不用新字节占用旧版本 |
| `INCONSISTENT` | 目录、清单或实际字节损坏，或磁盘与数据库绑定不一致 | 停止激活/回滚该版本，核对根目录、挂载、权限及已有证据；保留现场，不自动删除或覆盖正式目录 |

响应同时包含：

- `artifactDigest`：已验证的磁盘摘要，无法验证时为 `null`。
- `registeredArtifactDigest`：数据库产物表中的摘要，没有绑定时为 `null`。
- `manifest`：已验证的磁盘清单，无法验证时为 `null`。
- `references`：所有环境的 `{id, environment, artifactDigest, entryUrl, active}`。
- `recovery`：只读恢复提示。

`active: false` 不代表没有引用或允许删除。上一部署需另查 `GET /api/applications/{id}/deployments`；版本列表可用 `GET /api/applications/{id}/releases?environment=development` 等接口查询。本阶段的维护工具**不删除任何正式版本目录**，包括看似未登记、非活动、回滚后的目录。

以下 Bash 模板只使用明确填写的隔离/受控目标，不读取仓库或个人 `.env`。先填写真实 HTTPS origin、受信任测试 CA、应用 ID 和版本；`.invalid` 及路径占位不是可直接运行的目标。公开 CA 可使用系统信任库，不能使用 `-k` 跳过验证。

```sh
ZIP_ORIGIN='https://zip.example.invalid'
ZIP_CA_FILE='/absolute/path/to/test-ca.crt'
ZIP_APPLICATION_ID='<application-uuid>'
ZIP_VERSION='1.0.0'
umask 077
ZIP_RECORD_DIR=$(mktemp -d /var/tmp/zip-reconcile.XXXXXX)
read -r -s -p '受控环境平台会话 token: ' ZIP_SESSION_TOKEN
printf '\n'

printf 'header = "Authorization: Bearer %s"\n' "$ZIP_SESSION_TOKEN" |
  curl --fail-with-body --silent --show-error --cacert "$ZIP_CA_FILE" --config - \
    "$ZIP_ORIGIN/api/applications/$ZIP_APPLICATION_ID/releases/storage?version=$ZIP_VERSION" \
    --output "$ZIP_RECORD_DIR/storage.json"
```

保存状态响应、目标环境的 release/deployment ID、产物摘要和请求发生时间。不要把 token、数据库连接串或 TLS 私钥写入报告或存储卷。HTTP 查询失败只表示本次查询没有可靠结果，不能推断为未登记或授权删除。

## 上传超时、提交结果未知及环境提升

上传接口接收归档原始字节：

```http
POST /api/applications/{id}/releases/upload?environment=development&version=1.0.0&entryType=HTML&format=zip
Content-Type: application/zip
```

tar.gz 使用 `format=tar.gz` 和 `Content-Type: application/gzip`。前提是应用挂载路径为 `/micro/<code>`，类型为 `HTML`、运行方式为 `IFRAME`；ESM 应用随宿主构建发布。

连接断开、超时、代理 5xx 或提交确认丢失后，按以下顺序操作：

1. 暂停对该版本的人工改动，保留原始构建包和全部正式文件。
2. 查询 storage、目标环境 releases 及 deployments。`REGISTERED` 可能只涉及另一个环境，必须核对环境、ID 和摘要。
3. 同一环境已有相同摘要记录时，使用其稳定 release ID 继续查询或预检；同版本同产物重试也返回该 ID。上传已完成数据库登记后，后续预检或响应仍可能失败，不能据此删除文件。
4. 产物完整但尚未绑定时，重试**同版本、同产物**。仍无法确认数据库结果时继续保留正式目录，等待查询恢复；不通过改版本号掩盖未知提交，也不直接写数据库补记录。
5. `MISSING` 且已有数据库摘要时，只使用与该摘要一致的原包恢复；`INCONSISTENT` 时先处理存储异常。没有可靠原包或一致副本时，不伪造旧版本，应按独立恢复方案处理或发布新版本。

同一应用的版本号对应跨环境共享的不可变产物。同一环境同摘要重试保留 release ID；提升到另一个环境复用同一磁盘目录与摘要，但创建或复用**该目标环境自己的** release ID，不会自动激活。任一环境已经绑定不同产物、已有目录字节不同或已有发布定义不匹配时返回 `409 CONFLICT`，不覆盖旧目录。数据库事务失败或状态未知时仍保留已安装字节。

应用版本已绑定 ZIP 产物时，普通 `POST /api/applications/{id}/releases` URL 登记返回 `409 CONFLICT`；跨环境提升须通过上传接口提交同版本、同产物，不能改走 URL 登记。

沿用上面的明确参数，可以发送原包重试；此命令不会读取 `.env`，Authorization 经标准输入传给 curl：

```sh
ZIP_ARCHIVE='/absolute/path/to/unchanged-build.zip'
ZIP_ENVIRONMENT='development'
printf 'header = "Authorization: Bearer %s"\n' "$ZIP_SESSION_TOKEN" |
  curl --fail-with-body --silent --show-error --cacert "$ZIP_CA_FILE" --config - \
    --header 'Content-Type: application/zip' --data-binary "@$ZIP_ARCHIVE" \
    "$ZIP_ORIGIN/api/applications/$ZIP_APPLICATION_ID/releases/upload?environment=$ZIP_ENVIRONMENT&version=$ZIP_VERSION&entryType=HTML&format=zip" \
    --output "$ZIP_RECORD_DIR/upload.json"
unset ZIP_SESSION_TOKEN
```

上传会执行预检，但不会自动切换部署。绑定了产物摘要的版本在预检、激活和回滚前还要核对本地清单、实际字节及数据库摘要/清单。HTTP 返回 200 或内容类型正确不能替代完整性检查；本地缺失或损坏会产生 `ARTIFACT` 诊断，不能靠重新标记 `PASSED` 绕过。

## 离线暂存扫描与清理

平台二进制提供独立维护入口：

```text
saas-platform-backend --micro-staging --root <已存在的绝对目录> [--apply]
```

该入口在加载环境配置、创建数据库连接和启动 HTTP 服务之前分派，不依赖 `DATABASE_URL`、JWT 密钥或私有 `.env`。无 `--apply` 时默认为 `SCAN`，**不删除暂存项**；扫描可能创建或同步私有暂存目录及永久锁文件，不能把它理解为完全没有文件系统写入。

在同一主机以发布目录所属身份运行。下面的 native 模板以空环境调用显式二进制，不读取私有环境：

```sh
env -i /absolute/path/to/saas-platform-backend \
  --micro-staging --root /absolute/existing/micro-apps
```

Docker 模板使用已经核对的固定后台镜像、明确的本机 Docker socket 和现有目录；不加载 Compose 环境文件、不启动依赖或 API。`ZIP_BACKEND_IMAGE` 必须替换为实际固定 digest，`ZIP_ROOT` 是本机实际持久路径。维护挂载保持可写，因为 SCAN 也需要维护锁；`--read-only` 约束的是容器根文件系统：

```sh
ZIP_BACKEND_IMAGE='registry.example.invalid/platform-api@sha256:<actual-64-hex>'
ZIP_ROOT='/absolute/existing/micro-apps'
docker --host unix:///var/run/docker.sock run --rm --network none --no-healthcheck \
  --read-only --user 10001:10001 --cap-drop ALL --security-opt no-new-privileges \
  --mount "type=bind,source=$ZIP_ROOT,target=/micro-apps" \
  "$ZIP_BACKEND_IMAGE" --micro-staging --root /micro-apps
```

扫描成功输出 JSON，主要字段如下；这是响应结构说明，不是某次执行的结果：

| 字段 | 含义 |
| --- | --- |
| `mode` | 默认 `SCAN`；明确传入 `--apply` 才为 `APPLY` |
| `eligible` | 允许处理的 UUID-v4 暂存目录名 |
| `removed` | 本次实际移除的暂存目录名；SCAN 始终为空 |
| `retained` | 未识别或不安全的项，原因 `UNRECOGNIZED_OR_UNSAFE` |
| `installedVersionsUntouched` | 此维护入口保持为 `true` |

仅目录名为规范 UUID-v4、`owner.json` 的 schema/UUID/应用/版本有效且匹配该目录、内部只有已识别的 owner/payload、遍历不含链接或特殊文件的暂存项可进入 `eligible`。清理不按目录年龄、PID 或文件名猜测归属；payload 可以是崩溃后未写完的内容。缺少 owner、未知名称、额外文件及链接等项保留供人工调查，不自动删除。

先保存 SCAN 结果，核对目标和 `eligible` 后，使用同一明确根目录加 `--apply`：

```sh
env -i /absolute/path/to/saas-platform-backend \
  --micro-staging --root /absolute/existing/micro-apps --apply
```

Docker 模式将相同 `--apply` 追加到上述 `--root /micro-apps` 之后。命令会重新取独占锁并重新核对每个候选，不盲用旧扫描结果。存在活动写入、锁不可取得、根目录不合法或权限不足时以非零状态退出；成功的空结果也不表示数据库、正式产物或部署已恢复。

不要用 `rm -rf .staging`、通配符、目录年龄脚本或删除维护锁代替该入口。维护命令从不扫描删除 `<app>/versions/<version>`，不能清理最终目录、孤立数据库记录、外部静态 origin 或跨机副本。

## 升级、回退与核验

升级前保存已部署的固定后台/入口/静态镜像身份、数据库备份及持久产物副本，并记录各环境 active/previous release ID。备份与正式恢复应使用已批准并演练的流程；本阶段的暂存清理不提供数据库或正式版本的备份恢复能力。

切换新版或执行暂存维护之前，暂停全部旧上传和发布写入，等待在途请求结束并停止所有旧 writer。旧二进制不遵守新维护锁，其旧 handler 还可能在失败时删除正式目录；**不能让新旧上传 writer 同时读写同一卷**。确认只剩新版写入者后，再启用新版上传和基于维护锁的暂存清理。

`0002_release_artifacts.sql` 新增跨环境产物表、release 摘要绑定、外键及不可变约束，不修改 `0001_init.sql`。正常平台启动会执行嵌入迁移；离线 `--micro-staging` 模式不会执行迁移。先在隔离副本验证升级，再对明确目标启动新版；核对迁移记录、storage 四态以及各环境引用，旧记录的 `artifactDigest` 缺省或为 `null` 不能当作已完成字节核验的证明。

回退后台镜像时保留 0002 的数据和迁移记录，只使用**包含 0002 且已验证兼容当前元数据**的镜像。只嵌入 0001 的旧二进制可能因未知已应用迁移而拒绝启动；不要删除 `_sqlx_migrations`、产物表、摘要列或约束规避校验。本阶段没有自动 down migration；回到更旧数据库结构需要独立的一致恢复演练。

业务版本回滚与后台镜像回退分开处理。通过当前环境 deployments 找到上一 release，确认其正式文件和摘要完整，再执行已有预检/回滚流程。回滚会重新校验产物和资源；完成后核验活动入口、旧版本资源 URL、权限上下文和静态响应。ZIP 版本更新不需要重建宿主静态镜像，但旧文件仍须在线，不能在激活或回滚后立即删除。

真实隔离演练入口为 `tests/zip-release-e2e.mjs`。使用仓库固定工具链、全新绝对输出目录和明确的不可变镜像 ID；详细实际参数以该脚本当前 CLI 为准：

```sh
node tests/zip-release-e2e.mjs \
  --out-dir /absolute/new/phase-05-e2e \
  --frontend-image 'sha256:<actual-immutable-image-id>' \
  --edge-image 'sha256:<actual-immutable-image-id>'
```

该演练构建当前源码平台镜像；也可显式提供当前候选源码受控构建的 PASS `release-result.json`，使用 `--release-report /absolute/path/to/release-result.json` 复用其中三镜像。其中后台必须包含本阶段迁移和维护入口，不能用旧阶段的 PASS 报告代替当前产物。必要的构建代理由操作者显式提供 `--build-proxy`，不从私有 `.env` 推断。演练目标是在自建隔离 PostgreSQL、持久卷和正常验证的 HTTPS 中检查上传、查询/重试、预检、激活、跨环境复用、回滚及旧资源保留；是否通过以实际报告为准。环境缺失或必需检查失败必须报告，不能以 skip 或预期行为充当通过。

检查本次新输出目录中的 `result.json` 和脱敏 `logs/`；私钥与合成运行配置只用于该次隔离环境，不归档到报告或正式静态目录。核对结果报告的源码和镜像身份，以及测试自有容器、网络、卷和敏感暂存的清理记录。

生产容量、保留期、正式目录回收、多机分发和灾难恢复仍有各自的后续阶段，不由本页命令自动启用。
