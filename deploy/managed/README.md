# 受管静态镜像部署（Phase 36）

这是显式启用的容器管理模式。默认没有 `DEPLOYMENT_CONFIG_FILE` 时目标列表为空，原来的外部 CI/Compose → URL 登记 → 预检/激活继续工作。当前只提供非 root Nginx 静态镜像模板，不部署业务 API、数据库或迁移。生产接入需另行配置受管目标；本阶段验收范围仅本机隔离 Docker 演练。

## 权限与部署边界

平台 API 不持 Docker socket，不调用 Docker，不接收网络、端口、volume、命令或 shell 参数。独立 `saas-deployment-agent` 从固定 HTTPS 平台 origin 领取任务，只用固定 argv 操作其配置的 Docker daemon/网络和 Nginx。目标与仓库白名单、应用模板和正常 TLS 是独立边界。agent 运行账号具有该 daemon 的控制权，应专用并受到主机隔离，不能视为不具主机权限的沙箱。

管理 API 同时要求平台租户根上下文和独立 `system:deployment:read` / `system:deployment:write` 权限。全新演示库和首次生产管理员初始化提供相应 BUTTON；升级现有安装不自动授权。操作者须通过既有菜单/角色/租户及组织授权 API/UI 显式配置所需权限，组织管理员不因菜单可见而获得部署权。agent 使用按 target 独立的 token 文件，不接受浏览器会话作为 agent 凭据；不要共享不同目标的 token。

## 配置与启动

平台 `DEPLOYMENT_CONFIG_FILE` 指向服务器私有 JSON，参考 [platform.example.json](platform.example.json)。`policy` 会公开给有 read 权限的管理员；`agentTokenRef` 只是服务器绝对文件路径，实际 token 不进入 DB、任务、URL 或前端。token 为 32–512 字节字母/数字/`-_.`，在受限文件中保存，并向对应 agent 提供同一秘密。每次请求重新读取，轮换或删除目标可撤销旧凭据。

agent 的配置参考 [agent.example.json](agent.example.json)。平台和 agent 必须配置相同的完整 `policy`。policy 的 SHA256 绑定到不可变产物和每个任务；改变 origin、白名单或模板会使旧回执不能继续激活。已发布版本需要变更策略时使用新的受管目标/版本并重新验收，不覆写旧绑定。CA/秘密文件、state 和 route 目录由操作者预置；示例 `.invalid` 不能直接运行。

```sh
cargo build --locked --manifest-path tools/deployment-agent/Cargo.toml
saas-deployment-agent /secure/agent.json --once
# 显式常驻轮询；没有安装 systemd/timer 或连接生产目标
saas-deployment-agent /secure/agent.json --watch
```

只有 agent 主机安装 Docker CLI；平台容器不需要它。agent 使用 `/var/run/docker.sock`，目标网络与 Nginx 容器必须都带 `saas.deployment.owner=<agent配置owner>` 标签。route 目录由 agent 写，Nginx 只读挂载为 `/routes`，在对应 HTTPS server 中 `include /routes/*.conf;`。未知 `/sub-apps/` 路径保持 404。目录/秘密权限必须按服务账号限定；不要把真实秘密文件放入仓库或示例中。

固定模板为 UID/GID `101:101`、入口 `nginx -g 'daemon off;'`、只读根、丢弃全部 capability、no-new-privileges、无 bind/volume/宿主端口，固定 `/healthz`、128 MiB、1 CPU、64 pids、16 MiB `/tmp` 和有界容器日志。镜像必须与此模板兼容。业务 API 和任意 shell 不在模板范围。

## 状态、回执和未知结果

1. 在原发布面板登记 HTML 版本，入口精确为 `<target.publicOrigin>/sub-apps/<applicationCode>/versions/<version>/`；资源 URL 沿用版本化不可变路径。
2. 使用受管部署表单选择已登记版本/目标/模板，提交 `repository@sha256:<64小写hex>`。新 `/managed-deployments` API 保存独立 metadata/job/binding，不向旧 RegisterBody 添加镜像字段。
3. agent 领取 QUEUED 或过期 RUNNING 工作，获得唯一 attempt/nonce 和 10 分钟租约。任务不携带运行命令或秘密。Docker pull 后核对实际 RepoDigests、镜像 ID、容器身份和固定限制，再执行容器内固定 HTTP 健康检查。
4. 健康通过后安装固定版本路由，`nginx -t` 后 reload；失败恢复之前的路由文件，旧 active 指针不变。路由更新用同一目标的文件锁串行执行；崩溃后 OS 释放锁。
5. 回执必须来自对应 target 的 HTTPS Bearer，绑定 nonce/attempt/策略、派生容器名、镜像身份和路由摘要。平台事务保存状态与审计；过期/错目标/不同内容的重复回执拒绝。同一完成回执可重交。
6. READY 不等于激活。操作者再执行公开 URL 预检/激活；激活和回滚在指针事务内检查受管绑定、READY 回执、当前策略及未清理状态。HTTP 200/PASSED 单独不能激活受管 URL。

相同 requestId/相同正文重用同一 job，改变正文冲突。前端丢失创建响应后保留原 requestId；请读回任务状态。执行后回执丢失时，租约过期可重领相同不可变意图；固定容器身份和路由字节使重试复用已安装资源。不能在租约仍在途时强行创建新工作或更换版本产物。

Docker CLI 每次操作有截止时间。worker 的失败回执只含固定错误码，不含原始 Docker/HTTP 输出。若进程/主机在路由或删除途中崩溃，任务不会被推定完成；保留资源与状态，重试同意图并重新核对。失败清理的 `CLEANING` 状态会阻止新激活，需先消除引用/配置问题并重试清理，不能强制忽略状态。

## 回滚与清理

公开版本路由和容器按应用/版本/摘要稳定命名，不按 active 指针替换，因此旧版本继续在线。回滚沿用平台原有重预检和 active/previous 事务。版本跨环境使用同一字节产物；相同版本/target 不允许不同镜像或策略。

清理是单独的显式任务。提交和领取/完成时检查 active/previous、其他环境引用、平台在途发布操作及 agent 在途任务；曾 READY 的产物还受至少 30 天与最新 5 个候选保护。拒绝时不删除容器或路由。受管判定先按配置网关解析相对入口，并按 Nginx 语义归一百分号编码、重复分隔符和点路径，别名不能绕过回执门禁。跨环境保留按应用与版本保守保护，覆盖 URL 别名。CLEANING 状态还会在同一应用事务锁内拒绝创建或采用新的发布引用，封住领取任务到引擎完成之间的新环境登记窗口。只清理该受控产物的精确路由和 owner/target/app/version/image 全匹配的派生容器，不删除镜像、共享网络/卷、任意主机文件或 DB。旧环境 DRAFT 引用也会保守保护资源；没有自动清理调度或强制跳过功能。

## 隔离验收

```sh
docker build --target browsers --build-arg TARGETARCH=amd64 -f deploy/ci/Dockerfile -t saas-ci:phase04 .
docker pull postgres@sha256:45cd22f8d32e189d245403954882f88e7a8714301fda80dab6da90f1265b25a3
docker pull nginxinc/nginx-unprivileged:1.28.0-alpine
docker pull registry:3.0.0
node scripts/ci/managed-deployment.mjs --out-dir /absolute/new/report
```

受管部署和 ZIP 分发演练均默认使用上述工具标签与 PostgreSQL 固定 digest；可通过 `CI_TOOLCHAIN_IMAGE` 和 `CI_POSTGRES_IMAGE` 选择已准备的镜像。启动前检查 Linux amd64 平台并解析实际不可变 image ID，随后只按该 ID 创建容器，报告的 `inputImages` 保存请求引用及解析身份。无需复用某台开发机的本地镜像 ID。

该门禁需要 Linux Docker 或 Windows/WSL Ubuntu、本地固定 CI 工具镜像、PostgreSQL17、Nginx 和 registry:3。它创建独立数据库/本地 TLS registry/短期 CA/固定目标，只监听 loopback。Windows 仅为唯一 registry 端点临时配置 Docker CA，按 owner 核对后删除；不改系统全局 CA、不重启 daemon。平台/浏览器工具不挂 Docker socket，agent 在受控本机 Linux 进程执行。

门禁真实运行坏镜像、不健康、路由失败、回执丢失、过期回执、幂等重试、旧 URL/回滚、引用保护、安全清理、审计失败和三产品管理 UI。最终 PASS 由 closed typed validator 检查，编译-only、跳过、源变化、TLS 忽略和清理失败不能采纳。报告含源码摘要、原始 ELF/镜像归档与 SHA256、实际进程退出和 owner 清理；不等于已接入生产受管目标。
