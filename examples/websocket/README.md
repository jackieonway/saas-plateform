# Socket.IO 聊天示例

本目录提供可以独立安装和构建的 React / TypeScript / Vite 前端，以及 [Rust 聊天参考后台](backend/)。前端不依赖宿主的 `@/components` 或 `node_modules`。Vite 自带 TSX 转换，无需增加 React 构建插件。

## 安装、开发和预览

工具基线：Node **24.18.0**、npm **11.16.0**；使用本目录提交的 `package-lock.json`。

```sh
cd examples/websocket
npm ci
npm run dev
```

开发地址为 `http://127.0.0.1:3007`，端口被占用时明确失败。开发进程只启动 Vite。退出使用 `Ctrl+C`；根调度脚本也可受控启动和停止此前端。

```sh
npm run typecheck
npm run build
npm run preview
npm test
```

`build` 先执行 TypeScript 检查；`preview` 在相同的本机端口预览 `dist/`。`npm test` 先构建，再通过 Node 测试驱动验证默认离线、配置拒绝、Vite dev/preview HTTP、版本前缀相对资源、真实 404 和哨兵秘密不进入浏览器产物。测试只监听随机回环端口，不启动聊天后台、不连接平台或数据库。

已具备 Playwright 和 Chromium/Edge 的隔离验收环境还可设置 `PLAYWRIGHT_MODULE`（已安装模块路径）及 `BROWSER_EXECUTABLE`（浏览器可执行文件），执行 `npm run test:browser`。此测试通过浏览器拦截模拟 Engine.IO/Socket.IO 帧，实际操作 Connect、Join、发送、断开和重新加入；不会访问示例域名或运行聊天后台。浏览器测试驱动由验收环境提供，不参与本工程安装、打包或运行依赖。

## 受控聊天连接

默认构建不配置聊天服务，界面保持离线。先从仓库根执行 `npm run dev:chat:backend`，再为此前端设置公开的 `VITE_CHAT_ENDPOINT=http://127.0.0.1:3008`。该地址会公开写入浏览器代码，必须是不带用户名、密码、查询参数、片段和路径的 HTTP(S) origin。实际部署使用 HTTPS，避免混合内容；勿配置平台通知服务或共享敏感会话的宿主 origin。

PowerShell 示例：

```powershell
$env:VITE_CHAT_ENDPOINT = 'http://127.0.0.1:3008'
npm run dev
```

POSIX shell 示例：

```sh
VITE_CHAT_ENDPOINT=http://127.0.0.1:3008 npm run dev
```

配置后仍需点击 **Connect** 才会联网，再输入用户名并点击 **Join Chat**。断开时清除加入状态与在线用户，重连后需要重新加入。客户端不传入鉴权数据或宿主 session，`autoConnect` 与 `withCredentials` 均为 `false`。Vite 不读取 `.env` 文件，也不批量注入 `VITE_*`；仅显式发布经过校验的公开 endpoint。错误信息不会回显无效配置值。

## 协议与阶段边界

本页面使用 **Socket.IO 4**。Rust 参考保留 Engine.IO `path: '/'` 和以下事件：

| 方向 | 事件 | 内容 |
| --- | --- | --- |
| 客户端 → 服务端 | `join` | `{ username }` |
| 客户端 → 服务端 | `message` | `{ content, username }` |
| 服务端 → 客户端 | `message` | 聊天或系统消息 |
| 服务端 → 客户端 | `user-joined` / `user-left` | `{ user, message }` |
| 服务端 → 客户端 | `users-list` | `{ users }` |

聊天与平台通知具有不同的业务协议：通知使用 `/socket.io/`、平台会话事件认证与组织上下文。Phase 22 已用 [真实客户端矩阵](../../tests/phase22-socketio.mjs)分别验证聊天参考及通知协议候选；通知选型见 [实时协议决策](../../docs/realtime-protocol.md)，实际结果见 [Phase 22 证据](../../docs/validation/phase-22.md)。旧 Node `server.ts` 已删除，生产通知的迁移仍属于 Phase 23。

Rust 后台使用独立 `Cargo.toml` / `Cargo.lock`，不依赖平台 crate 或数据库。默认只监听 `127.0.0.1:3008`；`CHAT_BIND_ADDR` 可改为其它回环端口（测试用 `127.0.0.1:0`）。`CHAT_ALLOWED_ORIGINS` 是逗号分隔的精确 HTTP(S) origin，默认允许 `http://127.0.0.1:3007,http://localhost:3007`；polling 和 WebSocket Upgrade 均校验。无 Origin 的非浏览器测试驱动允许连接，Origin 校验不提供用户认证。此参考没有账号认证、平台会话或持久化能力。

后台 stdin EOF 或 Ctrl+C 时关闭 Socket.IO 和 HTTP。每次连接先 join 才能发送，用户名与该连接绑定；同名重复 join 只返回列表，改名需要重新连接。聊天室最多 256 名成员，用户名 1–64 字符/256 字节，消息 1–4096 字符/16384 字节；事件必须是严格 JSON 对象，非法输入返回 `error { code, message }`，不回显原始输入。保留 `test` → `test-response { message, data, timestamp }` 扩展；这是聊天协议测试能力，不接入通知发布。

从仓库根运行 `npm run test:realtime` 会编译 Rust 测试 peer，使用宿主锁定的 Socket.IO 客户端和当前 `createChatSocket` 进行真实 TCP 协议测试。它要求 Node 24.18.0、Rust 1.97.0 和可用的回环网络；Windows 安全软件阻止 Rust 入站时应改用受控 Linux 环境，不能据此选择原生 WS。`npm test` 仍只验证静态前端。

## 静态产物与发布

产物为 `dist/index.html` 和 `dist/assets/*`；资源使用 `./assets/...`，可以整体放在固定版本 URL（例如 `/sub-apps/chat/versions/<version>/`）下。保持末尾 `/`，或由静态服务器为目录 URL 重定向；不得将缺失资源回退到 HTML。Vite dev/preview 使用多页面模式，未知路径返回真实 404。开发服务也拒绝读取 `.env` 与旧 `server.ts`；生产只发布 `dist/`。

这些静态产物不包含聊天后台或数据库迁移；Vite dev 显式拒绝 `/backend/**`。Rust 参考独立启动，Vite dev/preview 是本地工具；生产静态 Nginx、镜像和入口代理由后续阶段交付。

## 依赖说明

- `react` / `react-dom`：保留现有 React 聊天页面与实际浏览器挂载。
- `socket.io-client`：保留既有聊天事件与 Engine.IO 传输行为，使用独立锁定的客户端。
- `vite` / `typescript` / `@types/*`：仅用于开发、静态打包与类型检查，不是生产后台依赖。

Rust 使用 MIT 的 `socketioxide 0.18.7` / `engineioxide 0.17.7` 实现现有 Socket.IO 协议，Axum/Tokio 提供 HTTP 与异步运行时，serde/serde_json 提供 DTO，chrono 仅生成协议要求的 RFC3339 时间。前端直接依赖保持原锁；Phase 2 的 npm 审计保留其历史语义。回退 Rust 参考只需恢复本目录与锁文件，不涉及平台、通知服务或数据库。
