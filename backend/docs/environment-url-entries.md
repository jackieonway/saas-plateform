# 环境 URL 入口

环境 URL 入口让平台在某个环境加载一个可变地址的 qiankun HTML 微应用，适合独立部署和开发联调。它与固定版本 release 共存：URL 记录优先于该环境 active release，移除 URL 记录后恢复按 release 或应用默认入口解析。

实现位于 `src/application_entries.rs`，解析函数由 `migrations/0001_init.sql` 定义；静态资源代理位于 `src/micro_proxy.rs`。固定版本交付见[ZIP 分发与发布](zip-distribution-runbook.md)，代理行为见[微应用 URL 代理](micro-url-proxy.md)。

## 支持条件与选择顺序

独立微应用必须是 `entryType=HTML`、`runtimeMode=TRUSTED`，且 `basePath` 为 `/micro/{appCode}`。内建平台路由不通过这个入口加载。现行启用应用的数据库约束要求 HTML + TRUSTED；旧 iframe/ESM 数据只用于历史迁移和明确重新配置，不是可选择的加载方案。

平台使用自己的 `APPLICATION_ENVIRONMENT` 解析有效入口，不接受普通页面请求临时指定环境。当前选择为：

1. 该环境存在 URL entry：使用该记录，代理开关决定返回原始地址还是代理地址。
2. 没有 URL entry 且有 active release：使用该 release 的入口、版本和 ID。
3. 没有以上记录：使用应用默认入口；内建平台路由始终返回空微应用入口。
4. 独立微应用元数据不符合 HTML + TRUSTED：有效入口返回空，不回退旧加载器。

SQL 返回的 source 名称可能为 `URL`、`RELEASE`、`LEGACY`；其中 LEGACY 表示应用表默认入口来源，不表示 iframe 运行模式。历史迁移 `0008` 中的旧条件已被 `0009`、`0010` 覆盖，判断现行行为应读取最新函数定义。

## 管理 API

```http
GET /api/applications/<application-id>/url-entry?environment=development
PUT /api/applications/<application-id>/url-entry?environment=development
DELETE /api/applications/<application-id>/url-entry?environment=development
Authorization: Bearer <platform-session-access-token>
```

三种操作都要求 `system:application` 和平台租户根组织上下文。PUT/DELETE 在数据库锁等待后检查真实会话及当前管理权限。environment 为 1–32 位小写字母、数字、下划线或中划线，首字符必须为字母或数字。

PUT 只接受：

```json
{
  "entryUrl": "http://localhost:5174/",
  "proxyEnabled": true
}
```

`proxyEnabled` 可省略，默认 true。示例 origin 必须先由运维加入当前环境的 `MICRO_ALLOWED_ORIGINS`；不是任意可提交的上游地址。GET 在没有记录时返回 JSON null；DELETE 成功返回 null。保存后返回：

```json
{
  "applicationId": "11111111-1111-4111-8111-111111111111",
  "environment": "development",
  "entryUrl": "http://localhost:5174/",
  "proxyEnabled": true,
  "revision": "22222222-2222-4222-8222-222222222222",
  "updatedAt": "2026-10-23T12:00:00Z",
  "browserEntryUrl": "/micro-proxy/work-orders/22222222-2222-4222-8222-222222222222/"
}
```

每次实际改变地址或代理开关会生成新 revision；完全相同的保存保持 revision 和时间不变。该接口没有 expectedVersion 参数，不能把 revision 当成 PUT 的乐观锁。

## URL 校验

地址必须是最终 HTML 文件（.html/.htm，忽略大小写）或以 / 结尾的目录，必须是完整 HTTP(S) URL。服务端校验：

- origin 在 `MICRO_ALLOWED_ORIGINS` 精确 allowlist 中；production 环境要求 HTTPS。
- 禁止用户名、密码、fragment、反斜线、控制字符，序列化后最多 2000 字符。
- query 参数名称不允许 `token`、`access_token`、`session`、`session_token`、`client_secret`，以及平台保留的 `__platform_entry`、`__saas_execution`、`__saas_lease`，检查不区分大小写。
- 平台托管的 `/sub-apps/{app}/versions/...` 固定版本目录不能用作可变 URL entry，应使用 release 激活和回滚。

保存 URL 配置不等于完成 release 预检，不会生成版本制品、不可变 release 或生产发布证明。入口服务仍需提供可被 qiankun 加载的 HTML、生命周期及正确资源路径。

## 浏览器代理入口

开启代理时，browserEntryUrl 为 `/micro-proxy/{appCode}/{revision}/`，上游 query 会保留。代理只根据当前环境已登记、已启用且符合 qiankun 元数据的记录选上游；URL 中不能指定其他 host。revision 变化后旧地址不继续解析当前配置。

代理是匿名静态资源通道，不转发平台或业务身份，不承担业务 API、cookie 或 WebSocket 代理。关闭代理时浏览器直接访问 entryUrl，微应用部署需要自行满足跨域资源加载和网络访问条件。无论采用哪种方式，应用身份都通过 SDK 2.0 和应用专用令牌传递，不能塞进 URL。

有效应用上下文的 `application.entryUrl` 是浏览器可用入口。URL entry 对应 `deploymentVersion=null`、`entryVersionId=revision`；固定 release 则返回相应版本和 release ID。入口 revision 用于识别配置变化，不证明上游内容不可变。

## 与发布、回滚和审计的关系

激活或回滚 release 时会在指针事务内清除同一环境的 URL override，避免新指针被旧可变入口遮蔽。若清理或审计失败，入口与发布指针一起回滚。仅删除 override 不删除 release 历史或原制品。

配置变更动作 `CONFIGURE_APPLICATION_URL_ENTRY` 使用事务审计，保存前后只记录应用/环境、代理开关、revision 和是否已配置，不把带查询参数的上游地址写入审计。读写权限详见[管理审计](management-audit.md)。

## 联调与排查

开发联调时先启动微应用 HTML 服务，登记其精确 origin，再用管理界面或 API 保存当前环境 URL。打开平台 `/micro/{appCode}`，从应用上下文确认 entryUrl 和 entryVersionId，再检查 HTML、JS、CSS 的实际响应与 qiankun 生命周期。

出现空入口时检查应用状态、HTML/TRUSTED 元数据、basePath 和当前 APPLICATION_ENVIRONMENT；出现旧页面时核对 revision、环境以及 URL override 是否仍优先；代理 404 时核对记录是否删除、revision 是否过期、应用是否停用。生产启用失败应检查 HTTPS 与 allowlist，而不是放宽为任意 origin。

回归位于 `src/release_tests/url_entries.rs`：环境与 revision、一致上下文、审计失败回滚、激活/回滚清除 override、qiankun HTML 上传及锁等待后的权限撤销。`src/micro_proxy/tests.rs` 覆盖代理边界。数据库测试需要隔离 PostgreSQL 环境，见[开发文档](development.md)。
