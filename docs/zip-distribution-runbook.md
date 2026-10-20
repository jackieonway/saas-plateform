# 本机隔离多节点 ZIP 分发

本实施对应 Phase 35。只验收本机 Docker 网络内的两个逻辑节点，不证明物理跨机、真实生产网络或生产存储可靠性。平台、node1、node2 的发布目录相互独立；agent 不连接 Docker。一次性演练控制器负责创建和清理本地测试资源。

## 方案选择

| 方式 | 优点 | 本阶段取舍 |
|---|---|---|
| 受控对象存储和静态托管 | 可避免节点逐份复制，适合有成熟存储、访问控制、CDN 的环境 | 本机独立节点范围没有该基础设施；可作为未来交付模式，不能假定公共 URL 命中代表多节点到齐 |
| 认证 HTTPS 推送到 Rust agent | 白名单节点、独立本地盘、明确逐节点内容证据 | 本阶段采用；仅发送安全解包后的规范文件和 SHA-256 清单，避免在节点重复实现 ZIP 安全解包 |

## 平台配置

设置 `ZIP_DISTRIBUTION_CONFIG_FILE` 为绝对私有文件路径，未设置时保持原 ZIP 同机模式。闭合结构如下，`policy` 与 agent README 的完整组策略相同：

```json
{"groups":[{"policy":{"schemaVersion":1,"id":"zip-local","name":"隔离双节点","publicOrigin":"https://host.example.test:8443","applications":["stores"],"nodes":[{"id":"node1","origin":"https://node1.example.test:8443"},{"id":"node2","origin":"https://node2.example.test:8443"}]},"nodes":[{"nodeId":"node1","caRef":"/private/node-ca.pem","tokenRef":"/private/node1.token"},{"nodeId":"node2","caRef":"/private/node-ca.pem","tokenRef":"/private/node2.token"}]}]}
```

节点数 2–8，所有节点均必需；无可选节点。publicOrigin 必须等于平台 MICRO_GATEWAY_ORIGIN。节点 origin 必须是 HTTPS、无路径/query/凭据且各自不同。平台只向白名单固定 `/agent/verify` 和 `/agent/install` 推送，不使用请求携带的 URL。平台校验 CA、证书主机名、不跟随重定向、不使用代理；每个节点 token 必须不同。镜像与 ZIP 不能管理同 origin 下的同一应用。

数据库按既有迁移流程应用 `0006_zip_distribution.sql`。新安装/演练 bootstrap 添加 `system:distribution:read` 和 `system:distribution:write`，已有安装需要管理员显式授予菜单、组织及角色权限；不自动扩权。只有平台根管理上下文可调用管理 API。

## 网关与节点

节点 agent 配置见 `tools/artifact-agent/README.md`。节点只提供自身本地盘中的固定 `/sub-apps/{code}/versions/{version}/`。HTTPS proxy 转发 `/agent/` 到 loopback；禁止点文件/私有暂存和元数据。节点服务只允许平台和公共网关访问。

公共网关部署 `deploy/gateway/zip-distribution.conf.example` 的 `auth_request` 门禁和固定 TLS 节点组。传入原始 `$request_uri`，不能重复解码 `$uri`。需开启 Nginx auth_request 模块。该配置替换 ZIP 静态路由，保留已有镜像路由及应用 CSP/隔离头。启动时先 `nginx -t`，再由运维显式 reload；平台和 agent 不改网关配置。

## 操作与恢复

1. 上传 ZIP。平台仍只安全解包一次，绑定不可变摘要。受管组中的版本返回 DRAFT，暂缓公共探测。
2. 发布页面的“ZIP节点分发”选择已上传版本和白名单节点组，提交固定 requestId 的分发意图。响应丢失后重提同一意图复用原任务。
3. 后台任务有持久化 attempt/nonce/lease。每次先核验现有文件，只有明确缺失才安装；各节点暂存、校验、同步、原子安装后保存可信回执。
4. 全部必需节点成功后为 READY。平台预检/激活/回滚再次逐节点重算文件，并逐文件访问各节点 HTTPS 静态路径，再运行公共预检；指针事务再次核对策略、摘要及新鲜回执。
5. 离线、缺文件、错摘要、错误 CA/主机名/token 或回执不匹配会阻止切换。失败任务可重试，正常节点被核验复用，异常已存在目录不能被覆盖。存储损坏须排查后由运维恢复，不以自动覆盖掩盖问题。
6. lease 超时可重新领取，旧 attempt/nonce 不能提交完成状态。关键审计与任务/回执写入同事务，审计失败不会部分提交。
7. 配置删除或拓扑变化不能解除历史受管候选的门禁；URL 编码/相对路径/其他环境不能借用别的发布回执。曾就绪的 active/previous 保留旧资源可访问，但新的切换仍须重新核验。当前阶段不自动清理节点旧版本。

## 演练

`node scripts/ci/zip-distribution.mjs --out-dir <新的绝对输出目录>` 在明确本机 Docker 测试上下文中执行。需要已安装的固定工具/PG/Nginx 镜像及 Node 24；Windows 通过 WSL Ubuntu Docker。报告记录当前源码清单/摘要、Linux ELF、真实 TLS/API/UI、逐节点故障和清理。任何必需检查失败都不能采纳为验收通过。
