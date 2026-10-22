# 平台开发指南

## 环境和启动顺序

准备 Rust 1.97.0、Node 24.18.0、npm 11.16.0，以及可连接的 PostgreSQL。建议与部署/恢复链路统一使用 PostgreSQL 17。先启动数据库与平台 API，再启动平台前端，最后按需要启动业务和通知。

命令默认使用独立终端；本地 PowerShell 示例不会启动其他仓库。

## 1. 启动后端

先用数据库管理工具创建专属数据库与账号，确保账号可执行本项目 migrations。进入本仓库 backend：

```powershell
$env:APP_ENV='development'
$env:APPLICATION_ENVIRONMENT='development'
$env:DATABASE_URL='postgresql://<账号>:<URL编码密码>@127.0.0.1:5432/saas_platform'
$env:AUTO_CREATE_DATABASE='false'
$env:JWT_SECRET='<独立随机签名密钥>'
$env:BOOTSTRAP_ADMIN_USERNAME='<初始管理员>'
$env:BOOTSTRAP_ADMIN_PASSWORD='<初始管理员密码>'
$env:PUBLIC_APP_URL='http://127.0.0.1:3000'
$env:BIND_ADDR='127.0.0.1:3001'
cargo run --locked
```

尖括号内容必须替换。启动时执行迁移和初始化；不要靠修改已应用迁移的内容或 checksum 解决启动错误。/api/health 用于检查 API 及数据库可用性。

配置加载顺序：进程环境优先；设置 APP_ENV_FILE 时读取指定绝对路径；未设置且非生产时可读取当前 backend/.env；生产不自动读取 .env。已有 .env.root-local 是单独保留文件，程序不会自动选它。

## 2. 启动前端

在另一个终端进入本仓库 frontend：

```powershell
npm ci
$env:BACKEND_PROXY_TARGET='http://127.0.0.1:3001'
npm run dev
```

打开 http://127.0.0.1:3000。后端 PUBLIC_APP_URL 应与浏览器使用的来源一致。首次登录后按平台界面创建租户、组织、角色与用户，不把初始化管理员当作普通业务用户。

## 3. 接入业务应用

工单、门店按自身开发指南配置专属数据库与机密客户端。平台登记应用、菜单、功能权限和数据授权后，微应用才能代表普通用户调用业务 API。

本地业务前端推荐显式使用绝对 API 地址：工单 http://127.0.0.1:3004/work-orders-api，门店 http://127.0.0.1:3031/stores-api。默认 opaque iframe 请求来源为 null，由业务后端按配置允许，不能以通配来源和 Cookie 替代授权。

宿主当前 /work-orders-api 开发代理写死指向 3104，与工单 API 默认 3004 不同；使用相对路径前须对齐端口。宿主没有默认门店 API 代理。不要把前端启动成功当作业务链路已接通。

可选静态版本目录通过 MICRO_APPS_DEV_DIR 配置绝对路径；缺失版本返回 404。Vue/jQuery 示例代理、聊天及 Socket.IO 等配置见 frontend/vite.config.ts，服务需分别启动。

## 4. 日常校验与排错

- 前端执行 npm run sdk:check、npm run lint、npm run typecheck、npm test、npm run build。
- 后端执行 cargo fmt --all -- --check、cargo check --locked --all-targets、cargo test --locked --all-targets。
- 端口被占用时先核对进程；修改端口后同步代理、来源、回调地址和应用登记。
- 401/403 先检查凭据类型、应用绑定、当前组织、菜单 permission 与数据授权；404 也可能表示记录在当前范围不可见。
- 结果未知的创建、发布或版本切换先查询确认，不能把网络超时当成事务回滚。

数据库与浏览器专项检查需要隔离环境，具体边界见 [工程与验证](engineering-baseline.md)。
