# Phase 8 验收记录

日期：2026-10-02。范围：宿主上下文管线、MicroProps 2.0、HTML/ESM 生命周期、Vue/jQuery 示例和卸载清理。

## Self-Check

| Acceptance Criterion | Result | Evidence |
| --- | --- | --- |
| 快速切换组织/应用，不展示旧组织数据、不使用旧 token 发新请求 | PASS | 切换开始清空 profile 并卸载；查询固定 token/epoch/signal；Node 乱序测试及真实浏览器组织、应用导航乱序测试通过。 |
| 续期和权限/菜单/主题/路由变化正确更新 | PASS | 短 TTL 浏览器续期、同数量菜单内容变化、角色/操作权限/独立数据授权、主题和子路由均验证；无 update 时等待旧卸载后重挂。 |
| 重复挂载/卸载无持续资源泄漏 | PASS | 沙箱 20 次循环计时器、RAF、window/document 监听器、STYLE/LINK/SCRIPT 清理；Vue/jQuery 请求与信号清理、真实 vendor 退出无页面错误。 |
| 当前授权应用不可用时退出路由并显示明确状态 | PASS | 应用上下文 401/403/404、签发失败和切换后失去应用授权，真实浏览器均退出 /micro 并显示 Toast。 |

## 验证结果

- Node 24 回归共 31 项通过：上下文运行时与便携 SDK 11 项、沙箱生命周期 8 项、Vue/jQuery 实际脚本 VM 契约 12 项。
- `tests/micro-host-browser.mjs` 的 11 个真实 headless Chromium 场景全部通过，包括真实 Vue/jQuery vendor、异步脚本取消及同入口失败缓存恢复。
- 前端 TypeScript 检查和 Vite 生产构建通过，`git diff --check` 通过。既有较大 bundle 与循环分块警告仍由运行交付阶段处理。

浏览器测试启动临时 Vite，拦截平台 API 并使用合成凭据；它验证真实前端控制流，不替代 Phase 5～7 的 PostgreSQL 授权语义验收。测试不读取 `.env.test`，结束后关闭自建浏览器与 Vite。

本机 IPv4/localhost 的 Node 与浏览器探针仍超时；显式 IPv6 `::1` 加 Chromium 无代理参数可实际访问，测试默认使用该配置。Windows 网络问题仍在 Phase 12 的 Q9 中保留，不能据此宣称生产监听已验收。

## 接入与剩余边界

接入规则见[MicroProps 2.0 契约](./micro-app-context-props.md)。角色与独立数据授权分别展示，业务 API 继续使用服务端校验组件。HTML 沙箱适用于可信代码；异步创建的 document 资源由应用自己清理，MutationObserver 不能完全区分同帧宿主和应用节点。独立版本、资源预检、发布回滚和 iframe 信任协议属于 Phase 9。
