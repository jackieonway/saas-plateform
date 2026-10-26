# 全新安装与数据库边界

本系统从当前空库基线开始，不再保留旧版本数据库的就地升级、UUID 重编号或迁移历史接管。旧库不能通过删除 `_sqlx_migrations` 或改写 checksum 冒充新库；重建必须明确指定目标库，由管理连接执行。

| 数据库 | 建表归属 | 初始化文件 |
| --- | --- | --- |
| 平台共享库 | 平台 canonical migration job；身份和平台分别使用受限运行角色 | `saas-plateform/backend/migrations/0001_init.sql` |
| 门店库 | 门店 API | `saas-stores/backend/migrations/0001_stores.sql` |
| 工单库 | 工单 API | `saas-work-orders/backend/migrations/0001_work_orders.sql` |
| 客户门户库 | 客户门户 BFF | `saas-customer-portal/backend/migrations/0001_customer_portal.sql` |

身份服务使用平台共享库中的账号、SSO/OIDC 和身份命令表；通知服务使用该库中的通知表，不创建第二份账号或通知数据。SDK、微应用前端和示例不另建业务数据库。

## 初始化顺序

1. 明确环境与数据库名，创建四个空数据库和专属账号。不要将库名、密码或账号从旧环境隐式继承到新环境。
2. 在平台 `backend` 目录，以明确的进程 `DATABASE_URL` 执行 `cargo run --locked --bin platform-migrate`。该命令只建表，不启动 HTTP、不导入数据，也不创建用户或演示业务数据。
3. 在门店、工单 `backend` 目录分别以专属 URL 执行 `stores-migrate`、`work-orders-migrate`。完整命令和环境变量见各项目 README。客户门户在完整配置校验后应用自身 SQLx 基线；也可由统一发布作业调用其同一迁移文件。
4. 配置管理员 bootstrap、固定 HTTPS issuer、独立签名密钥、PLATFORM_CONSOLE 与 CUSTOMER_PORTAL RP、精确 callback、CA 和不同用途的服务凭据。表结构存在不代表账号、RP 或业务授权已登记。
5. 在共享平台库完成[运行角色交接](../ops/identity-deployment/README.md)，再用只读 readiness 验证启动身份和平台 runtime。独立部署的 runtime 不代替迁移作业执行 DDL。
6. 使用新数据库分别启动业务 API、客户门户与通知服务，最后启动宿主和所需微应用。

每个库的 SQLx 历史仅有版本 1 和当前文件的 SHA384 checksum。首次建表具有事务边界；再次启动校验同一基线，不重新生成业务数据。工单和门店还核对表、字段、约束、索引、触发器和函数的结构指纹；漂移必须显式处理，不能重新签认旧状态。

## 新库约束

所有应用生成的标识使用 UUIDv7 和 RFC variant，数据库不依赖 PostgreSQL 18 的 `uuidv7()`。建议 PostgreSQL 17 或更新版本。成员键为 `(user_id, organization_id, member_type)`，不需要旧成员收缩工具或阶段 receipt。

平台会话必须具有真实 BFF/OIDC 绑定；schema 不预建旧控制台 OAuth 客户端，也不导入旧账号、会话、令牌、审计、工单或门店记录。应用、角色菜单、RP 和管理员属于显式初始化数据，按实际部署提供。

## 重置范围与资源

本机数据重置只处理已确认属于本工作区的数据库、临时资源和旧验证产物。远端库需要明确纳入目标清单后才执行删除。完成后记录库名、表数、业务行数及验证结果，不记录连接串或凭据。

开发验证结束清理本次容器、日志、临时卷、网络、镜像及无用构建缓存。Docker 内部回收与 WSL VHDX 文件压缩分别记录，不能互相替代。
