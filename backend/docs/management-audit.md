# 管理变更审计

平台把关键管理变更与审计记录放在同一个数据库事务中，避免状态已变但没有审计的情况。公共实现位于 `src/util/audit.rs`，查询与 CSV 导出位于 `src/routes/audit.rs`，事务故障回归位于 `src/management_audit_tests.rs`。qiankun 应用的发布、入口配置、应用令牌和业务通知都保留应用归属，前端加载方案不改变这些审计边界。

## 记录内容

`audit_logs` 保存操作者 user ID 和用户名、租户与组织 ID、名称快照、action、detail、来源 IP、User-Agent、request ID 和 application ID。删除对象前在事务中写入名称快照，使租户或组织删除后仍可理解历史记录。

`change_detail` 生成以下结构并作为 detail 字符串存储：

```json
{
  "objectType": "membership_data_grants",
  "objectId": "11111111-1111-4111-8111-111111111111",
  "before": {},
  "after": {}
}
```

before/after 由每个调用点明确挑选安全字段，没有“把完整请求或数据库行自动写入审计”的通用能力。密码、Argon2 PHC、client secret、token 明文或摘要不能作为审计内容。客户端审计记录配置摘要和计数；URL 入口审计记录“已配置”、代理开关及 revision，不记录可能带查询参数的完整上游 URL。

User-Agent 修剪并截断到 500 字符，缺失时为 `unknown`。IP 在进入路由前经过可信代理规范化，详见[登录保护](login-protection.md)。请求编号由平台可观测性中间件建立，可以把浏览器回执、接口日志与审计关联起来。

## 写入方式与事务边界

| 实现 | 保证 |
| --- | --- |
| `audit::write` | 独立写入；失败记录审计失败指标与结构化错误，不回滚已经完成的业务 |
| `audit::write_in_tx` | 复用调用者事务；失败向上传播，调用方必须回滚业务变更 |
| `audit::write_in_tx_for_app` | 同上，显式绑定 application ID |

事务插入不只检查 SQL 是否报错，还要求 `rows_affected()==1`。数据库触发器以 `RETURN NULL` 静默跳过审计插入也被视为失败，避免成功响应却没有 durable audit row。

关键事务审计覆盖账户安全、密码、成员与角色替换、租户和组织及两级授权、应用和菜单、统一客户端／凭据／密钥管理与内部应用能力、版本发布/激活/回滚、数据授权、通知发布及平台设置等现有调用点。登录成功/失败等使用独立审计入口，不能把它们与关键变更事务混为同一种保证。

典型动作有 `CREATE_APPLICATION`、`UPDATE_APPLICATION`、`DELETE_APPLICATION`、`TOKEN_ISSUED`、`DATA_GRANT_CHANGE`、`CONFIGURE_APPLICATION_URL_ENTRY`。新增业务动作时，应明确对应对象、最小安全快照和失败是否需要回滚，而不是拼接不受控请求内容。

完全没有实际变更的操作可返回当前状态而不生成变更审计；不能只按审计行数量判断一个幂等请求是否成功。发布预检、注册和指针切换各有自己的事务边界，不能认为跨 HTTP 请求的整套发布工作流是一个事务。

## 查询与导出

```http
GET /api/audit-logs?page=1&size=20&action=DATA_GRANT_CHANGE
Authorization: Bearer <platform-session-access-token>
```

需要 `system:audit`。列表返回 `records`、`total`、`page`、`size`；记录包含 `id`、`username`、`tenantId`、`tenantName`、`organizationName`、`action`、`detail`、`ip`、`createdAt`、`userAgent`、`requestId`、`applicationId`。

| query | 规则 |
| --- | --- |
| `keyword` | 匹配用户名、detail、租户/组织名称、请求编号或应用 ID |
| `action` | 精确动作筛选 |
| `tenant_id` | 为空不增加过滤；`none` 只选 NULL 租户；其他为 UUID |
| `request_id` | 精确请求 UUID |
| `application_id` | 精确应用 UUID |
| `page`、`size` | 通用分页，size 上限 200 |

API query 使用以上 snake_case 字段，响应使用 camelCase。无效关联 UUID 返回 400，不应忽略错误后扩大查询范围。

具有平台管理上下文的用户可在其权限边界中查询跨租户记录；普通上下文强制限定本租户和当前组织子树。租户根组织可以包含没有 organization ID 的本租户历史行，子组织不能将这些未归属行当成本组织数据。用户提交 tenant 筛选只能进一步收窄后端范围。

`GET /api/audit-logs/export` 使用同样的权限和过滤，按时间及 ID 倒序导出，最多 10000 行。CSV 使用 UTF-8 BOM，并包含时间、用户、租户、组织、动作、详情、IP、UA、请求编号、应用 ID。对可能触发电子表格公式的 `=`、`+`、`-`、`@` 等前缀做文本化处理，再转义 CSV 分隔符。

## 排查顺序

1. 从成功回执或接口响应获取请求编号，使用 `request_id` 查询同一 action 和 application。
2. 核对当前组织、租户以及是否具备 `system:audit`；查询不到可能是范围限制，不能据此认定变更未发生。
3. 重新读取业务对象实际状态。网络断开导致结果未知时，不要直接重复管理写请求。
4. 如果写入返回失败，同时核对业务状态是否回滚以及审计失败指标；保留请求编号便于关联服务端日志。

## 回归约束

`src/management_audit_tests.rs` 通过抛错和静默跳过两种触发器验证失败回滚，检查 before/after 安全字段、名称快照、事务副作用、会话及 token 撤销、发布和通知幂等。`src/data_grant_tests.rs` 与 `src/release_tests/url_entries.rs` 对各自新增审计边界另有覆盖。`src/routes/audit.rs` 的单元测试验证 CSV 公式处理。

数据库回归明确要求 `TEST_DATABASE_URL` 和创建/删除隔离测试数据库权限。验证命令应在平台 backend 目录执行：

```powershell
cargo test management_audit_tests -- --ignored
```

该命令会进行数据库集成验证；运行前应按[开发文档](development.md)准备专用测试环境。不要把源码中存在的测试当作当前部署已经执行过的审计证明。
