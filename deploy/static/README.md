# 宿主静态镜像与旧 hash 资源保留

从仓库根目录构建，基础镜像版本固定，Node/npm 仅在构建阶段使用。运行阶段是 `nginxinc/nginx-unprivileged:1.28.0-alpine`，UID/GID `101:101`、端口 `8080`、PID 和临时文件在 `/tmp`。镜像直接执行 `nginx`，不会运行修改只读 `/etc/nginx` 的入口脚本。发行时还应记录构建基础镜像与最终镜像 digest；版本标签不具有 digest 的不可变保证。

```sh
docker build -f frontend/Dockerfile \
  --build-arg SOURCE_REVISION="$(git rev-parse HEAD)" \
  --build-arg HOST_BUILD_ID=host-20261015-01 \
  -t registry.example.com/platform/host:host-20261015-01 .

docker run --rm --read-only --user 101:101 \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m,mode=1777 \
  --cap-drop ALL --security-opt no-new-privileges:true \
  --network platform-edge \
  registry.example.com/platform/host:host-20261015-01
```

该命令不发布宿主容器端口。独立入口 Nginx 通过 Docker 私网访问 `frontend-web:8080`，只有入口发布 `80/443`。`/healthz` 返回健康信号。语法检查可使用 `docker run --rm --read-only --tmpfs /tmp:rw,mode=1777 IMAGE -t`；真实 UID、只读启动、HTTP、HTTPS 和代理验证仍须在 Linux/Docker 环境执行，配置文本检查不算通过。

宿主镜像只处理静态文件与 `frontend/src/App.tsx` 对应的导航前缀。导航 fallback 要求 GET/HEAD、明确接受 `text/html` 且没有 `q=0`，API、未知路径、文件型路径、Vite 开发路径以及所有 `/sub-apps` 均返回真实 404。`/assets/*` 缺文件不会返回 HTML。HTML 为 `no-cache`，assets 为 `immutable`，错误为 `no-store`。宿主有禁止嵌入 CSP；已登记微应用的匿名 CORS 与可嵌入 CSP 由独立版本静态服务提供，入口透传。开发环境仍可使用未版本化的兼容示例；生产镜像须以明确版本 URL 在入口登记。

## 保留旧页面正在使用的资源

Phase29将缺失 `/assets/*` 的响应固定为 `404 text/plain`、`Cache-Control: no-store`；命名错误位置清空扩展名MIME映射，避免缺失JS/CSS继承脚本/样式类型。前端显示局部恢复入口，由用户明确刷新；没有自动刷新循环。这不代替下方旧hash保留要求，已有旧资源仍应返回原字节。实测与回退见[Phase29](../../docs/validation/phase-29.md)。

每个镜像的 `/usr/share/nginx/html/release-metadata.json` 记录 `buildId`、可选完整 Git revision、前端源码内容 SHA-256、全部 assets 的路径/长度/摘要，以及保留版本的固定镜像 digest 与原 metadata 摘要。未传 `SOURCE_REVISION` 的本地构建明确记录 `null`，内容摘要仍可核对；发行应传完整 SHA。此文件是公开的构建信息，只含公开标识，不得写入秘密。

`currentAssets` 单独记录本次构建在导入历史前生成的资源，`assets` 记录合并后实际交付的完整资源。发布验收应从旧 `index.html` 取实际入口 URL，确认它在新版本 `currentAssets` 中不存在、在新版本 `assets` 中存在，再实际请求并核对字节；这样可证明旧入口由保留机制提供。此字段是 schema 1 的兼容扩展，导出工具仍接受没有 `currentAssets` 的旧 metadata。

首个发行允许 `deploy/static/history/` 仅有 `.gitkeep`。之后每次发行与回退都须从**当前在线镜像的固定 digest**提取其全部 assets，再构建候选镜像。脚本只创建停止状态的导出容器，以 `--network none` 禁用其网络，不执行容器内程序。

```sh
# 输出目录不得已有 assets 或 manifest.json，避免悄悄覆盖历史。
# 如已有历史，先将其归档到构建上下文外，再使用新的空 history 目录。
node deploy/static/prepare-history.mjs \
  --image registry.example.com/platform/host@sha256:填写实际64位摘要 \
  --output deploy/static/history

docker build -f frontend/Dockerfile \
  --build-arg SOURCE_REVISION="$(git rev-parse HEAD)" \
  --build-arg HOST_BUILD_ID=host-20261015-02 \
  -t registry.example.com/platform/host:host-20261015-02 .
```

脚本拒绝 tag/`latest`、不匹配的文件/摘要、路径穿越、符号链接和重复资产。构建再次校验历史包，遇到相同 URL 不同内容时失败，然后把旧 assets 合并到新镜像中。新 metadata 保留完整来源链；运行时无需共享目录、`MICRO_APPS_DIR` 或 Docker socket。历史包仅在受控发行构建机上使用，不公开，不提交 Git。CI 应归档 manifest 和最终镜像 digest。首次从旧无 metadata 镜像迁入时脚本会拒绝，必须先基于可核实的原构建资产建立受审查的 metadata，不能伪造摘要跳过检查。

本地离线验收也可以使用完整 Docker image ID，无需创建 registry 或推送镜像。操作人先从已构建的旧镜像取得完整 `sha256:<64位>` ID，再显式传给 `--image-id`：

```sh
docker image inspect --format '{{.Id}}' host-local:previous
node deploy/static/prepare-history.mjs \
  --image-id sha256:填写实际64位本地imageID \
  --output deploy/static/history
```

离线模式再次执行 `docker image inspect` 并核对 `.Id` 完全相等，然后按该不可变 ID 创建停止状态的导出容器，不执行 pull。元数据标明 `identityKind: docker-image-id`；推荐发行模式标明 `registry-digest`。本地 ID 只证明当前 Docker daemon 内的镜像对象身份，不提供跨机器可拉取能力，发行仍优先使用 registry digest。离线模式同样校验原 metadata 和全部 assets 摘要，且拒绝短 ID、tag 和不匹配的 inspect 结果。

该基线**不自动删除历史 assets**，自然满足保留窗口内旧 URL 在线；容量及有保护条件的清理由后续保留政策阶段处理。每次提取当前在线镜像的合并 assets，可把更早页面所需文件带入下一镜像，仅在 registry 保留旧镜像并不足够。

回退时，直接切换到更旧镜像会丢失新页面已经引用的新 chunk。因此应以目标回退源码重新构建一个宿主候选，同时导入当前在线镜像的历史 assets，再完成检查与切换。若必须直接回切旧 digest，应提前准备并验证同时包含新/旧 assets 的回退候选，不能以刷新页面代替保留。在变更入口 upstream 前实际请求旧页面引用的每个 hash 文件，核对 HTTP 200、MIME 和内容摘要，并确保未知资源仍为 404。

本目录的 Node 测试验证真实文件合并、摘要冲突及导出工具控制流；它们不证明 Docker 镜像已运行。运行：

```sh
node --test deploy/static/tests/static-image-policy.mjs
```
