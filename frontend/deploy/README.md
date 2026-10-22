# 宿主静态交付

本目录提供 `frontend/Dockerfile` 使用的 Nginx 配置、MIME 表、构建元数据与历史资源保留工具。在 `frontend` 构建镜像：

```sh
docker build --build-arg SOURCE_REVISION=<完整提交SHA> --build-arg HOST_BUILD_ID=<构建标识> -t saas-platform-web:local .
```

构建阶段安装锁定依赖、校验固定 SDK、构建前端，然后执行 `deploy/finalize-build.mjs`。运行阶段仅包含 Nginx 和静态产物，使用 UID/GID 101、8080 端口；PID 与临时文件位于 `/tmp`。按[部署指南](../../backend/docs/deployment.md)配置入口、只读文件系统及 `/tmp`，镜像本身不发布宿主机端口。

`/healthz` 返回健康状态；合法页面导航使用 SPA fallback，缺失脚本、样式、API 与微应用路径返回真实 404。HTML 使用 `no-cache`，hash 资源使用 `immutable`，错误响应使用 `no-store`。宿主拒绝嵌入；微应用静态路由由独立入口配置。

`dist/release-metadata.json` 记录源码 SHA-256、可选完整 Git revision、构建标识，以及当前和全部保留 assets 的路径、长度和摘要。元数据公开可读，不能包含凭据。未传 `SOURCE_REVISION` 时明确记录 `null`。

首个构建允许 `deploy/history` 为空。后续发布和回退应从当前在线镜像的不可变摘要提取全部旧资源；在 `frontend` 执行：

```sh
node deploy/prepare-history.mjs --image registry.example.com/platform/host@sha256:<实际64位摘要> --output deploy/history
docker build --build-arg SOURCE_REVISION=<完整提交SHA> --build-arg HOST_BUILD_ID=<构建标识> -t saas-platform-web:candidate .
```

输出目录不得已有 `assets` 或 `manifest.json`。旧历史应先归档到构建上下文外；工具拒绝静默覆盖。离线环境可显式传 `--image-id sha256:<完整本地镜像ID>`，工具会核对 Docker inspect 结果且不拉取镜像。导出只创建停止状态、禁用网络的容器，完成后删除该容器。

构建校验保留文件、摘要与路径，拒绝符号链接、遍历路径及同一 URL 内容变化，再将历史 assets 合并进新版本。只保留旧镜像不能保证旧页面引用的资源继续在线；回退候选也应导入当前在线 assets。本工具不自动清理历史文件。

在 `frontend` 执行文件与导出控制流回归：

```sh
node --test deploy/tests/static-image-policy.mjs
```

这些检查不证明真实镜像已构建、Nginx 已运行或生产路由已验证。Docker 与部署环境验收需单独执行。
