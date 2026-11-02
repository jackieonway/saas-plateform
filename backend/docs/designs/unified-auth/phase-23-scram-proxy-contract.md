# Phase 23 固定 SCRAM 路径透明代理

[`phase23-scram-proxy.py`](../../../tests/unified-auth-https/phase23-scram-proxy.py)
只在本次 WSL 任务中透明转发 TCP 字节，不解析 PostgreSQL、判断认证结果、
记录流量或修改 HBA。真实角色/数据库的 SCRAM 规则及 structured SQLSTATE
认证检查由唯一环境协调者负责；经此端口连接成功本身不是密码验证证据。

## 固定输入与启动

```json
{
  "contractVersion": "identity-scram-proxy-input/1",
  "taskDir": "/tmp/rust-saas-unified-auth-20261028",
  "listenPort": 25495,
  "maximumConnections": 8,
  "backlog": 8,
  "maximumBytesPerConnection": 1048576,
  "totalTimeoutMs": 10000,
  "idleTimeoutMs": 3000,
  "chunkBytes": 65536
}
```

只接受这些字段及精确值。没有目标/source/协议/URL/凭据参数；任何额外字段、
更大预算或生产 port=0 均拒绝。生产监听固定 `127.0.0.1:25495`，仅连接
`127.0.0.1:55494`，upstream socket 显式 `local_addr=('127.0.0.2',0)`。

```sh
python3 phase23-scram-proxy.py --config /tmp/rust-saas-unified-auth-20261028/phase23-scram-proxy-input.json
```

由 root 协调者进入现有任务 mount namespace 后，降为 OWNER 的非 root uid
执行。taskDir 必须精确匹配、无 symlink、0700 且归当前 uid；config 必须在
该目录直接子层、普通文件、当前 uid、精确 0600、至多 4096 字节，以目录 fd
anchored `O_NOFOLLOW` 打开。没有 stdin 或环境变量凭据读取。

启动核验 OWNER 标记、namespace.pid 的 pid/startTime/namespace、当前 mount
namespace、namespace.identity 及实际控制器 cmdline 的任务脚本绑定。common
控制器的两个租约文件允许 root 拥有的 0644，但不得组/其他可写；此兼容不
放宽 config 和 proxy PID 文件的 0600。每 100ms 复查相同租约内容和实际 PID
启动时间；租约消失或变化即结束全部连接和 listener。

独占 PID 文件为 `phase23-scram-proxy.pid`，0600，记录真实 pid、startTime、
namespace、leasePid、leaseStartTime、脚本路径和固定监听端口。现有记录不
被覆盖；退出只删除与本进程原始字节相同的记录。唯一环境 owner 负责外层
异常终止后的精确 PID 核验和补偿清理，不能仅凭 pid 或端口杀其他进程。

## 连接预算和结束

最多 8 个 accepted/connect/active 工作项，OS backlog 为 8；满时立即关闭新
socket，不排后台等待任务。每连接两方向**合计**最多 1 MiB，达到预算关闭；
包含连接建立在内的总期限 10 秒，连接建立/无进度空闲期限 3 秒。
每次读取至多 64 KiB，写入后 await drain，reader 采用有界缓冲；不积累全文。
允许正常 TCP half-close，返回方向仍受总期限和空闲期限约束。

SIGINT/SIGTERM/SIGHUP 或租约结束触发 finally：先关 listener，abort 所有
两侧 sockets，取消并 drain 工作任务，再等待 server closed，最后删除自己
的 PID 记录。listener 的 wait_closed 不先于活动连接关闭，避免等待死锁。

正常 stdout 只有固定 READY／STOPPED 行，失败 stderr 只有固定
`PHASE23_SCRAM_PROXY_FAILED`，无异常对象/traceback、连接明细或流量内容。
代理不替代数据库权限、认证失败分类、source/restore 归属检查或零连接确认。
同一进程可在源库关闭连接后供恢复库继续使用；它不选择库或角色。

## 本地自测与依赖核查

```sh
python3 phase23-scram-proxy.py --self-test
```

自测仅在 WSL 启动独占临时回环 echo listener；内部构造器用 port=0 和 echo
目标端口，永不访问真实 25495/55494。覆盖 source 地址、透明字节/half-close、
8 个连接上限、idle、累计字节、持续活动下的 total deadline、主动 close、
私有文件/symlink、PID 记录及租约消失。期限测试使用内部缩短预算，不改变
CLI 固定合同；临时 listener、socket、任务和文件均 finally 清理。

已在当前 WSL Python **3.14.4** 只读检查 import availability：`psycopg`、
`psycopg2`、`pg8000` 均不存在。没有安装包、连接数据库或改动 HBA。
真实 PostgreSQL structured SQLSTATE 将由协调者的独立 Rust SQLx probe 检查。
