# URL 微应用静态代理

直接 URL 入口启用 `proxyEnabled` 后，平台通过 `/micro-proxy/{code}/{revision}/` 提供 HTML 与静态资源。业务导航仍是 `/micro/{code}/*`。revision 来自环境 URL 配置；重新保存改变配置会换 UUID，旧 revision 返回 404，避免旧应用加载新上游资源。

## 路径映射

假设登记 `https://orders.example.com/client/index.html?locale=zh`：

| 浏览器资源 URL | 上游请求 |
| --- | --- |
| `/micro-proxy/orders/{revision}/?locale=zh` | `/client/index.html?locale=zh` |
| `/micro-proxy/orders/{revision}/assets/main.js?v=2` | `/client/assets/main.js?v=2` |
| `/micro-proxy/orders/{revision}/orders/123` | `/client/orders/123`，仍按静态类型/MIME约束 |

入口也可使用以 `/` 结尾的上游目录。代理将缺少末尾斜杠的 revision 地址规范化；入口应直接指向最终 HTML 或目录。重定向必须留在登记入口的同源目录内。

代理只负责传输，不全局改写 HTML、CSS 或 JavaScript 的地址：

- qiankun 可以按 HTML 入口解析 script、stylesheet，子应用仍要正确设置 publicPath。
- 挂载后的图片相对路径、内联样式、CSS字体及动态 chunk 可能按宿主业务地址解析，应使用正确构建 URL、运行时 publicPath 或内联资源。
- `/assets/main.js` 指向宿主根，不能自动映射上游 `/client/assets/main.js`。
- Webpack 可采用 qiankun 注入的 `__INJECTED_PUBLIC_PATH_BY_QIANKUN__`；现有 Vite UMD 示例主要使用单包及内联资源。
- 业务路由 base 使用 `/micro/{code}`，资源代理目录不可拿来代替业务路由。

## 准入与凭据

只解析当前服务端环境中、已启用的 HTML/TRUSTED 应用、匹配编码的 `/micro/{code}`，以及开启代理的环境 URL。上游必须位于 `MICRO_ALLOWED_ORIGINS`；生产只接受 HTTPS。浏览器不能通过请求选择任意上游。

代理不复制请求 Authorization、Cookie、Origin 或用户/组织头，禁用系统 HTTP 代理和自动重定向，每次跳转重新验证来源和目录。响应只选取允许的静态信息，剥离 Set-Cookie、上游 CSP/认证挑战等头。资源是匿名静态内容，业务数据依旧由应用授权和业务 API 保护。

该路径支持 GET/HEAD；不承载业务 API、OAuth、Socket.IO/WebSocket、内部指标或 Vite HMR。业务服务单独接入 `/work-orders-api/`、`/stores-api/` 等网关路径。

## 限制和响应

| 限制 | 当前实现 |
| --- | --- |
| 来源限速 | 每来源每 60 秒 600 次，最多 4096 个跟踪来源 |
| 上游并发 | 24 |
| 超时 | 总请求 8 秒，连接 2 秒 |
| 响应体 | 16 MiB |
| 重定向 | 最多 5 次，每次校验 origin 和目录 |
| 响应策略 | `Cache-Control: no-store`、`nosniff`、匿名 CORS、禁止 worker |

隐藏路径、穿越、双重编码、禁止的 API/内部目录及不支持资源类型在转发前拒绝。保留普通 query 原编码，移除 `__platform_entry`、`__saas_execution`、`__saas_lease`。配置入口本身还禁止 token/session/client_secret 等凭据 query。

上游 404 保持 404。JavaScript、CSS 等必须有对应 MIME；缺失脚本被上游 SPA fallback 成 HTML 时返回 502。超时返回 504，限流返回 429，并发耗尽返回不可用。修改入口后旧 revision 请求失败应重新获取授权上下文，不能自动拼接新 revision。

## 接线与验证

Vite 开发代理和生产网关应将 `/micro-proxy/*` 原样送至 Rust API，不启用 WebSocket Upgrade。`/micro/*` 交给平台 SPA。独立前端镜像对未接线的代理请求返回 404，避免当成宿主导航成功。

在 `backend` 执行：

```powershell
cargo test --locked --bin saas-platform-backend micro_proxy::tests -- --test-threads=1
node --test tests/nginx-render-policy.mjs
```

HTTP 专项覆盖无凭据、MIME、GET/HEAD、query、404、重定向、大小与响应头。Windows 若 Rust 临时 HTTP fixture 超时，可为测试进程设置 `MICRO_PROXY_TEST_NODE` 为已有 Node 可执行文件的绝对路径，再重跑；它使用真实 Node HTTP fixture 调用相同 Rust 生产逻辑，不修改系统防火墙。测试结果与未运行的数据库/生产接线项见[审查报告](qiankun-audit.md)。
