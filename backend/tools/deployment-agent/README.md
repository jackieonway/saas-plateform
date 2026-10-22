# 静态镜像部署代理

saas-deployment-agent 是独立 Rust worker，通过平台 HTTPS 领取固定目标任务，在受控 Docker 环境运行已批准的静态镜像并维护路由。平台 API 不持 Docker 权限，worker 不查平台数据库，不执行任务提供的任意命令。

## 构建与运行

在本目录执行 cargo build --release --locked、cargo test --locked。Linux 目标环境需 Docker、已有私有网络、受控 Nginx 容器及专属路由/状态目录。

```sh
saas-deployment-agent /srv/saas/private/agent.json --once
saas-deployment-agent /srv/saas/private/agent.json --watch
```

--once 单轮处理，--watch 持续轮询。没有自动安装的服务或默认生产目标。

## 配置

agent.json 采用闭合 camelCase JSON，拒绝未知字段。核心形状如下，实际值必须与平台的目标策略一致：

```json
{
  "policy": {
    "id": "static-local", "name": "静态镜像目标",
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

文件和目录预先创建，使用绝对非符号链接路径，CA/token 可读，路由/状态为专属可写目录。策略及镜像仓库使用契约允许的闭合值，模板健康路径固定 /healthz；精确校验见 src/lib.rs 与 deployment-contract。

平台通过 DEPLOYMENT_CONFIG_FILE 配置 targets，每项包含相同 policy 和 agentTokenRef。当前平台基础 Compose 未默认注入该文件，实际启用需要明确环境与只读文件挂载。平台根管理者还需受管部署权限，不能只运行 worker 就获得目标。

## 交付与恢复

任务绑定目标、应用、版本、镜像身份、attempt 和 nonce，使用固定 Docker 参数，不接受任意端口/网络/卷。镜像就绪和真实静态入口需实际验证后才更新路由；未知回执先核对平台任务、容器与状态。

新路由使用完整身份摘要命名，旧格式仅在内容精确一致时复用。不能删除或覆盖未知文件解决冲突，旧版本与清理需要明确所有权。实际 production 网络、registry、权限与监督运行需单独验收，当前代码的隔离候选结果不等于真实目标已经部署。
