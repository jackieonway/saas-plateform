# 平台管理命令行工具

`saas-platform-admin` 是独立 Rust CLI，通过正式平台管理 API 处理工单和门店的登记、授权与版本流程。默认无参数只输出离线计划；它不创建 OAuth 客户端、不执行 SQL、不构建源码、不安装 ZIP，也不控制 Docker。

## 构建与帮助

在本目录执行：

```sh
cargo test --locked
cargo build --release --locked
cargo run --locked -- --help
cargo run --locked -- plan
```

可执行文件位于本工程 `target/release`，Windows 带 `.exe`。下文假定二进制已放入 `PATH`；也可以使用它的绝对路径。

## 连接与权限

远程命令通过受控进程环境读取：

| 变量 | 用途 |
| --- | --- |
| `PLATFORM_API_ORIGIN` | 准确平台 HTTPS 来源 |
| `PLATFORM_ADMIN_BEARER` | 当前有效的平台管理 Bearer |
| `PLATFORM_CA_FILE` | 私有 CA 文件，按需配置 |

秘密不写命令行、业务清单或日志。默认环境为 `production`；隔离回环开发需要同时指定 `--environment development --allow-loopback-http`。`--app-code` 仅支持 `work-orders`、`stores`，默认工单；`--app-id` 及清单中的实体标识须为 UUID v7／RFC variant，旧 v4、nil 和错误 variant 在发出请求前拒绝。`--timeout-ms` 为整条命令共享的 10–35000 毫秒预算。

CLI 不授予额外管理权限。平台实时检查身份、平台上下文、资源权限和 `DATA_GRANT_POLICY`，计划成功不代表可以执行写入。

## 命令清单

| 命令 | 行为 |
| --- | --- |
| `plan` | 离线显示内置应用登记计划，不联网 |
| `inspect` | 查询应用和菜单 |
| `bootstrap` | 默认预览，`--apply` 才补齐应用和菜单缺项 |
| `access-policy-read --policy-target application --policy-id UUID` | 读取具体应用的当前准入策略，禁止 `--apply` |
| `access-policy-save --manifest FILE` | 默认离线预览；`--apply` 以清单旧版本保存策略 |
| `grants-plan --manifest FILE` | 校验和预览数据授权清单 |
| `grants-apply --manifest FILE` | `--apply` 才通过管理 API 执行 |
| `artifact-bind`、`artifact-verify` | 离线核对 release/artifact/inventory 声明，bind 写入需 `--apply` |
| `register`、`release-reconcile` | 登记固定 URL 版本，或对未知登记结果进行核对 |
| `releases`、`deployments` | 查询版本和部署指针 |
| `preflight --release-id UUID` | `--apply` 才提交服务端预检 |
| `activate --release-id UUID` | `--apply` 才激活已通过版本 |
| `rollback` | `--apply` 才请求重新预检并回退 |
| `retention-preview` | 远程只读预览，无 apply 能力 |

读命令不接受 `--apply`。精确参数组合以当前二进制 `--help` 为准。没有环境 URL 配置、ZIP 上传、ZIP 分发或受管镜像任务的 CLI 命令，这些由平台对应管理界面/API 完成。

## qiankun 内置登记

内置清单采用 SDK 3.0.0，应用为 `HTML`、`TRUSTED` 且 `allowedMemberTypes=["EMPLOYEE"]`。`inspect` 和 `bootstrap` 只维护业务应用与菜单，不读取或绑定 OAuth 客户端。宿主向 `/api/apps/session-token` 申请绑定原始 BFF 来源的内部应用能力；外部 USER/MACHINE 客户端另经身份服务的统一注册簿管理。用途不符的现有应用返回冲突，不自动扩大准入。

| 应用 | 宿主导航 | 内置本机开发 HTML 入口 |
| --- | --- | --- |
| `work-orders` | `/micro/work-orders` | `http://127.0.0.1:3013/index.html` |
| `stores` | `/micro/stores` | `http://127.0.0.1:3014/index.html` |

`bootstrap` 只补缺项，不迁移已有登记。回环入口仅适合平台 API 和静态服务同主机的本机开发；API 在容器内时，回环指容器自身。生产先在平台后台配置准确 HTTPS 环境 URL，或使用不可变发布版本，再验证真实 qiankun 产物。

`/micro/*` 是宿主导航，代理开启后的 HTML 与资源通过 `/micro-proxy/*` 提供；固定发布资源使用 `/sub-apps/<app>/versions/<version>/`。这些路径不能混用，见[环境 URL](../../docs/environment-url-entries.md)和[微应用接入](../../docs/micro-application-onboarding.md)。

清单和 schema 随二进制编译，修改后重新构建。业务仓库副本及依赖摘要须与实际契约同步。内置 `grants` 为空，功能权限、菜单语义和数据授权单独审阅。

门店内置清单包含 7 项资源动作权限、9 个菜单条目：根菜单不声明权限，营业记录详情页复用 `store-daily-report:read`。`store:create` 用于为已存在的平台 `STORE` 组织完善营业资料；`store-staff:read` 读取平台门店成员。组织及成员关系由平台维护，清单不包含员工关联、解除关联权限。业务后端的受控组织和成员查询用途由平台策略单独配置，不扩展浏览器 SDK。CLI 会拒绝带 `store-staff:assign`、`store-staff:unassign` 的新授权清单；已有菜单和授权需管理员显式审阅，重复 `bootstrap` 仍只补缺项。

## 典型流程

```sh
saas-platform-admin inspect --app-code work-orders
saas-platform-admin bootstrap --app-code work-orders
saas-platform-admin grants-plan --manifest /srv/saas/config/work-orders-grants.json
saas-platform-admin releases --app-code work-orders
saas-platform-admin deployments --app-code work-orders
saas-platform-admin retention-preview --app-code work-orders --minimum-age-days 30 --keep-latest 5
```

确认计划符合目标后，在原命令添加 `--apply` 执行支持写入的命令。授权清单取所属业务 `backend/manifests`，使用真实租户、组织、成员与应用 UUID。角色/用户集合类变更先读取完整现值，避免覆盖无关项，见[数据授权管理](../../docs/data-grant-management.md)。

## 应用准入策略

先读取具体记录的当前策略，再按 [策略 schema](schemas/access-policy.schema.json) 填写[策略清单](manifests/access-policy.manifest.example.json)。`--policy-id` 和清单 `id` 使用数据库记录 UUID v7。

```sh
saas-platform-admin access-policy-read --policy-target application --policy-id 00000000-0000-7000-8000-000000000001
saas-platform-admin access-policy-save --manifest /srv/saas/config/application-access-policy.json
saas-platform-admin access-policy-save --manifest /srv/saas/config/application-access-policy.json --apply
```

清单必须提供 `target=application`、`id`、`allowedMemberTypes`、读取到的 `accessPolicyVersion` 和真实 `reason`。原因 trim 后非空、最多 500 字符且不得含控制字符。成员集合非空，只能包含 `EMPLOYEE`、`CUSTOMER` 且不能重复。旧客户端策略和 `subjectMode` 清单拒绝；统一客户端使用身份服务的管理命令、版本和原 operation 恢复合同。

预览只校验本地清单，不读取凭据或联网，原因内容不回显。执行时先读当前版本，版本不符直接返回 `ACCESS_POLICY_CONFLICT`／409；随后 PUT 仍携带原版本，由服务端事务防止读取后的并发覆盖。CLI 不刷新旧版本、不重试写入。实际策略变化由服务端递增版本并撤销相应凭据；相同策略不递增版本、不追加审计。未知写入结果要求先读取并核对，不能直接重试。响应只输出策略标识、类型、版本、状态，不输出秘密或任意远端字段。

准入策略不创建客户成员，也不授予资源权限。既有员工应用保持 `EMPLOYEE`；只有完成登记和安全基线的客户用途才可显式开放客户。客户与同组织第二身份的创建仍受平台生命周期开关限制。

## 版本与产物绑定

`--release-file` 遵循 [release schema](schemas/release.schema.json)：工单和门店资源需位于所属应用、版本的固定前缀。`artifact-bind`、`artifact-verify` 要求绝对 `--release-file`、`--artifact-file`、`--inventory-dir`；`register`、`release-reconcile` 可携带成对的 artifact/inventory 参数。

优先核对完整产物声明后再登记。未提供产物对的兼容登记路径不验证 digest；离线绑定也只证明本地材料一致，不证明平台元数据、远端静态 URL、容器就绪或 qiankun 生命周期。

写入结果未知时，保留同一文件与原意图，先 `release-reconcile` 或读取当前状态。不能换文件、换版本或换请求编号重复写入。完整交付见[独立业务交付](../../docs/independent-business-delivery.md)。
