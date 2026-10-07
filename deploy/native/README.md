# Linux 原生部署

本目录提供配置生成器、锁定依赖构建和 systemd 模板。2026-10-07 的交付目标是本机隔离 Linux 准生产验收；实际结果见 `docs/phase12-validation.md`。模板存在不代表已部署现网。

## 1. 构建可冻结的部署产物

使用 Node 24 和仓库要求的 Rust 工具链，指定一个**尚不存在**的构建目录：

```sh
node deploy/native/build.mjs --out-dir /srv/dante-builds/20261007-001 --cargo /usr/bin/cargo
```

脚本仅复制明确允许的源文件，拒绝符号链接，排除已有 `.env`、`node_modules`、构建产物和 Git 数据；随后执行各自的 `npm ci`、TypeScript、Vite 和 `cargo build --release --locked`。任何一步失败即停止，不启动网关或替换当前版本。构建生成源文件 SHA256、私有日志和 `artifacts`，平台库迁移嵌入 Rust 二进制。依赖缓存可以复用，已有项目安装目录不参与构建。

受限网络中可用绝对路径 `DANTE_NPM_CACHE` 指向预热过的 npm 内容缓存，并设置 `DANTE_NPM_OFFLINE=1` 要求离线安装。缓存需包含锁文件中的 Linux 原生可选包；缺包时构建会失败，不会回退到现有 `node_modules`。缓存仅供依赖下载，源码仍从白名单复制。
重复构建时可用绝对路径 `DANTE_CARGO_TARGET_DIR` 复用 Rust 编译缓存；每次输出目录仍必须是新的空路径，构建结束写入新的源码清单与产物。

构建目录应使用持久文件系统。本机 WSL 的 `/tmp` 是 tmpfs，发行实例停止时会丢失内容；准生产产物放入自有 `0700` 的 `/var/tmp` 目录。可用 `DANTE_NPM_CACHE=/persistent/owned/cache` 复用依赖缓存，源目录和安装仍从空目录重新建立。

`artifacts/frontend` 是宿主，`notification-ws` 与 `work-orders-api` 是各自独立运行产物；微应用 Vue 构建使用独立版本发布工具，放入 `micro-apps/work-orders/versions/<version>`，不重建宿主。

## 2. 服务身份、密钥与网关

由运维分别建立无登录服务账号 `dante-platform`、`dante-notification`、`dante-work-orders` 和 `dante-gateway`，避免业务服务读取平台进程的签名密钥环境。本次本机验收使用当前用户的自有进程和目录，不改变全局账号、服务或证书信任；未把 systemd 模板安装当作已完成门禁。生产数据库账号按服务分别授予最小权限；通知账号仅需要其查询表的 `SELECT` 与连接／LISTEN 权限。备份管理员凭据不交给服务或浏览器。

创建 `/srv/dante/secrets`，限制目录为 `0700`，配置文件为 `0600`，由 root／运维管理，systemd 管理器读取各自 `EnvironmentFile`，应用只接收自己的环境。私有 CA 文件可单独提供只读公共证书路径；网关 TLS 私钥放在仅网关身份可读的独立目录。分别填写本目录的三份 `*.env.example`。随机签名和指标密钥至少 32 字符；各服务指标 token 相互独立。平台生产启动拒绝开发 JWT、默认管理员密码和 HTTP origin；已有用户库不再次建立默认管理员。机密客户端 ID/secret 来自已创建并显式关联工单应用的客户端。

设置正常可信 HTTPS 证书；私有 CA 可通过平台 `MICRO_CA_CERT_FILE`、Node 启动前 `NODE_EXTRA_CA_CERTS` 指定。保持证书和主机名验证；不要关闭 TLS 校验，也不要修改全局信任。浏览器测试使用自有 XDG 数据目录中的 NSS 信任数据库。模板路径要求绝对 POSIX 路径，无空白或配置元字符。

按 `deployment.example.json` 填写实际端口、证书和产物位置，再生成配置：

```sh
node deploy/native/render.mjs --config /srv/dante/deployment.json --out-dir /srv/dante/configuration
nginx -t -p /srv/dante/ -c /srv/dante/configuration/nginx.conf
```

模板的 listener 默认只绑定本机。本次准生产使用 `[::1]:<隔离端口>`；外部生产入口应由运维显式选定如 `0.0.0.0:443` 或 `[::]:443`，并匹配防火墙／负载均衡边界。内部服务地址始终限定回环，TLS 必须仍在生成的网关中启用。

网关精确代理 `/api/`、`/oauth/`、`/work-orders-api/` 和 Socket.IO；覆盖客户端伪造的 `X-Real-IP`／`X-Forwarded-For`。`/api/internal/*` 及所有指标接口禁止公共访问，监控直接访问私有上游并提供对应 token。`/notification-health` 只转发就绪探针。

默认 CSP 不允许 `eval`，适用于 opaque iframe 工单。配置 `applicationOrigins` 可以显式加入独立 HTTPS iframe 应用来源；CSP 是浏览器加载限制，不能代替平台应用发布来源白名单。外部 ESM 若要在宿主执行，还须平台明确标记可信，并由运维在 `trustedScriptOrigins` 中精确批准脚本来源；添加 iframe 来源不会自动获得宿主脚本执行许可。已明确受信的 HTML 应用若依赖框架沙箱 `eval`，须由运维审核后显式设置 `trustedHtmlEval:true`；该操作放宽宿主页脚本策略，应在发布记录中说明。不会自动加入 `'unsafe-eval'`。样式 inline 由 React/Vue 控件需要，脚本仍禁止 inline。

## 3. 启动与迁移停止路径

固定不可变版本目录，审查生成的四份 systemd unit，再复制至机器自己的 unit 目录。代码和静态目录由运维持有，服务身份只有读取／执行权限；创建模板列出的日志和 nginx-temp 目录，只赋予 `dante-gateway` 必要写权限。先启动平台，确认 `/api/health` 及迁移校验；启动 HTTPS 网关后再启动通知，使其 HTTPS 平台探针收敛；创建/关联机密客户端并登记应用，最后启动独立业务服务。原生网关只需要绑定低端口能力，其他服务无需 root 或 Linux capability。

迁移失败时平台不得开放健康监听；保留失败日志和原库，停止本次发布，不自动覆盖或回退数据库结构。修复前先确认 `_sqlx_migrations` 的已成功版本和校验和，按备份恢复手册恢复到新库或使用经过审核的增量修复迁移。不可修改历史迁移或将程序版本回滚等同数据库回滚。

微应用发布按预检 → 启用 → 验证操作；回滚只移动已有版本部署指针，保留所有版本目录。平台／业务 API 版本回滚须先评估数据库向后兼容，不自动回退迁移。

## 4. 监控、备份和留存

运行 `deploy/monitoring/monitor.mjs`，公开健康／应用入口采用 HTTPS 并正常验证专用 CA；私有 loopback 指标按服务 token 校验。告警只在状态转移时记录 `DOWN`／`RECOVERED`，本机演练使用本地事件日志。故障注入只停止本次启动的自有 PID。

执行 `deploy/recovery/pg-backup.mjs` 前停止所有写入者，平台和业务库作为同一备份集加密保存。每日备份和 24 小时 RPO、1 小时 RTO 为本次演练目标；不是商业 SLA。恢复默认只创建新库，先验证两个密文再创建目标，并强制撤销旧会话/令牌、轮换 JWT 密钥和重新登录。详情见备份恢复文档。

初版不自动清理工单软删除、通知、审计或会话历史。正式保留期、批量清理与现网告警接入由后续产品／运维决策。不要将生产秘密、证书私钥或测试运行数据提交 Git。
