# UUID-U05 固定本地运行材料准备

`prepare-uuid-u05-runtime.py` 只读取受限输入并生成新运行目录。它不连接数据库、不创建或轮换凭据、不启动／停止容器、不调用迁移或 seed，也不把旧 preview state 发给运行服务。真实角色准入、凭据轮换、目标 API 重建 state 和 U05 端到端验收由操作者单独完成；此工具不执行 U06 最终切流。

固定公共 origin 为 `http://127.0.0.1:13000`。平台 API、通知、OAuth API、chat、门店、工单分别绑定 loopback `13001`、`13002`、`13007`、`13008`、`13010`、`13011`，不接管旧 3000 系列端口。chat 的 Engine.IO path 为 `/`，前端直接使用 `http://127.0.0.1:13008`；通知保留网关 `/socket.io/`。chat 的 launch argv 明确包含 `--interactive`，避免 stdin EOF 触发退出。

## 私有输入

全部 JSON 使用 UTF-8，不允许重复键。manifest、receipt、各服务 env JSON、policy、config-rekey summary、frontend manifest 必须是当前执行用户所有、单链接、mode 0600 的常规文件。所有路径必须是无符号链接、无 `.`／`..` 的规范 Linux 绝对路径。运行程序固定为 `/usr/local/libexec/saas-uuid-u05-{platform,stores,orders,notification,oauth,chat}`；六个 SHA-256 固定在脚本内，平台只能使用冻结的 12 项迁移版本，不能替换为包含身份 0013 的当前源码构建。

主 manifest 字段精确如下，不接受额外字段：

```json
{
  "format": "saas-uuid-u05-runtime/1",
  "profile": "uuid-local-preview",
  "output_dir": "/var/lib/saas-uuid-u05-runtime-20261024",
  "images": {
    "runtime": {"name": "saas-ci:phase04-tools", "id": "sha256:<实际镜像 ID>"},
    "gateway": {"name": "nginx:1.28.0-bookworm", "id": "sha256:<实际镜像 ID>"}
  },
  "receipt_file": "/absolute/private/imported-receipts.json",
  "policy_file": "/absolute/private/config-u05-02/data-grant-policy.json",
  "policy_summary_file": "/absolute/private/config-u05-02/summary.json",
  "frontend_manifest": "/absolute/private/frontends.json",
  "services": {
    "platform": {"binary": {"path": "/usr/local/libexec/saas-uuid-u05-platform", "sha256": "<固定摘要>"}, "env_file": "/absolute/private/platform.json"},
    "stores": {"binary": {"path": "/usr/local/libexec/saas-uuid-u05-stores", "sha256": "<固定摘要>"}, "env_file": "/absolute/private/stores.json"},
    "work_orders": {"binary": {"path": "/usr/local/libexec/saas-uuid-u05-orders", "sha256": "<固定摘要>"}, "env_file": "/absolute/private/orders.json"},
    "notification": {"binary": {"path": "/usr/local/libexec/saas-uuid-u05-notification", "sha256": "<固定摘要>"}, "env_file": "/absolute/private/notification.json"},
    "examples_oauth": {"binary": {"path": "/usr/local/libexec/saas-uuid-u05-oauth", "sha256": "<固定摘要>"}, "env_file": "/absolute/private/oauth.json"},
    "examples_chat": {"binary": {"path": "/usr/local/libexec/saas-uuid-u05-chat", "sha256": "<固定摘要>"}, "env_file": "/absolute/private/chat.json"}
  }
}
```

镜像 ID 由操作者用实际本地 `docker image inspect` 获取，launch argv 使用不可变 ID。脚本只验证 ID 语法，不替操作者证明镜像来源。

receipt 的格式为 `{"format":"saas-uuid-u05-imported-receipts/1","targets":{...}}`。targets 恰有 `platform`、`stores`、`work_orders` 三项，每项精确包含 `database`、`import_marker`、`schema_sha256`、`runtime_role`。固定三个 U04 目标及本代经双路审查的完整原始 schema 摘要写在脚本 `TARGETS`；marker 必须属于同一 canonical RFC UUID v7 batch、同一 converted manifest 摘要。运行角色名须是新的 `qiankun_*_u05_*20261024`，不得包含 owner，三主服务及通知不得共用角色。脚本不连接数据库验证 LOGIN／权限，操作者仍须完成真实数据库围栏验收。

每个 env_file 是仅含字符串值的 JSON 对象，字段精确如下。绑定地址、origin、imported 四字段、禁 seed 配置等由脚本补齐，输入不能覆盖；chat 输入为 `{}`。

| 服务 | 必需的私有输入字段 |
|---|---|
| platform | `DATABASE_URL`、`JWT_SECRET`、`METRICS_TOKEN` |
| stores | `STORES_DATABASE_URL`、`PLATFORM_CLIENT_ID`、`PLATFORM_CLIENT_SECRET`、`STORES_METRICS_TOKEN` |
| work_orders | `WORK_ORDERS_DATABASE_URL`、`PLATFORM_CLIENT_ID`、`PLATFORM_CLIENT_SECRET`、`WORK_ORDERS_METRICS_TOKEN` |
| notification | `DATABASE_URL`、`METRICS_TOKEN` |
| examples_oauth | `OAUTH_AUTH_CLIENT_ID`、`OAUTH_AUTH_CLIENT_SECRET`、`OAUTH_DEVICE_CLIENT_ID`、`OAUTH_CC_CLIENT_ID`、`OAUTH_CC_CLIENT_SECRET`；仅另允许可选 `OAUTH_DEVICE_CLIENT_SECRET` |
| examples_chat | 无 |

DB URL 必须为明确的 `127.0.0.1:5432`、对应固定目标和新角色，带认证密码且不含 query／fragment，不能夹带 `options`／SET ROLE。通知只连接新平台目标，使用独立 notification 角色，严禁传入 JWT secret。秘密须为 32～256 个 URL-safe 字符且彼此独立；工具不读取旧 secret，因此“已轮换且旧凭据不可用”仍由操作者的真实轮换和拒绝测试证明。OAuth `client_id` 保持 opaque 标识，不按 UUID 改号。

policy 只接受已成功完成 `uuid-config-rekey` 的 `data-grant-policy.json`。脚本检查 published summary 的成功状态、batch、converted 摘要、owner、输出文件名、字节数和 SHA-256，随后将紧凑 JSON 写入平台 `DATA_GRANT_POLICY`。它消费已经认证的 config-rekey 交接，不另行解密或声称认证其加密 manifest。旧 state 的清点或 `REGENERATION_REQUIRED` 输出不是可复用运行输入。

frontend manifest 精确包含 `format="saas-uuid-u05-frontends/1"`、`origin="http://127.0.0.1:13000"`、`sdk_version="3.0.0"`、`artifacts`。artifacts 恰有 `platform`、`stores`、`work_orders`、`vue-workshop`、`legacy-jquery`、`report-center`、`oauth-playground`、`websocket-chat` 八项；每项只含 `app`、Linux 绝对目录 `root`、`version`、`files`。files 每项只含相对路径 `path`、整数 `bytes` 和 `sha256`。完整目录文件集合必须与清单一致，无额外文件／链接；门店和工单 `build-info.json` 的 SDK 与版本也必须一致。工具消费前端构建代理审查的清单，不重新执行 SDK 或构建测试。公开审计报告中的额外说明字段应保留在报告中，操作者另建上述精简私有 DTO。

## 检查与准备

```sh
python3 prepare-uuid-u05-runtime.py /absolute/private/u05-runtime-manifest.json --check
python3 prepare-uuid-u05-runtime.py /absolute/private/u05-runtime-manifest.json --prepare
```

输出根只能是尚不存在的 `/var/lib/saas-uuid-u05-runtime-20261024`。成功后有 0600 env、复制且校验摘要的六个 binary、空 0700 state、环境隔离 runner、只读静态副本、网关配置、纯 argv 的 `launch-plan.json` 和最后发布的 `prepared.json`。私有输出为 0600、目录为 0700；仅公开静态子树为 0755／0644，公开网关配置为 0644，便于无特权 Nginx 读取。失败留下的部分目录应继续隔离；没有 `prepared.json` 不能启动，也不自动覆盖或清理。

操作者先核对 `prepared.json` 和 launch-plan，再按服务需要执行其中 argv。工具没有启动入口。新容器名均以 `qiankun-uuid-u05-` 开头，保留阶段和服务 ownership label、只读 rootfs、cap-drop ALL 与新运行目录。API runner 在 execve 前重新核对 binary SHA，使用明确的新环境，清除宿主的 PG／JWT／bootstrap 环境继承，stdout／stderr 仅写本服务 private state 中的 0600 文件。平台只加载专用空 env 文件，避免搜索仓库 `.env`。env 文件不是 shell 脚本，不应使用 `source` 执行。

网关路由来源为现有 `backend/nginx/policy.mjs`、宿主 `frontend/vite.config.ts` 和业务本地 Nginx 合同：业务 API 保留 `/stores-api`／`/work-orders-api` 前缀，OAuth API 精确改写 `/oauth-playground-api/*` 到 `/api/*`，剥离平台 bearer 和其他 cookie，只转发自己的 OAuth cookie。`/callback` 使用 OAuth `callback.html`，`/sub/oauth-playground/` 保留回调资源别名。八份静态产物复制到本轮新目录；每应用使用新版本路径和 `/sub/.../` 别名，缺失静态资源返回真实 404，只有宿主 HTML 导航可 SPA fallback。没有改写旧发行字节或接入未知版本。

## 独立验证

```sh
python3 -m unittest discover -s scripts/tests -p test_prepare_uuid_u05_runtime.py -v
UUID_U05_NGINX_FIXTURE=isolated-network-none-v1 python3 -m unittest discover -s scripts/tests -p test_uuid_u05_nginx_fixture.py -v
```

2026-10-24 实跑：6 项 Linux 合同／生成测试通过（含 restrictive umask 下的公开资源权限）；显式 Docker fixture 的 2 项测试通过，其中 Nginx 含 22 个真实 HTTP 检查，另一项实际 exec runner 证明旧 PG／JWT／无关环境没有继承、新配置完整生效且日志为 0600。Nginx fixture 使用独立 `--network none` 容器及共享该私有 namespace 的合成 probe，没有发布 host 端口、业务卷或数据库连接，退出前核验完整 ID、随机 label、network 和无端口绑定后仅移除自己创建的容器。覆盖真实 HTML／版本／SPA 路由、缺失 404、JS／MJS MIME、private 与编码 internal 拒绝、API 前缀、OAuth cookie／bearer 隔离及通知 Upgrade。首次独立 fixture 发现只读 rootfs 下 Nginx 默认 fastcgi 临时目录不可写，已将全部临时目录固定到私有 tmpfs 后通过；此失败没有涉及实际 U05 服务。
