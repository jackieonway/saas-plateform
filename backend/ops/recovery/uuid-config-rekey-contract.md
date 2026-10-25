# UUID-U05 类型化配置改号合同

`saas-recovery uuid-config-rekey /absolute/private/config.json` 是离线配置入口，不连接数据库、不应用授权、不启动服务。配置示例见 [uuid-config-rekey.example.json](uuid-config-rekey.example.json)。仅支持 `profile=uuid-local-preview`；先完整调用与 `uuid-import` 相同的 `verify::load_verified`，认证 source、converted、typed map、v2 注册表及所有表/证明文件，并重新验证映射源闭包。

配置字段为 `profile`、`source_dir`、`converted_dir`、`scratch`、`registry`、`key_file`、`output_dir`、`inputs`。`inputs` 为 1～3 项 `{owner,path}`，每种 owner 最多一个，输入路径也不可重复。所有 JSON 先拒绝重复键；配置本身拒绝未知字段。操作方须传入实际私有文件，示例仓库里的 placeholder grants 不应改写。

| owner | 处理与可复用性 |
| --- | --- |
| `data-grant-policy` | 封闭 DATA_GRANT_POLICY DTO，按已登记的 applicationId、tenantId、organizationId 字段改号；改号前后验证完整 owner 约束，目标只能为 RFC UUID v7。输出可用于新运行配置。 |
| `platform-admin-grants` | 封闭 platform-admin grant-intent DTO，仅支持现有 `work-orders` / `stores` 合同；按已登记的 application/menu/target 五种主体字段改号。输出是授权意图清单，不会通过 HTTP 重新应用或替换目标库现有授权。 |
| `preview-private-state` | 只做受限旧状态清点，最终输出为 `REGENERATION_REQUIRED` 和 typed counts；没有旧/新业务 UUID、OAuth clientId、配置值或秘密。没有可供 launcher 复用的 state。 |

只有 v2 注册表中已有的有限格式生效。owner 的 operation、format、unknownUuidPath、路径全集、authority、nullable、cardinality 必须与编译的封闭 handler 精确匹配。policy 和 grants 的所有嵌套 DTO 拒绝未知字段，保留未涉及的值和数组顺序。UUID 字段输入仅接受规范小写、非 nil 的 RFC v4/v7；通过 `(authority,oldUuid)` 查找已有 source-entity 映射，不新增映射，不允许历史 tombstone 满足配置引用。目标再次按 owner 完整校验。

`menuIds`、state 的 `applications` / `clients` 是对象 map；wildcard 访问 value，逻辑 key 不变。OAuth `client_id`、应用 code、角色/组织/租户 code、静态 menu key / parentKey 等已知业务标识不会因为形似 UUID 而改号。未登记路径上的独立 UUID 值会阻断；不对描述文字、URL 或 token 做全局字符串替换。

旧 state 处理先整体丢弃 `platformSession`、`businessSession`、`applicationTokens`，删除 `clients.*.clientSecret` 和 `storeOrganization.administrator.initialPassword`，丢弃 fingerprint 和三个阶段标志，再验证其余受支持的有限 owner 字段。非空组织 `children` 没有递归登记路径，直接阻断；不要删掉 children 以伪造可复用状态。实际旧 state 若包含这种树，应整体归档隔离，由目标正式 API 重建。source state 从不写回，也不会把已脱敏的 source state 发给 launcher。

配置、密钥和输入必须为执行用户所有的 mode 0600 单链接普通文件，拒绝符号链接以及 `.` / `..` 路径；policy 上限 4 MiB，grants 上限 1 MiB，state 上限 16 MiB。scratch 必须为 Linux tmpfs。output 必须是独立的新路径，其父目录已存在且为执行用户所有的 mode 0700；不得与 source、converted、scratch、key、registry、CLI 配置或输入重叠。输出目录以 0700 创建，文件以 0600 create-new 写入，不覆盖旧文件。

成功目录包含各 owner 的 JSON、`summary.json` 和 `manifest.uuid7`。两个可复用 owner 的 JSON 是私有明文运行配置，须保留在受控私有存储；state JSON 只有重建状态和 counts。每个输出均逐字节回读验证。summary 只含 owner、固定输出文件名、输入/输出摘要和大小、authority counts、source/converted/map/registry 摘要及维护 batch ID，没有业务身份映射、私有输入路径或秘密。manifest 使用独立 purpose `saas-uuid-config-rekey-manifest/1/<batchId>` 认证封存并回读；认证成功后才发布 summary。失败可能留下未完成的新私有目录，没有成功 summary 的目录不可作为完成证据。

配置改号不代替 U05 的运行验收：必须重新登录、重新选择身份上下文、轮换 confidential OAuth client secret、重新签发应用 token、重新计算目标 fingerprint，并核验目标授权和三个阶段标志。新主体 ID 应通过新认证后的目标 API 查询。该入口不证明服务已启动或业务链路可用，U05 阶段结果仍以 validation 和台账的实际记录为准。
