# 受管静态镜像部署代理

`saas-deployment-agent` 是独立 Rust worker，经平台 HTTPS 领取固定目标任务，在受控 Docker 环境运行批准的静态镜像并维护版本路由。平台 API 不持 Docker 权限，worker 不读取平台数据库，不执行任务提供的任意命令。镜像内应为可供 qiankun 加载的 HTML/UMD 静态产物。

新任务的 `jobId`、`releaseId`、`nonce` 必须为 UUID v7／RFC variant，旧 v4、nil 和非法值在任何引擎或文件系统操作前拒绝。UUID 标识不代替 attempt、nonce 回执比对、策略 revision 或摘要检查，也不作为安全 secret。

## 构建与运行

在本目录执行：

```sh
cargo test --locked
cargo build --release --locked
saas-deployment-agent /srv/saas/private/agent.json --once
saas-deployment-agent /srv/saas/private/agent.json --watch
```

运行命令假定二进制已放入 `PATH`，也可使用本工程 `target/release` 下二进制的绝对路径。`--once` 处理一轮领取，`--watch` 持续轮询。运行目标需要 Linux、Docker、已有私有网络、受控 Nginx 容器以及专属可写路由/状态目录。仓库不自动安装服务或选择生产目标。

## Worker 私有配置

配置拒绝未知字段，路径必须符合实际运行环境：

```json
{
  "policy": {
    "id": "static-local",
    "name": "门店静态镜像目标",
    "publicOrigin": "https://platform.example.com",
    "repositories": ["registry.example.com/apps/stores"],
    "templates": [{"id":"stores-static","applicationCode":"stores","healthPath":"/healthz"}]
  },
  "platformOrigin": "https://platform.example.com",
  "caFile": "/srv/saas/private/platform-ca.pem",
  "tokenFile": "/srv/saas/private/agent.token",
  "network": "platform-edge",
  "gatewayContainer": "platform-gateway",
  "routeDirectory": "/srv/saas/routes",
  "stateDirectory": "/srv/saas/agent-state",
  "owner": "static-local"
}
```

`platformOrigin` 和 `publicOrigin` 为准确 HTTPS 来源。镜像只接受策略白名单仓库的 `repository@sha256:<64位小写摘要>`，模板健康路径固定 `/healthz`。CA/token 及路由/状态目录预先存在，使用绝对、非符号链接路径；token 为 32–512 个 ASCII 字母、数字或 `-_.` 字符。CA 文件须包含可读取的 PEM 证书，token 与状态不放公开静态目录。

## 平台与网关配套

平台设置 `DEPLOYMENT_CONFIG_FILE`，顶层 `targets` 的每项包含同一份 `policy` 和平台可读的绝对 `agentTokenRef`。每个目标使用独立凭据。默认 Compose 未传入该变量或挂载文件，需要明确 overlay。平台管理员还需满足受管部署权限和目标策略。

worker 创建的容器与 Nginx 必须在 `network` 所指同一网络。`routeDirectory` 是宿主目录；Nginx 需将其挂载到自身可读目录，并在对应 HTTPS `server` 中 include 其 `*.conf`。基础 Compose 仅挂配置和 TLS，网关渲染器也不生成这一 include，必须额外完成。`gatewayContainer` 应填写真实容器名，不能依赖 Compose 服务名恰好等于容器名。

先验证网关能解析新容器 DNS，include 生效且没有同前缀手写 `versionRoutes`。仅 `nginx -t`/reload 成功不能证明新路由被加载。平台随后还会检查真实 URL；上线必须验证浏览器取得正确 HTML/JS。

## 执行与回执

任务绑定目标策略摘要、应用、版本、镜像、attempt 和 nonce。worker 使用固定 Docker 参数，不接受任意端口、网络、挂载或 shell；先检查镜像和健康，再安装 `/sub-apps/<app>/versions/<version>/` 路由并回执。真实路径由固定协议派生，不能与 ZIP 分发重复管理同一应用和公开来源。

新路由以完整身份摘要命名，旧格式仅在文件字节精确一致时复用。未知文件、容器或所有权冲突保持失败供调查，不直接覆盖。回执未知时查询平台原任务和本地状态，不能另造任务重复操作。

清理任务先经过平台保护检查，worker 只移除所属且身份一致的路由和容器。`CLEANING` 围栏在失败或结果未知时保持，禁止借旧部署重试重新安装。保留与回退流程见[升级与保留](../../docs/upgrade-and-retention.md)。

## 交付验收

在目标环境验证 registry 信任与认证、平台 TLS、私网 DNS、Docker 权限、路由挂载/include、健康检查、真实不可变 URL、最终宿主 CSP、qiankun 生命周期和旧版本访问。worker 的隔离测试不证明某个真实生产目标已部署；监督运行、日志、凭据轮换、备份和恢复由运维明确配置。
