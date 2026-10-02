# Phase 9 独立发布与隔离接入验收

## 验收范围

应用定义及授权、不可变发布版本、环境部署指针分别保存；平台管理员人工预检、启用、回滚。应用默认 `IFRAME`，只有管理员明确标记 `TRUSTED` 的自有应用允许在宿主页执行 HTML/ESM。

本阶段功能与下列验收门禁均已通过，台账标为 `COMPLETED`。实际生产部署与容器演练属于 Phase 12，未以本机测试替代。

## 已完成的验证

| 门禁 | 结果 | 证据 |
| --- | --- | --- |
| 上下文、沙箱与 Vue/jQuery 回归 | PASS，31/31 | `tests/micro-context-runtime.mjs`、`tests/micro-sandbox-lifecycle.mjs`、两个示例 smoke test |
| iframe 协议及真实隔离 | PASS，16/16 | `MICRO_IFRAME_BROWSER=1` 运行 `tests/micro-iframe-protocol.mjs`；真实 Chromium 验证同源 iframe 仍不可访问宿主 DOM、全局对象及 localStorage |
| 宿主与管理页面浏览器流程 | PASS，21/21 | `tests/micro-host-browser.mjs`；覆盖可信示例、默认 iframe、过期 TRUSTED profile、禁止 iframe ESM、版本启用与回滚；实际 React 管理页面登记/诊断/激活/回滚与平台权限负向场景 |
| 真实 Vue 独立版本资源 | PASS | 用实际打包 CLI 将 Vue/vendor 产物打包为 1.0.0 和内容不同的 1.0.1；浏览器验证新资源及回滚，宿主文档和 token 管线保持，旧 props.signal/回调取消 |
| TypeScript | PASS | Node 24 执行 `tsc -b frontend/tsconfig.json` |
| 宿主生产构建 | PASS | Vite production build；已有 circular chunk 与大于 500 KiB chunk 告警保留给交付阶段处理 |
| 差异格式 | PASS | `git diff --check` |
| 不可变打包工具 | PASS，29/29 | WSL/Linux Node 24；实际文件、软链接/junction、路径/敏感文件和版本覆盖负向测试 |
| Caddy 网关 | PASS，17/17 | Caddy 2.10.2，Windows：配置校验与 16 个实际 HTTP 子项 |
| Nginx 网关 | PASS，17/17 | Nginx 1.28.0，WSL 局部编译：配置校验与 16 个实际 HTTP 子项 |
| Compose 配置 | PASS | 官方 Compose 2.39.4 `config --quiet`；缺少 Docker daemon，未执行镜像构建和容器运行 |
| Rust 编译及默认单元测试 | PASS，4/4 | `cargo check`，默认 `cargo test`；包含静态文件真实进程内路由测试 |
| 真实 PostgreSQL / HTTP / 发布 API | PASS，8/8 | WSL 运行原始 ignored 集成测试，25.76 秒；7 个随机数据库全部断言、清理及连接池关闭成功 |

浏览器测试使用独立 API 响应与随机临时资源目录，未读取数据库凭据。真实 PostgreSQL 和实际 HTTP 预检测试另行记录，浏览器 fixture 不能替代服务端验收。

## 已修复的审查问题

- 缓存的 TRUSTED profile 不能决定再次签发 token 后的执行模式；宿主核对应用上下文中的实时运行模式，不一致就停止加载并刷新 profile。
- 挂载实例和授权上下文共同约束 props.signal；版本替换、卸载及组织切换取消旧业务请求与回调。
- 续期取消旧 iframe update 时忽略正常 AbortError；初次握手取消后使用最新上下文重建实例。
- 并发预检返回本次实际结果，不读取另一检查覆写的 PASSED；部署事务再核对本次检查使用的运行模式。
- 新版本及存量微应用路径的缺失资源不得回退为宿主 HTML；Rust 单进程静态托管也只给宿主 HTML 导航使用 SPA fallback。
- 原生 module 的 opaque origin、嵌入限制头、匿名资源规则纳入预检；公开版本资源提供匿名 CORS，API/OAuth 不放宽。TRUSTED HTML 的原生 module 会给出不支持诊断，可使用 iframe HTML 或直接 ESM。
- 网关实测修复 Nginx 旧目录首页 404，补齐 JS/MJS/CJS/CSS 的准确 MIME、错误 no-store 与不可变资源缓存。

## 升级与验收边界

0021 将存量应用默认设为 IFRAME。现有 Vue/jQuery 等宿主生命周期产物需要平台管理员审核后明确标记 TRUSTED，或改用 iframe SDK。运行模式实际变更会撤销对应应用 token，平台 session 保持有效。

自动审批拒绝迁移时一次性撤销所有旧应用 token，理由是持久全局变更会中断现有应用集成。该变更未执行；升级前签发的旧应用 token 按原最长 5 分钟 TTL 到期。新宿主始终采用 iframe 默认值及实时模式校验。

本机 Windows Rust/.NET/Nginx TCP loopback 探针超时；真实 Chromium 的 IPv6 无代理通道和 Windows Caddy 通道可用。WSL 对同一 `SELECT repeat('x',8000)` 的对照探针显示：普通传输超时，测试进程设置 `TCP_MAXSEG=1000` 后完整响应立即到达。最终 Rust gate 在该局部传输设置下通过，原始 SQL、HTTP 内容及业务断言保持完整。该设置仅在 Git 忽略的测试工具路径中作用于测试进程；实际生产网络仍须单独验收。Nginx 已用局部 Linux 编译实测；网关二进制和生成的版本目录均在 Git 忽略路径，不是生产部署。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 独立 Vue 新资源无需重建宿主，授权不变 | PASS | 浏览器实际打包版本 1.0.0→内容不同的 1.0.1→回滚，宿主文档及 token 管线保持；后端完整授权快照不变 |
| 无效入口、错误 MIME、缺失资源与健康失败阻止启用 | PASS | 真实 HTTP 故障、redirect、来源/CORS 和 module/frame 矩阵；启用前再检；Caddy/Nginx 缺文件真实 404 |
| 切回上一可用版本，入口与资源一致 | PASS | PostgreSQL 部署指针/环境/版本、浏览器可信与 iframe 替换、固定版本文件和长期缓存验证 |
| 未可信应用不在宿主页执行 | PASS | 默认和未知模式 iframe；平台权限负向、实时 mode 核对、opaque iframe 真实浏览器隔离与 iframe ESM 拒绝 |

## 后端集成覆盖

8 项真实 gate 包含 IPv6 raw TCP/reqwest、双租户管理拒绝、数据库不可变和复合外键、入口/资源/MIME/健康/redirect 故障、v1→v2→回滚的授权与 token 保持、显式模式变更撤销、跨源 CORS，以及 25 个 module/frame 正负场景。七个数据库逐个确认 `assertions_ok=true`、`database_removed=true`、`pool_closed=true`，最终没有残留测试连接。
