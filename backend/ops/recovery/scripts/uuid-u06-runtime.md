# UUID-U06 final／recovery 运行材料合同

`prepare-uuid-u06-runtime.py` 是 Linux root 离线准备器，只接受 `uuid-local-preview`。它读取已审查的三库导入 receipt、credential epoch、六份服务环境 JSON、原始权限策略和 U05 r2 静态清单，验证后生成启动 argv。它不连接数据库、不调用 Docker、不启动服务、不恢复会话，也不自动覆盖或删除目录。数据库实况、导入身份保持、实际凭据轮换和旧凭据拒绝，由 U06 操作者的独立证据证明。

```sh
python3 -I prepare-uuid-u06-runtime.py /absolute/private/u06-runtime.json --check
python3 -I prepare-uuid-u06-runtime.py /absolute/private/u06-runtime.json --prepare
```

`--check` 不创建输出。`generation` 仅为 `final` 或 `recovery`，分别只允许新路径 `/var/lib/saas-uuid-u06-runtime-20261024/final` 和 `/var/lib/saas-uuid-u06-runtime-20261024/recovery`。base 不存在时由 prepare 建立；已存在时须 root／0700。每代目录、state 和其他私有目录为 root／0700；私有文件为 root／0600，复制二进制为 0700。静态子树为 0755／0644，Nginx 配置为 0644。所有私有输入须 root／0600、单硬链接、父目录 root／0700，路径任何部分不得为符号链接。

manifest 精确包含以下字段，未知字段拒绝：

| 字段 | 合同 |
|---|---|
| `format` | `saas-uuid-u06-runtime/1` |
| `profile` | `uuid-local-preview` |
| `generation` | `final` 或 `recovery` |
| `output_dir` | 对应固定新目录 |
| `images` | 恰有 `runtime` 和 `gateway`，各项只含 `name`、`id` |
| `receipt_file`、`receipt_sha256` | 私有导入 receipt 路径和完整原始字节 SHA-256 |
| `policy_file`、`policy_sha256` | 私有原始策略路径和完整原始字节 SHA-256 |
| `frontend_manifest`、`frontend_manifest_sha256` | 精简私有前端 DTO 路径和完整原始字节 SHA-256 |
| `services` | 恰有六个服务，各项只含 `binary`、`env_file` |

镜像名和不可变 ID 均固定，不能仅替换 digest：

| 角色 | 镜像名 | ID |
|---|---|---|
| runtime | `saas-ci:phase04-tools` | `sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7` |
| gateway | `nginx:1.28.0-bookworm` | `sha256:3b03dfd827515a07ccf4946000f0bbdd2a4fb6aca7499249fcb6b1f9726823d6` |

六个服务为 `platform`、`stores`、`work_orders`、`notification`、`examples_oauth`、`examples_chat`。`binary` 精确含 `path` 和 `sha256`，复用 U05 已冻结的六个 `/usr/local/libexec/saas-uuid-u05-*` 二进制及其固定摘要，尤其平台仅包含迁移 1～12。`env_file` 是私有 JSON 对象，字段与 [U05 服务输入合同](uuid-u05-runtime.md) 相同：platform 的三项、stores／work_orders 的四项、notification 的两项、OAuth 的五项及可选 device secret，chat 为 `{}`。所有固定 origin、端口和 imported guards 均由准备器补齐，输入无法覆盖。DB 密码及各类 secret／token 为互不复用的 32～256 位 URL-safe 字符串。

receipt 精确包含：

```json
{
  "format": "saas-uuid-u06-imported-receipts/1",
  "generation": "final",
  "targets": {
    "platform": {
      "database": "qiankun_platform_uuid7_final_20261024",
      "schema_sha256": "<实际原始 schema SHA-256>",
      "import_marker": "saas-uuid-import/1/<canonical UUID v7 batch>/<converted digest>",
      "owner_role": "qiankun_platform_uuid7_final_owner_20261024",
      "runtime_role": "qiankun_platform_uuid7_u06_final_runtime_20261024",
      "migration_versions": [1,2,3,4,5,6,7,8,9,10,11,12],
      "migration_history_sha256": "80706cce23384465caf0e66a2a3056b4b003f6ee1ea47ea1d8095e985efbb2c3"
    },
    "stores": {"...": "同一组精确字段"},
    "work_orders": {"...": "同一组精确字段"}
  },
  "notification_runtime_role": "qiankun_notification_uuid7_u06_final_runtime_20261024",
  "epoch_file": "/absolute/private/credential-epoch.json",
  "epoch_sha256": "<epoch 原始字节 SHA-256>"
}
```

上例中的摘要占位符及 `...` 用于说明，不能直接执行。三库名称分别为 `qiankun_{platform,stores,orders}_uuid7_{generation}_20261024`；owner 为 `qiankun_{platform,stores,orders}_uuid7_{generation}_owner_20261024`；runtime 为 `qiankun_{platform,stores,orders}_uuid7_u06_{generation}_runtime_20261024`。notification 使用独立精确角色，指向本代平台库。DB URL 只接受显式 `127.0.0.1:5432`、对应数据库及角色；不接受 query、fragment、隐藏连接 options。

三个 marker 必须为同一 v7 batch 和 digest。准备器验证 receipt 的语法、代际和字节承诺；实际 UUID 原值保持及 marker 对应的导入事实须由导入器／验收器验证，准备器不能把私有 JSON 当成数据库实况证明。

迁移历史完整绑定冻结的版本和 checksum，不能只提供数量或最大版本。历史 digest 是下列数组按版本排序后，用 `json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()` 编码再做 SHA-256：

```json
[{"version":1,"success":true,"checksum_sha384":"<实际迁移文件 SHA-384>"}]
```

| 库 | 版本 | 历史 SHA-256 |
|---|---|---|
| platform | 1～12 | `80706cce23384465caf0e66a2a3056b4b003f6ee1ea47ea1d8095e985efbb2c3` |
| stores | 1～3 | `0ee5122f007f1b19e75e5fa0f8974f7da3944522473619bfdfdaf91b62b3b0e7` |
| work_orders | 1～4 | `ceb4a5cd8aa674a201bd98f4e2831b89e20b40c91b38bbca4a35eac9c39c479a` |

credential epoch 精确含 `format="saas-uuid-u06-credential-epoch/1"`、`generation`、`jwt_secret_sha256`、`previous_jwt_secret_sha256`、`confidential_clients`、`secrets_fresh=true`。clients 恰有 12 项，每项只含 `client_id`、`previous_secret_hash`、`new_secret_hash`；不含原始 secret。源 opaque ID 集合和 final 的前代 client hash 映射、JWT hash 都绑定已审计 U05 commitment。新 JWT 和 client secret 互相独立，也不复用前代。四个实际 confidential service secret 及可选 device secret，按 env 中的 opaque client ID 与 epoch 对应；平台 JWT 必须对应新 hash。其余 confidential client 的真实轮换由 epoch 工具与 DB 拒绝测试验收。

recovery 必须在 final 已准备后生成。它读取固定 final 输出中的 `prepared.json`、`imported-receipts.json`、`credential-epoch.json`，复核两个完整文件 hash 及 epoch hash 绑定，要求 recovery 前代恰为 final 新凭据，并拒绝复用最初 U05 的 JWT／client secret。它无需也不会重新改号。

policy 继续使用原始已恢复策略；除 manifest 中的原始字节 hash 外，canonical hash 还须为 `a4742919d165d32a13fb439b3e4f213b13c260f76f47806261d54ad600622563`。不消费 U05 config-rekey summary，不把新 import batch 拼入旧 summary，不允许临时通知授权混入。

前端 DTO 精确沿用 U05 的 `format/origin/sdk_version/artifacts` 与每项 `app/root/version/files` 字段。它绑定 [已验收 r2 报告](../../../docs/identity-sso/validation/uuid-u05-frontend-builds-r2.json) 的八个应用、SDK 3、253 个文件的完整 path／bytes／sha256 和版本；报告额外字段及 Windows root 须另投影为私有 DTO。rootless descriptor canonical hash 固定为 `eb14516b7c2894b6b793e77a1cb90a49121a884d10fbf13f8f29ce99f95f210a`。仅允许平台源目录 `/var/lib/saas-uuid-u05-materials-20261024/host-u05-r2/static`，其余应用源目录 `/var/lib/saas-uuid-u05-runtime-20261024/static/{app}`。拒绝额外文件、缺失文件、符号链接和自填新摘要绕过 r2 审查。

准备成功后最后写 `prepared.json`，此前失败留下的目录继续隔离，不能自动重试覆盖。每代有六个空 state、新 env、新静态副本、epoch／receipt 副本和七条 `launch-plan.json` 命令。容器名含 U06、generation、service；所有 mount 只来自本代输出。保留只读 root、cap-drop ALL、no-new-privileges、无 Docker env-file、不可变镜像 ID。chat 保留 interactive stdin；runner 只执行固定 `/run/u06/service`，每次复核二进制 hash，以 Python `-I` 启动，再用 `os.execve` 提供明确环境，服务日志只写本代 private state。

final 和 recovery 使用同一 origin 13000 和同一组端口（platform 13001、notification 13002、OAuth 13007、chat 13008、stores 13010、orders 13011），须由操作者保证互斥启动，launch plan 明确 `requires_no_other_generation_running=true`。准备器不查询／停止另一代。Nginx 复用 U05 路由合同：stores／orders 保留 API 前缀，OAuth rewrite 和 cookie／Authorization 隔离，notification Upgrade，chat 使用独立 13008，不转发到通知。私有路径拒绝和静态真实 404 保持原合同。

离线验收命令：

```sh
python3 -m unittest discover -s ops/recovery/scripts/tests -p test_prepare_uuid_u06_runtime.py -v
python3 -m unittest discover -s ops/recovery/scripts/tests -p test_prepare_uuid_u05_runtime.py -v
```

2026-10-24：U06 的 17 项 Linux root 测试、U05 原有 6 项测试通过。U06 测试只使用临时 synthetic 材料，覆盖双代新目录和空 state、receipt／schema／迁移绑定、epoch 续链和旧秘密拒绝、image 固定、权限／硬链接／符号链接／重复 JSON、r2 篡改、policy 篡改、生成网关保留 U05 发行路径，以及真实 synthetic runner 的 `execve` 环境隔离。未执行真实服务、数据库、Docker 或切流操作；RPO／RTO 由主 U06 演练计时与真实数据验收证明。
