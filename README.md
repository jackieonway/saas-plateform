# SaaS 平台

React/Vite 管理宿主与 Rust/Axum API，负责租户、组织、具体成员、角色菜单、数据授权、应用接入与发布、通知收件箱。账号认证由独立身份服务承担。

平台唯一登录方式为 SSO/OIDC BFF：平台 → 认证中心 → 平台回调 → 员工身份选择。浏览器使用 HttpOnly Cookie 恢复会话，短期业务 Bearer 仅保存在内存；没有本地密码登录或故障回退。

| 目录 | 内容 |
| --- | --- |
| [frontend](frontend/README.md) | 管理宿主、SDK 固定副本、前端和浏览器测试 |
| [backend](backend/README.md) | API 与唯一数据库初始化基线 |
| [文档](backend/docs/README.md) | 当前架构、开发、部署及业务合同 |
| [网关](backend/nginx/README.md) | HTTPS、来源与资源代理策略 |
| [管理 CLI](backend/tools/platform-admin/README.md) | 应用登记、授权、制品及发布管理 |
| [运维](backend/ops/README.md) | 监控、备份恢复和身份数据库权限 |

从[全新安装](backend/docs/fresh-installation.md)开始准备数据库，再按[开发指南](backend/docs/development.md)配置身份服务与 HTTPS 网关。后端默认端口 3001，宿主默认端口 3000；相邻项目独立安装、构建和部署。

系统管理页面由 React Router 渲染；微应用在 `/micro/{code}/*` 使用 qiankun，入口固定为 `HTML + TRUSTED`，活动 SDK 为 3.0.0。门店身份由平台 `STORE` 组织维护，营业资料由独立门店 API 保存。
