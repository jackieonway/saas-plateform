# 全新安装与数据库边界

本系统从当前空库基线开始，不再保留旧版本数据库的就地升级、UUID 重编号或迁移历史接管。旧库不能通过删除 `_sqlx_migrations` 或改写 checksum 冒充新库；重建必须明确指定目标库，由管理连接执行。

| 数据库 | 建表归属 | 初始化文件 |
| --- | --- | --- |
| 平台共享库 | 平台 canonical migration job；身份和平台分别使用受限运行角色 | `saas-plateform/backend/migrations/0001_init.sql` |
| 门店库 | 门店 API | `saas-stores/backend/migrations/0001_stores.sql` |
| 工单库 | 工单 API | `saas-work-orders/backend/migrations/0001_work_orders.sql` |
| 客户门户库 | 客户门户 BFF | `saas-customer-portal/backend/migrations/0001_customer_portal.sql` |

身份服务使用共享库中的账号、SSO 与统一认证 `auth_*` 模型，包括唯一应用投影／客户端／凭据、资源和 scope、同意、pairwise subject、授权请求／code、grant、opaque Access Token、refresh family、密钥生命周期和可靠投递。平台 runtime 无权直接读取或写入身份私有表。通知服务只使用平台通知数据。

当前 canonical `0001` 已删除旧 `oauth_*`／`oidc_*` 登记、签发、consent/token 运行表及其桥接函数和 ACL；新库只建立统一体系。管理命令收据与平台 intent 分别使用 `identity_auth_command_receipts`、`platform_auth_command_intents`，保留原 operation、摘要与来源 fence，不构成第二个登记权威。平台内部 BFF 会话、`platform_app_capabilities` 与 `platform_resource_services` 分别承担会话、内部应用能力和私有调用方登记。

## 初始化顺序

1. 明确环境与数据库名，创建四个空数据库和专属账号。不要将库名、密码或账号从旧环境隐式继承到新环境。
2. 在平台 `backend` 目录，以明确的进程 `DATABASE_URL`、`BOOTSTRAP_ADMIN_PASSWORD` 执行 `cargo run --locked --bin platform-bootstrap`，一次完成当前结构基线和平台基础数据初始化。若部署作业需要单独建表，可用 `platform-migrate`；该命令仍只建表，不启动 HTTP、不导入数据，也不创建用户或演示业务数据。
3. 在门店、工单 `backend` 目录分别以专属 URL 执行 `stores-migrate`、`work-orders-migrate`。完整命令和环境变量见各项目 README。客户门户在完整配置校验后应用自身 SQLx 基线；也可由统一发布作业调用其同一迁移文件。
4. 平台基础数据准备后，登记固定 HTTPS issuer、独立签名密钥、分别为平台员工和门户客户登记的独立 WEB_BFF / USER / PLATFORM_CONTEXT RP、精确 callback、CA 和不同用途的服务凭据，再完成运行角色权限交接。表结构存在不代表 RP 和业务授权已登记。
5. 在共享平台库完成[运行角色交接](../ops/identity-deployment/README.md)，再用只读 readiness 验证启动身份和平台 runtime。独立部署的 runtime 不代替迁移作业执行 DDL。
6. 使用新数据库分别启动业务 API、客户门户与通知服务，最后启动宿主和所需微应用。

每个库的 SQLx 历史仅有版本 1 和当前文件的 SHA384 checksum。首次建表具有事务边界；再次启动校验同一基线，不重新生成业务数据。工单和门店还核对表、字段、约束、索引、触发器和函数的结构指纹；漂移必须显式处理，不能重新签认旧状态。

## 新库约束

所有应用生成的标识使用 UUIDv7 和 RFC variant，数据库不依赖 PostgreSQL 18 的 `uuidv7()`。建议 PostgreSQL 17 或更新版本。成员键为 `(user_id, organization_id, member_type)`，不需要旧成员收缩工具或阶段 receipt。

平台会话必须具有真实 BFF/OIDC 绑定；schema 不预建旧控制台 OAuth 客户端，也不导入旧账号、会话、令牌、审计、工单或门店记录。平台租户、根组织、超管及基础菜单授权由显式初始化步骤创建，RP 和业务授权按实际部署另行登记。

## 平台租户与超管初始化

初始化数据语句为 [`initialization/0001_platform.sql`](../initialization/0001_platform.sql)，由 `platform-bootstrap` 校验／应用当前结构基线后，在事务内执行，与只建表的 `migrations/0001_init.sql` 分开。该命令应在 SSO 权限交接前使用数据库管理连接执行；认证中心和平台的受限运行账号不负责初始化。

首次执行创建：平台管理租户（`admin-tenant`）、平台总部根组织（`HQ`）、一个超管账号、该账号的员工成员关系及 `super_admin` 角色，以及系统管理／消息中心应用和完整的基础菜单授权。生成的标识均为 UUIDv7，不创建演示租户、普通账号、OAuth 客户端或登录会话。

`DATABASE_URL` 和 `BOOTSTRAP_ADMIN_PASSWORD` 必须通过进程环境提供，`BOOTSTRAP_ADMIN_USERNAME` 默认 `admin`。密码至少 12 位，包含大小写字母、数字和符号；使用带随机盐的 Argon2id 保存，命令不会输出密码或摘要。可在 WSL zsh 中执行：

```zsh
# DATABASE_URL 已由当前部署提供，准确指向本次专用的平台共享库。
read -rs 'BOOTSTRAP_ADMIN_PASSWORD?请输入初始超管密码：'
export BOOTSTRAP_ADMIN_PASSWORD
BOOTSTRAP_ADMIN_USERNAME=admin cargo run --locked --bin platform-bootstrap
unset BOOTSTRAP_ADMIN_PASSWORD
```

重复执行会核对并返回原有平台租户、根组织和超管标识，不增加账号、不覆盖密码或授权。已有数据与指定初始化账号不匹配时拒绝执行；失败会回滚整笔事务。初始化账号通过认证中心 SSO 登录，平台仍只支持 SSO。

## 重置范围与资源

本机数据重置只处理已确认属于本工作区的数据库、临时资源和旧验证产物。远端库需要明确纳入目标清单后才执行删除。完成后记录库名、表数、业务行数及验证结果，不记录连接串或凭据。

开发验证结束清理本次容器、日志、临时卷、网络、镜像及无用构建缓存。Docker 内部回收与 WSL VHDX 文件压缩分别记录，不能互相替代。
