# U06 HTTP 凭据与业务恢复验收

两个工具均要求 Linux root、root／0700 私有父目录和 root／0600 单链接私有输入，只向 stdout 输出固定状态、检查编号、计数和 SHA-256。它们不连接数据库、不调用 Docker、不冻结或切流，也不跟随 callback。认证协议允许写入真实登录、授权和 token 状态；业务工具只接受 final／recovery。

## 真实凭据捕获与代际拒绝

`validate-uuid-u06-runtime.py` 的私有 config 共有字段为：

```json
{
  "format": "saas-uuid-u06-http/1",
  "generation": "source",
  "output_dir": "/var/lib/saas-uuid-u06-20261024/http-source-attempt01"
}
```

`generation` 仅为 `source`、`final` 或 `recovery`，输出必须是固定父目录下尚不存在的 `http-{generation}-[a-z0-9-]+`。final 再精确增加 `source_capture_file`、`source_capture_sha256`，指向真实 source capture 文件及原始字节 SHA-256。recovery 的这两项指向 final capture，并须另加 `original_source_capture_file`、`original_source_capture_sha256` 指向最初 source capture；recovery 会同时重放两代真实旧凭据。

```sh
python3 -I ops/recovery/scripts/validate-uuid-u06-runtime.py /absolute/private/http-config.json --check
python3 -I ops/recovery/scripts/validate-uuid-u06-runtime.py /absolute/private/http-config.json --run
```

`--check` 验证本代所需私有材料但不创建目录、不请求 HTTP。工具固定读取 U05 的 `credential-handoff.private.json` 作为 login／应用／opaque client ID／v7 身份拓扑；source key 来自 U05 runtime env，final／recovery key 来自各自固定 U06 runtime env。新一代 client raw map 读取 `/var/lib/saas-uuid-u06-20261024/epoch-{generation}/client-secrets.private.json`，并对照 credential epoch 的全部 12 项新 hash；它不使用预 U05 的旧密钥。

source 模式实取并私存以下 14 个样本：credential ticket、普通 platform session、platform refresh 各 1，七个实际应用 access token，普通 OAuth access／refresh 各 1，以及真实批准后未兑换的授权码（含原 PKCE verifier／challenge）、真实批准后未兑换的 device code 各 1。它验证 11 个 ENABLED confidential client 用正确本代密钥认证成功、1 个 DISABLED client 用正确本代密钥仍返回 `invalid_client`；不会把禁用客户端当作正向认证样本。OAuth callback 仅解析为数据。

final／recovery 先登录并验证新 key 的签名、原 v7 身份和 opaque ID 保持，随后重放上一代真实样本。旧 session 和七个旧 app access 必须真实 HTTP **401**；旧普通 OAuth access 另外用新 client secret 调用 digest introspection，必须 inactive；code 必须带原始 PKCE verifier，refresh／device 必须使用真实原值。随后检查平台 refresh、七应用 context／authorize、OAuth code／PKCE／refresh、机器凭据边界和 device lifecycle。最后再捕获本代 14 样本供下一代使用，并请求拒绝所有旧 raw client secret：final 12 项，recovery 同时拒绝 source 和 final 两组共 24 项。

每次输出 `transcript.private.json`、成功取得的 `capture.private.json` 和 `summary.json`，均为 0600。capture hash 写入 summary，可直接作为后续 config 的承诺；下一代还须回读同目录 summary，只有 PASS 且 generation／capture hash／identity hash 一致才能继续。U06 的业务 v7 ID 原值保持，没有伪造 v4 old state 或重复 UUID 改号。

冻结二进制的授权码只有 **60 秒 TTL**。工具将未兑换码放在捕获末尾，记录实际签发时刻及按冻结合同计算的 expiry；device 使用真实 `expires_in`。重放报告分别计数已自然过期样本，`natural_expiry_is_epoch_proof=false`。旧样本过期后的拒绝不能单独证明轮换，须结合六凭据表排除、JWT 改 key、session_version 增量及数据库真实证据。不能为赶验收延长 TTL。

2026-10-24 第一轮 source 实跑：**67 次 HTTP、5 组检查全部 PASS**，14 样本完整；11 ENABLED 认证、1 DISABLED 拒绝、7 应用协议通过；0 DB、0 业务写、0 外部 callback。私有 capture 原始 SHA-256 为 `ee088793e0768f08a63536d2531418eff01898f1348736d48764eac35f704d14`。此轮用于验证工具，可在主任务 freeze 前换新目录重捕获；不要覆盖已有证据。

## final／recovery 业务探针

`validate-uuid-u06-business.py` 私有 final config：

```json
{
  "format": "saas-uuid-u06-business/1",
  "generation": "final",
  "output_dir": "/var/lib/saas-uuid-u06-20261024/business-final-attempt01",
  "http_config_file": "/absolute/private/http-final-config.json"
}
```

recovery 增加 `final_receipt_file`、`final_receipt_sha256`，指向成功 final 的 `business-receipt.private.json` 及其原始 SHA-256。输出只能是固定父目录下新 `business-{generation}-[a-z0-9-]+`。`http_config_file` 使用上节当前代已准备的 config，可对应已经存在的 HTTP 输出；工具重新读取当前 key／epoch，并另建 fresh session，不消耗共享 refresh。

```sh
python3 -I ops/recovery/scripts/validate-uuid-u06-business.py /absolute/private/business-config.json --check
python3 -I ops/recovery/scripts/validate-uuid-u06-business.py /absolute/private/business-config.json --run
```

final 先核验实际服务 JWT 为本代 key，再分别建立 stores／work-orders 业务 context，才允许业务请求。它读取已有门店，以同值 `businessContact` 做一次带 version 的 PATCH，要求 version 恰加 1、原值及 tenant／organization／owner 保持，随后读取完整 row。工单仅新建本轮 `u06-final-*` 测试 ticket，以稳定 Idempotency-Key 重复 POST 验证完整响应相同，再以 version PATCH 本 ticket 的描述及 `IN_PROGRESS` 状态，最后 GET 保存含 ID、owner、version、createdAt／updatedAt 的完整 row。

recovery 重新登录后精确比较 final 门店和更新后 ticket 的持久字段，包括时间、version 和 owner；门店实时 organization 投影也逐字段一致，仅 `organization.expiresAt` 按每响应 lease 合同独立验收。平台 `app_organizations.rs` 的窗口为 3 秒，stores 再校验 `now < expiresAt ≤ now+3000ms`；准备器记录每个实际 API 响应的接收时刻，并要求该字段为 UTC 毫秒 RFC3339 且在接收后 3 秒内有效。比较只排除这一精确字段，`revision`、`contextVersion`、全部其他组织／营业字段和新增未知字段仍严格匹配。完整原始响应及其 lease 留在私有 API receipt 中，不改写旧证据。recovery 再重放 final 原始创建请求，要求返回保存的初始幂等响应，而当前 GET 仍为更新后版本。完整分页读取的既有通知 inbox 行和 deep link 必须与 final 精确一致。全部保持检查通过后，才新建本轮一张 `u06-recovery-*` v7 ticket，并重复创建／GET 核对新写可用。

通知仅 GET；endpoint allowlist 禁止发布、remind、read mutation、delete、外部 URL 和其他平台写接口。真实执行必须由主任务在对应 U06 运行环境启动后触发。source generation 被拒绝；当前 U05 服务使用旧 key 时，登录 JWT 验证会先失败，无法进入业务写步骤。

每次调用前保存私有稳定 request plan；每个业务 HTTP 返回立即写独立不可覆盖的 `api-NNNN.private.json`。最终保存 `business-receipt.private.json`、`authentication.private.json`、`summary.json`，均为 0600。失败保留部分私有证据和已发生操作，不自动重新创建或覆盖目录。公共 summary 只含计数和完整 receipt hash；不输出 ticket／owner／通知 ID、标题、link 或原始凭据。

离线回归：

```sh
python3 -m unittest discover -s ops/recovery/scripts/tests -p test_validate_uuid_u06_runtime.py -v
python3 -m unittest discover -s ops/recovery/scripts/tests -p test_validate_uuid_u06_business.py -v
```

HTTP 10 项、business 9 项离线测试通过。business 测试覆盖 final 全流程、recovery 完整持久 row／幂等响应／通知 deep link 保持、新 v7 写，以及时间、owner、version、link 篡改拒绝；另以真实首轮 PATCH／GET 的 lease 时间差复现误报，并验证仅 3 秒内合法新 lease 可变化。未针对当前 U05 执行任何真实业务写。实际 final／recovery 的 PASS 和 RPO／RTO 由主任务运行结果验收，不由离线测试代替。
