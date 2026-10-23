# ZIP 静态节点代理

`saas-artifact-agent` 将平台已经安全解包的规范文件集合安装到独立节点，并重算真实文件树。它不解压 ZIP、不下载任意 URL、不启动容器，也没有远程删版本接口。qiankun 通过公共静态 URL 加载产物，agent 本身不是浏览器入口。

## 构建和运行

在本目录执行：

```sh
cargo test --locked
cargo build --release --locked
saas-artifact-agent /srv/saas/private/artifact-node.json
```

上面的运行命令假定二进制已放入 `PATH`，也可使用本工程 `target/release` 下二进制的绝对路径。实际节点面向 Linux，以非特权账号运行。`SIGTERM`/Ctrl+C 触发停止。HTTP 只允许绑定回环，节点外部访问由固定 HTTPS 代理提供正常 TLS；每个 `/agent/*` 请求，包括健康检查，都需要节点专用 Bearer。

## 节点私有配置

配置为闭合 camelCase JSON，拒绝未知字段。示例如下，实际值须与平台及同组节点完整策略一致：

```json
{
  "policy": {
    "schemaVersion": 1,
    "id": "zip-local",
    "name": "门店静态节点",
    "publicOrigin": "https://platform.example.com",
    "applications": ["stores"],
    "nodes": [
      {"id":"node1","origin":"https://node1.example.com"},
      {"id":"node2","origin":"https://node2.example.com"}
    ]
  },
  "nodeId": "node1",
  "bind": "127.0.0.1:3009",
  "rootDirectory": "/data/sub-apps",
  "tokenFile": "/srv/saas/private/node1.token"
}
```

节点组包含 2–8 个必需节点。`nodeId` 必须存在于策略；`bind` 必须是非零端口的回环地址。预先创建 `rootDirectory` 并授予服务账号所有权，目录组和其他用户不可写。`tokenFile` 放在静态根之外，Linux 权限应为 `0600`；token 为独立的 32–512 个 ASCII 字母、数字或 `-_.` 字符。

配置、目录、凭据和祖先路径不得为符号链接。节点各自持有独立目录与 token，不共享可写产物卷。token 按请求重读，轮换需同时协调平台引用。

## 节点协议

| 接口 | 用途 |
| --- | --- |
| `GET /agent/health` | 带认证检查进程及节点身份 |
| `POST /agent/install` | 接收固定清单和 base64 内容，安全安装不可变版本 |
| `POST /agent/verify` | 重算完整文件树，拒绝缺失或额外文件，回执绑定 nonce |

协议最多 49 个文件，单文件 4 MiB、解码总计 16 MiB、JSON 24 MiB。先认证后解析，请求期限 45 秒，并发请求槽 2 个；磁盘任务与版本根锁限制并发。安装通过私有暂存与同步后以 Linux 不覆盖 rename 发布。同摘要版本可重验复用，损坏或冲突内容不自动覆盖。

agent 将版本保存为 `rootDirectory/<app>/versions/<version>/`。上述示例的实际目录为 `/data/sub-apps/<app>/versions/<version>/`；仓库 micro-static 会剥离公开 `/sub-apps/` 前缀，所以其 `microRoot` 应为 `/data/sub-apps`。配置公共映射时拒绝 agent 元数据、点文件和暂存目录，静态服务返回真实 MIME 和缺资源 404。

## 配套与故障处理

平台通过 `ZIP_DISTRIBUTION_CONFIG_FILE` 配置同组策略与每节点 CA/token 引用；公共网关使用真实 TLS 与分发就绪门禁。浏览器不直接访问 `/agent/*`。完整操作见[ZIP 分发指南](../../docs/zip-distribution-runbook.md)。

改变策略或拓扑会改变身份，旧任务不能直接借用新配置完成。节点离线、摘要冲突、磁盘损坏或凭据轮换失败时核对原任务、回执和静态目录，不删除旧版本来强行重试。上线前验证所有必需节点独立存储、真实 URL、qiankun 加载、备份和故障恢复。
