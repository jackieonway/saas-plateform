# SaaS 平台

平台由 React 管理宿主和 Rust/Axum API 组成，负责登录会话、多租户组织、角色菜单、功能权限、数据授权、应用入口、版本发布及通知收件箱。工单、门店、通知、SDK 和示例分别维护在独立仓库。

独立微应用在 `/micro/{code}/*` 下通过 qiankun 加载，现行入口契约固定为 `HTML + TRUSTED`，公共 SDK 为 2.0.0。系统管理等平台内置页面仍由 React Router 渲染。外链展示页是单独的嵌入功能，不走微应用生命周期。

当前活动加载链、默认 CSP、业务接入配置与交付门禁已统一，旧宿主示例副本已清理。源码与隔离环境验收结果、本地候选和正式上线条件见[迁移实施与验证](backend/docs/qiankun-audit.md)。

## 工程入口

| 路径 | 用途 |
| --- | --- |
| [frontend](frontend/README.md) | React/Vite 宿主、SDK 固定副本、前端与浏览器测试 |
| [backend](backend/README.md) | Axum API、PostgreSQL 增量迁移、应用发布控制面 |
| [backend/docs](backend/docs/README.md) | 架构、开发、接入、授权、运维及审查导航 |
| [backend/nginx](backend/nginx/README.md) | 网关配置生成与策略 |
| [backend/tools](backend/tools/platform-admin/README.md) | 管理 CLI 与独立部署/制品代理 |
| [backend/ops](backend/ops/README.md) | 监控和恢复工具 |
| [.github](.github/README.md) | 独立仓库 CI 和结构门禁 |
| [RELEASE.md](RELEASE.md) | 版本更新与安全升级记录 |

## 本地运行

按照[开发指南](backend/docs/development.md)准备专属 PostgreSQL 和后端环境，分别在 `backend` 执行 `cargo run --locked`、在 `frontend` 执行 `npm ci` 和 `npm run dev`。API 默认端口为 3001，宿主开发端口为 3000。

空开发库使用演示 seed，登录方式见开发指南；空生产库通过启动配置显式初始化管理员。真实业务用户还需要租户、组织、角色菜单和资源数据授权。应用端 token 与平台管理会话分开使用。

生产交付使用各自目录作为 Docker 上下文，见[部署指南](backend/docs/deployment.md)。本仓库没有根 npm 调度工程，也不在构建时读取相邻业务仓库源码。
