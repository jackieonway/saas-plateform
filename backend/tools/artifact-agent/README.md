# ZIP 静态节点代理

saas-artifact-agent 是独立 Linux 服务，将平台已安全解包的规范文件集合安装到一个节点。它不解压 ZIP、不下载任意 URL、不运行容器，也没有远程删版本接口。

## 开发与部署

本目录执行 cargo test --locked、cargo build --release --locked。使用非特权服务账号运行：

```sh
saas-artifact-agent /srv/saas/private/artifact-node.json
```

SIGTERM 停止。HTTP 只绑定回环，由本节点固定 HTTPS 代理提供正常 TLS；每次 RPC 还要求节点专用 Bearer，安装与校验均不匿名开放。

## 节点配置

```json
{
  "policy": {
    "schemaVersion": 1, "id": "zip-local", "name": "隔离节点组",
    "publicOrigin": "https://platform.example.com", "applications": ["stores"],
    "nodes": [
      {"id":"node1","origin":"https://node1.example.com"},
      {"id":"node2","origin":"https://node2.example.com"}
    ]
  },
  "nodeId": "node1", "bind": "127.0.0.1:3009",
  "rootDirectory": "/data/sub-apps", "tokenFile": "/srv/saas/private/node1.token"
}
```

策略在平台与所有节点一致，每节点有独立 token 和存储。预建 rootDirectory，服务账号拥有，权限 0755 或更严且组/其他用户不可写；token 放静态目录外，权限 0600，32–512 个 URL 安全 ASCII 字符。路径及祖先不得为符号链接，秘密按请求重读以支持协调轮换。

## 安装和验证

POST /agent/install 接受固定清单与 base64 集合，最多 49 文件、单文件 4 MiB、解码总计 16 MiB、JSON 24 MiB。POST /agent/verify 重新计算真实完整文件树，拒绝缺失或额外文件，回执绑定 nonce。

先认证后解析；请求期限 45 秒，并发槽 2 个，根锁串行安装/验证。0700 暂存经同步后以 RENAME_NOREPLACE 发布。同摘要已存在版本可重验复用，损坏/冲突版本不自动覆盖。

## 入口和恢复

节点 HTTPS 仅向平台与公共网关网络开放；静态代理拒绝点文件、暂存与元数据。本节点静态服务和代理可共享本地根，不与其他节点或平台共享目录。

更改拓扑会改变策略身份，使旧候选任务不能直接复用。节点离线、磁盘损坏、摘要冲突由运维调查，不能通过删除旧版本让重试强行成功。平台配置与发布门禁见 [ZIP 分发指南](../../docs/zip-distribution-runbook.md)。
