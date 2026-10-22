# 独立业务接入与交付

工单、门店各自拥有前端、Rust API、迁移、权限清单、依赖锁和 CI。平台提供身份与授权，不构建业务源码。

## 接入顺序

1. 在业务仓库构建自身前后端，准备专属 PostgreSQL 数据库与账号。
2. 在平台创建机密客户端，登记业务应用并绑定客户端；真实 ID 来自平台，不用显示名称代替。
3. 使用业务 backend/manifests 中的清单核对应用、菜单、入口和资源动作。
4. 在 DATA_GRANT_POLICY 登记应用 capability、memberLookups 和管理范围。
5. 给普通用户当前有效直接成员配置功能权限与数据授权。
6. 配置业务 API 的平台 HTTPS、客户端凭据、精确来源，再登记前端不可变版本入口。
7. 实际验证列表、详情、写入、成员目录、切组织与撤权；工单另验提醒和收件箱。

管理工具见 [平台管理 CLI](../tools/platform-admin/README.md)，协议见 [应用授权](application-authorization-contract.md)。

## 发布边界

各业务 frontend、backend 分别作为 Docker context，镜像由自身仓库发布。自己的 backend/compose.yml 管理数据库、API 和静态前端；私有 edge 网络与 HTTPS 路由由部署环境配置。平台 Compose 不会自动启动业务。

前端公开配置只包含版本、准确宿主来源与 API 地址；机密客户端 secret、数据库 URL、平台 session 不进入静态构建。SDK 固定版本升级需要来源、摘要和兼容审阅。

## 升级与恢复

发布新静态版本时保留仍被引用的旧资源；后端迁移不修改已执行 SQL。回退镜像先确认数据库兼容。写入或发布未知结果按原事件查询恢复，不自动换键或覆盖版本。三库恢复必须符合当前恢复工具的固定材料协议，见 [恢复指南](../ops/recovery/README.md)。
