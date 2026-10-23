# 平台开发

## 工具与启动顺序

当前基线是 Rust 1.97.0、Node 24.22.1、npm 11.16.0，版本以仓库锁定文件为准；数据库建议使用与 CI 一致的 PostgreSQL 17。Windows 先检查 `node --version` 和 `Get-Command node`，避免被其他软件自带 Node 覆盖。

分别启动数据库、平台 API、宿主前端，再按需启动独立业务和示例。没有根调度脚本，也不会因启动平台而自动启动相邻仓库。

## 平台 API

先创建平台专属数据库和有迁移权限的账号。在本仓库 `backend` 终端设置开发环境；以下尖括号为必须替换的输入：

```powershell
$env:APP_ENV='development'
$env:APPLICATION_ENVIRONMENT='development'
$env:DATABASE_URL='postgresql://<账号>:<URL编码密码>@127.0.0.1:5432/saas_platform'
$env:AUTO_CREATE_DATABASE='false'
$env:JWT_SECRET='<独立随机签名密钥>'
$env:PUBLIC_APP_URL='http://127.0.0.1:3000'
$env:BIND_ADDR='127.0.0.1:3001'
cargo run --locked
```

启动时迁移并初始化平台；`GET /api/health` 返回 200 才说明数据库探测成功。非生产模式进入 `src/seed.rs` 的演示 seed，空用户库会创建开发账号 `admin` / `admin123`，仅用于隔离开发。已有用户库不会靠重复启动重设密码。

`BOOTSTRAP_ADMIN_USERNAME`、`BOOTSTRAP_ADMIN_PASSWORD` 用于空生产库初始化，不替换开发 seed 的登录账号。配置中若提供这两个值，仍须成对并满足校验；生产部署拒绝已启用的演示凭据。具体流程见[部署指南](deployment.md)。

环境加载顺序为进程环境优先，显式 `APP_ENV_FILE` 读取指定绝对文件；非生产才允许读取当前目录 `.env`。程序不搜索相邻仓库或自动读取其他命名环境文件。`AUTO_CREATE_DATABASE` 默认为 false，开启只创建明确不存在的目标库，账号还需相应 PostgreSQL 权限。

迁移嵌入二进制。不要修改数据库 checksum 来绕过错误；当前代码只认可两种明确登记的旧初始基线摘要，其他变更仍校验。现有库升级应先做备份，见[升级指南](upgrade-and-retention.md)。

## 宿主

在本仓库 `frontend` 的另一个终端：

```powershell
npm ci --ignore-scripts --no-audit --no-fund
$env:BACKEND_PROXY_TARGET='http://127.0.0.1:3001'
npm run dev
```

浏览器使用 `http://127.0.0.1:3000`。同一环境中保持 localhost 与 127.0.0.1 的 origin 一致；来源、回调和 Cookie 设置按浏览器实际 URL 配置。额外开发主机名使用 `DEV_ALLOWED_HOSTS`，仅允许逗号分隔的完整主机名，不传 URL、端口或通配符。

## 子应用与代理

子应用必须先单独安装和启动，再在后台登记。编译型子应用的 `npm run dev` 使用 UMD 构建监听/静态预览，不能把原生 Vite module HTML 交给 qiankun。

| 组件 | 默认端口 | 说明 |
| --- | --- | --- |
| 工单前端 / API | 3013 / 3004 | 宿主业务路径 `/work-orders-api/` |
| 门店前端 / API | 3014 / 3031 | 宿主业务路径 `/stores-api/` |
| Vue / jQuery / 报表 | 3005 / 3006 / 3009 | 独立示例工程 |
| 聊天前端 / API | 3007 / 3008 | 显式启用聊天连接，独立于通知 |
| OAuth 前端 / API | 3010 / 3011 | 独立演练会话与 CSRF |
| 通知服务 | 以通知配置为准 | 可选 Socket.IO 服务 |

直接 URL 的 origin 必须列入 API 的 `MICRO_ALLOWED_ORIGINS`，例如将本次实际使用的 `http://127.0.0.1:3013` 加入列表后重启 API。原有列表也需保留所需获准来源。生产使用 HTTPS。

宿主开发代理配置位于 `frontend/vite.config.ts`：

| 配置 | 默认目标与用途 |
| --- | --- |
| `BACKEND_PROXY_TARGET` | 3001，平台 API/OAuth/静态代理 |
| `WORK_ORDERS_DEV_PROXY_TARGET` | 3004，工单 API |
| `STORES_DEV_PROXY_TARGET` | 3031，门店 API |
| `OAUTH_PLAYGROUND_DEV_PROXY_TARGET` | 3011，`/oauth-playground-api/*` 改写为演练 `/api/*` |
| `OAUTH_PLAYGROUND_STATIC_DEV_PROXY_TARGET` | 3010，`/sub/oauth-playground/*` 与精确 `/callback` |
| `MICRO_APPS_DEV_DIR` | 可选绝对路径版本仓，提供 `/sub-apps/{code}/versions/{version}/*` |

OAuth 代理移除平台 Authorization，只允许演练 Cookie，保留 CSRF/Origin 和 Set-Cookie；静态入口不带凭据。演练服务的 public origin 与客户端 callback 必须按实际宿主配置。示例提供自身可运行的开发调度和对应边缘路由，具体参数见示例各工程 README。

业务服务从 qiankun 页面收到的跨域来源是宿主 origin。工单的 `WORK_ORDERS_ALLOWED_ORIGINS` 和门店的 `STORES_ALLOWED_ORIGINS` 现在显式必填，缺失、空值、`null` 和通配符会在启动配置校验时被拒绝；本地填写实际的 `http://127.0.0.1:3000`，生产填写准确 HTTPS 来源。宿主旧 `public/sub-apps/vue`、`jquery` 副本已删除，示例从独立工程构建。完整结果见[实施与验证](qiankun-audit.md)。

## 登记与验证

通过应用管理创建 HTML/TRUSTED 应用，配置 URL 或固定版本，再授权租户、组织、角色菜单。调用业务 API 还需机密客户端、功能权限与数据授权，见[接入指南](micro-application-onboarding.md)和[业务交付](independent-business-delivery.md)。

在前端依次执行 `npm run sdk:check`、`npm run lint`、`npm run typecheck`、`npm test`、`npm run build`。后端执行 `cargo fmt --all -- --check`、`cargo check --locked --all-targets`、`cargo test --locked --all-targets`。数据库和浏览器专项需要额外环境，见[工程验证](engineering-baseline.md)。

401/403 优先核对凭据类型、应用绑定、期限、当前组织、permission 和数据授权；404 可能是无权访问或静态资源缺失。结果未知的写入先查回状态，不根据超时判断事务已回滚。
