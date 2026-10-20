# OAuth 协议实验室静态页面

此目录是独立 Vite 工程，只负责公开界面。四类流程的客户端认证、授权状态、PKCE、设备码、令牌和资源代理均由相邻的 Rust API 实现。页面通过同源 `/api/*` 和 HttpOnly 会话 Cookie 操作，不连接平台机密端点。

## 本地构建

使用 Node **24.18.0**、npm **11.16.0**。Vite 精确锁定为 **8.3.4**，无运行时框架依赖，无 CDN 资产。

```powershell
npm ci
npm test
npm run build
npm run dev
```

开发与预览默认监听 `127.0.0.1:3010`，同源代理到 `http://127.0.0.1:3011`。可在启动 Vite 前设置以下仅供工具端读取的变量：

| 配置 | 默认值 | 用途 |
| --- | --- | --- |
| `OAUTH_PLAYGROUND_API_TARGET` | `http://127.0.0.1:3011` | `/api`、`/health` 开发/预览代理目标 |
| `OAUTH_PLAYGROUND_FRONTEND_PORT` | `3010` | 前端开发/预览端口 |

配置不会注入浏览器产物。客户端密钥等私有设置只能交给 Rust API，不应写入 Vite 的环境变量或静态文件。公开 origin、scope、各流程是否启用由运行时 `GET /api/config` 返回。完整私有配置、HTTPS 和镜像部署见上级 README。

## 静态交付与回调

构建产生 `dist/index.html`、`dist/callback.html` 及本地 `dist/assets/*`。生产静态服务应明确映射 `/` 到 `index.html`、`/callback` 到 `callback.html`，并由同站入口转发 `/api/*` 至 Rust API。缺失资源必须返回 404，不得使用 SPA 全路径回退。回调地址必须与平台登记的地址、API `publicOrigin` 一致。

回调页在任何网络请求前立即移除 query。随后仅一次 POST `/api/callback` 处理授权结果。设备流程只接收公开用户码，按 `interval` 轮询，遇到 `slow_down` 将间隔增加 5 秒；停止、重启、过期和离开页面都会取消旧请求和计时器。

password 是默认禁用的历史兼容演练，不作为新业务登录方案。服务端能力关闭时，密码表单不可操作。页面不预置测试账号或客户端密钥。普通 OAuth 令牌调用平台应用绑定资源时，界面展示平台真实拒绝结果，不构造成功响应。

## 验证

`npm test` 使用 Node 内置测试运行器，无浏览器或外部平台依赖。可控时钟覆盖设备授权等待、降频、重启取消、在途过期和拒绝终止；API 测试覆盖同源 JSON 请求与嵌套凭据字段过滤。真实平台浏览器/HTTPS/交付验收记录在 `docs/validation/phase-24.md`。
