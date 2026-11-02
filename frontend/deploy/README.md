# 宿主静态交付

本目录是 `frontend/Dockerfile` 的静态运行配置与构建辅助工具。运行镜像只提供宿主 `dist/`，微应用入口、版本选择、业务 API 和 URL 资源代理由边缘网关及 Rust 后端负责。完整生产拓扑与配置见[部署指南](../../backend/docs/deployment.md)。

## 构建与运行

以下命令均在 `frontend` 目录执行。使用真实 Git revision 和合法构建标识：

```powershell
$sourceRevision = git rev-parse HEAD
$hostBuildId = 'host-local-20261023'
docker build --build-arg "SOURCE_REVISION=$sourceRevision" --build-arg "HOST_BUILD_ID=$hostBuildId" -t saas-platform-web:local .
docker run --rm --read-only --tmpfs /tmp:rw,noexec,nosuid,size=32m -p 127.0.0.1:8080:8080 saas-platform-web:local
```

构建阶段使用 Node 24.22.1、npm 11.16.0，安装锁定依赖且禁用安装脚本，再校验 SDK、运行 TypeScript/Vite 构建、qiankun 交付检查和 `finalize-build.mjs`。交付检查会拒绝宿主 public 或 dist 中夹带微应用副本、旧桥接代码或非宿主 HTML 的 module 脚本。运行阶段使用 `nginxinc/nginx-unprivileged:1.28.0-alpine`，UID/GID 为 `101:101`，端口 8080。PID、请求临时文件和代理临时目录均在 `/tmp`，只读根文件系统需要可写 `/tmp`。镜像包含健康检查，访问 `http://127.0.0.1:8080/healthz` 返回 `ok`。

`SOURCE_REVISION` 为空时元数据记录 `null`；提供时必须是完整的 40～64 位十六进制 revision。`HOST_BUILD_ID` 允许字母、数字、点、下划线、连字符，最长 128 字符，未提供时从源码摘要生成。

## 路由与 qiankun 的部署分工

| 请求 | 宿主镜像行为 | 正式入口责任方 |
| --- | --- | --- |
| `/assets/*` | 只返回现存文件；缺失返回文本 404 | 宿主静态服务 |
| `/micro/:appCode/*` | 合法 HTML 导航回退到宿主 `index.html` | 宿主 React Router 与 qiankun |
| `/sub-apps/*` | 固定返回 404 | 网关的已登记版本静态路由 |
| `/micro-proxy/*` | 固定返回 404 | 网关转发至 Rust URL 资源代理 |
| `/api/*`、`/auth/*`、业务 API、`/socket.io` | 不提供服务，不回退 HTML | 网关及对应后端 |

`nginx.conf` 只对明确的宿主导航路径、GET/HEAD 方法和允许 HTML 的 Accept 开启 SPA fallback，缺失脚本、样式或 API 不会伪装成成功页面。宿主的 `/micro/*` 导航与 `/micro-proxy/*` 资源代理必须分别配置，不能将后者回退至 React 页面。

qiankun 在宿主 DOM 内挂载 HTML 应用，不需要把宿主变成可嵌入页面。宿主响应设置 `X-Frame-Options: DENY`、`frame-ancestors 'none'`、`object-src 'none'` 和 `base-uri 'self'`。这不妨碍宿主 `/embed` 页面加载菜单外链 iframe。

## 缓存与发布元数据

HTML 和一般静态文件使用 `no-cache`，`/assets/*` 成功响应使用一年 `immutable`；健康检查、发布元数据和错误响应使用 `no-store`。

`finalize-build.mjs` 在 `dist/release-metadata.json` 中写入：

- `sourceSha256`、可选 `sourceRevision` 和 `buildId`；
- 本次构建的 `currentAssets`；
- 合并历史后的完整 `assets`，每项包含路径、长度、SHA-256；
- `retainedReleases`，记录历史镜像身份和元数据摘要。

该文件公开可读，只用于产物追踪，不得加入凭据。源码摘要覆盖脚本中列明的源码、public、包锁与构建输入，不应将其当作整个 Git 仓库的摘要。

## 保留旧 hash 资源

已打开的页面可能继续请求旧 hash chunk。后续发布以及回退候选都应导入当前在线版本的全部 assets，单纯保留旧镜像不能让旧 URL 继续可用。首次构建允许没有 `deploy/history`。

从当前在线镜像的不可变 registry digest 提取历史：

```powershell
# 先将以下变量设为当前在线镜像的真实不可变引用。
$onlineImage = 'registry.example.com/platform/host@sha256:<实际64位摘要>'
node deploy/prepare-history.mjs --image $onlineImage --output deploy/history
docker build --build-arg "SOURCE_REVISION=$sourceRevision" --build-arg "HOST_BUILD_ID=$hostBuildId" -t saas-platform-web:candidate .
```

输出目录若已有 `assets` 或 `manifest.json`，工具会拒绝覆盖。将现有历史明确归档到构建上下文之外，再提取新历史。离线环境可以改用完整本地镜像 ID：

```powershell
$localImageId = docker image inspect --format '{{.Id}}' saas-platform-web:local
node deploy/prepare-history.mjs --image-id $localImageId --output deploy/history
```

`--image` 与 `--image-id` 只能选择一个。前者会拉取不可变镜像；后者检查完整本地 ID 并且不拉取。工具创建禁用网络的停止容器、导出 assets 和元数据，验证后写出 manifest，最后移除导出容器，不启动应用。

构建会拒绝符号链接、路径遍历、文件集合/摘要不符及同一资源 URL 内容变化，再合并历史文件。工具不自动决定历史保留期限，不自动清理历史文件。

## 验证入口

```powershell
npm run test:static
```

该入口检查资源保留、元数据、篡改拒绝、导出控制流和静态配置策略，属于 Node 文件/fixture 测试。真实镜像构建、Nginx 路由、TLS、微应用 URL 代理、版本激活和业务连通性需要在部署环境另行验收；`npm run build` 也不会替代这些检查。
