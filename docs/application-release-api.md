# 应用独立发布 API

Phase 9 将应用身份和授权、发布版本、当前部署分别保存。`applications.id` 是授权与应用 token 的稳定身份；`application_releases` 保存某个应用、环境、版本的不可变入口；`application_deployments` 仅保存当前和上一版本指针。注册、预检、启用、回滚均不增删租户、组织、角色、菜单及数据授权。

## 管理权限与运行模式

以下管理 API 均使用平台 session Bearer token，同时要求 `system:application` 权限和平台租户上下文；超管可执行。普通租户管理员即使拥有应用管理权限，也不能调用发布管理 API。

应用定义的 `runtimeMode` 为 `IFRAME | TRUSTED`。创建时缺省为 `IFRAME`；0021 迁移也将所有存量应用设置为 `IFRAME`。平台管理员审核自有代码后，显式调用 `PUT /api/applications/{id}`，传入 `{"runtimeMode":"TRUSTED"}`，才能让宿主执行该应用的 HTML/ESM。仓库 Vue、jQuery、report-center 示例升级后也需要管理员逐个明确标记，不依据应用编码或入口 URL 自动推断信任。

`IFRAME` 应用只能启用 HTML 发布版本；ESM 的启用和回滚都要求 `TRUSTED`。运行模式实际变更会撤销该应用当前的应用 token，下一次内省或授权上下文请求即拒绝旧 token；平台 session 保持有效。重写相同运行模式、普通版本启用和回滚不会撤销应用 token。0021 新增默认 IFRAME 字段本身保留已有应用 token，存量 token 最长经过 5 分钟 TTL 兼容窗到期；升级宿主立即按默认 IFRAME 处理这些应用。随后任何明确的运行模式 UPDATE 均按前述即时撤销规则执行。

## 配置

| 配置 | 缺省值 | 作用 |
| --- | --- | --- |
| `APPLICATION_ENVIRONMENT` | `development` | `auth/profile` 为当前宿主选择的部署环境 |
| `MICRO_GATEWAY_ORIGIN` | `PUBLIC_APP_URL`，为空时 `http://localhost:3000` | 服务端访问相对入口使用的网关 origin |
| `MICRO_ALLOWED_ORIGINS` | 仅 `MICRO_GATEWAY_ORIGIN` | 逗号分隔的精确 HTTP(S) origin，包含端口；入口、资源、健康检查及每一次 redirect 都必须命中 |

网关及允许来源只能填写 origin，例如 `https://saas.example.com`，不能包含路径、query、fragment 或用户名/密码。允许列表配置授予服务端访问该来源的能力，应按部署需要列出网关/CDN/独立服务的来源；用户提交的 URL 无法扩展此列表。预检禁用系统 HTTP 代理。

宿主环境或注册环境为 `production` 时，所有预检 URL 必须为 HTTPS。远程来源不在缺省允许列表中；开发环境本地 HTTP 也须与配置的 origin 完全一致。

## 版本注册

`POST /api/applications/{id}/releases`

```json
{
  "environment": "development",
  "version": "1.0.0",
  "entryType": "HTML",
  "entryUrl": "/sub/vue/1.0.0/index.html",
  "healthUrl": "/micro-services/vue/health",
  "resources": [
    {"url":"./assets/main.js","kind":"SCRIPT"},
    {"url":"./assets/main.css","kind":"STYLE"},
    {"url":"./assets/logo.svg","kind":"ASSET"}
  ]
}
```

`environment` 是 1–32 位小写字母、数字、下划线或中划线，首位为字母/数字。`version` 是 1–64 位字母、数字、点、下划线或中划线，首位为字母/数字，不能包含 `..`。应用、环境、版本组合唯一，重复注册返回 409。

入口 URL 的路径必须包含与 `version` 完全相同的目录段，例如 `/sub/vue/1.0.0/`。入口不接受 query/fragment；HTML/ESM 入口 redirect 必须仍在该版本目录内。HTML redirect 还须保持原始来源和入口所在目录，`base[href]` 也须保持相同资源目录，保证预检和宿主 HTML 加载器解析到相同 URL。同源脚本和样式必须保留在版本目录，避免加载 `/latest` 或被其它发布覆盖的可变资源。版本目录里的文件应作为不可变归档保存，发布新内容应使用新版本目录；平台数据库的不可变约束不代替文件存储的写入控制。

`entryType` 为 `HTML | ESM`，`healthUrl` 可省略或为 `null`，`resources` 可省略，显式清单最多 48 项；`kind` 为 `SCRIPT | STYLE | ASSET`。资源的相对 URL 和根路径按入口 URL/入口 origin 解析，顶层入口和健康检查的根路径按网关解析；资源保存为解析后的完整 URL。版本定义字段一经注册不可修改，数据库触发器也拒绝直接改写。API 不提供修改或删除版本的操作。

成功返回 200 及下文的发布行，初始状态为 `DRAFT`。不合法 URL、来源、版本路径或枚举返回 400；该操作还不会访问入口网络。

## 列表与预检

`GET /api/applications/{id}/releases?environment=development` 返回发布行数组；省略环境则返回全部环境。每行结构如下：

```json
{
  "id": "release-uuid",
  "applicationId": "application-uuid",
  "environment": "development",
  "version": "1.0.0",
  "entryType": "HTML",
  "entryUrl": "/sub/vue/1.0.0/index.html",
  "healthUrl": null,
  "resources": [],
  "validationStatus": "DRAFT",
  "validationResult": {},
  "checkedAt": null,
  "active": false,
  "createdAt": "2026-10-02T00:00:00Z"
}
```

`POST /api/applications/{id}/releases/{releaseId}/preflight` 由平台服务端实际发起 HTTP GET。返回 200 及更新后的发布行；检查不通过也返回可诊断的 `FAILED` 发布行，便于管理界面展示并保存故障。此处 HTTP 200 只表示预检操作完成，必须检查 `validationStatus`。

检查 HTML/ESM 入口的状态和 MIME；通过真实 HTML 解析读取 `script[src]`、stylesheet、modulepreload、preload script/style，并按入口解析 URL。显式 resources 也逐个 GET。SCRIPT 接受 JavaScript/ecmascript MIME，STYLE 仅接受 `text/css`，ASSET 必须有非 HTML MIME。入口或资源为空、缺失文件 404、错误 MIME 或健康检查非 2xx 均失败。SPA fallback 返回的 HTML 不能冒充脚本/样式/其它静态资源。

TRUSTED 入口、脚本和样式通过宿主 fetch 或模块加载使用时，跨源响应还必须返回 `Access-Control-Allow-Origin: *` 或精确宿主网关 origin；预检发送对应的 `Origin` 并检查每一步响应，避免服务器可访问而浏览器被 CORS 拒绝。IFRAME HTML 顶层导航不要求宿主 fetch CORS。图片等 ASSET 只检查可用性和非 HTML MIME；它们的具体浏览器使用方式由应用决定。

TRUSTED HTML 的现有加载器以 classic script 方式执行脚本，预检会拒绝任何外部或内联 `type="module"` 以及原生 modulepreload；标准 Vite 模块产物应选择 IFRAME HTML，或经管理员明确信任后使用 ESM 入口。IFRAME 在不允许同源身份的 sandbox 中具有 opaque origin，原生模块、modulepreload、声明 crossorigin 的脚本/样式，以及模块文档的显式 SCRIPT 依赖清单必须支持 `Origin: null`，响应须允许 `Access-Control-Allow-Origin: *` 或 `null`。发现的普通 classic script 不额外强制 CORS。发布资源采用匿名获取，`crossorigin=use-credentials` 不在本接口的预检支持范围内。

IFRAME 入口的 X-Frame-Options 与所有生效 CSP frame-ancestors 策略须允许宿主网关 origin；DENY、来源不匹配以及后续 CSP 策略的拒绝均产生持久 FAILED 诊断。该检查验证配置的宿主来源，实际应用若另嵌在其它门户中，还需在部署时检验完整祖先来源链。

HTML 运行时动态 import、CSS 运行时引用及 ESM 的传递依赖须在发布时提供完整的显式 `resources` 清单，例如由构建 manifest 生成；预检不执行 JavaScript，也不声称能够发现运行时产生的任意请求。平台控制的源 allowlist 同样用于显式清单与 HTML 中发现的 URL。

每个请求最多 4 秒、响应体最多 4 MiB；整个预检最多 30 秒、总响应最多 16 MiB、最多 64 项检查，单资源最多 5 次 redirect。redirect 由平台手动处理，每一步在请求前校验 origin、HTTPS 和凭据规则，不跟随未经配置的来源。

`validationResult` 含 `passed`、`checkedAt`、`checks` 和限制信息。每个检查含 `url/kind/passed/statusCode/contentType/bytes/message`；策略失败或超时时可能仅含 URL、类型、是否通过和诊断消息。成功/失败及 `checkedAt` 保存于数据库，前端不能提交 PASSED 声明。

## 启用与回滚

`POST /api/applications/{id}/releases/{releaseId}/activate` 首先要求该版本已通过预检；DRAFT/FAILED 返回 409。启用前再次真实预检，失败时持久化诊断并保留当前部署。通过后原当前版本成为上一版本，新版本成为当前版本。重复启用当前版本保留原上一版本。

`GET /api/applications/{id}/deployments` 返回当前已部署环境的数组。启用及 `POST /api/applications/{id}/deployments/{environment}/rollback` 成功返回单个部署行：

```json
{
  "applicationId": "application-uuid",
  "environment": "development",
  "activeReleaseId": "current-release-uuid",
  "previousReleaseId": "previous-release-uuid",
  "version": "1.0.0",
  "entryType": "HTML",
  "entryUrl": "/sub/vue/1.0.0/index.html",
  "updatedAt": "2026-10-02T00:00:00Z"
}
```

首次启用的 `previousReleaseId` 为 `null`。没有部署时回滚返回 404，没有上一版本时返回 409。回滚重新预检上一版本，失败则保存诊断并保持当前版本。回滚成功交换当前与上一指针，可再次切回；并发部署在预检期间发生变化则返回 409，要求重新请求。

部署切换对应用行加锁，active/previous 通过复合外键保证属于相同应用和环境。部署指针和启用/回滚审计在同一事务提交。注册记录审计也与注册事务一致；预检保存结果并写入审计。

## 已登录用户的入口

`GET /api/auth/profile` 在当前租户及组织两级应用授权内，读取 `APPLICATION_ENVIRONMENT` 的活动版本，返回入口及 `deploymentVersion`、`entryVersionId`、`runtimeMode`。没有活动版本时返回存量应用定义入口，两个版本字段为 `null`，作为迁移期兼容。profile 不因发布而新增应用权限。

宿主刷新 profile 后发现版本 ID 变化，应卸载旧实例并用完整新入口重新挂载；旧版本目录须保留，使已挂载用户的后续资源请求仍命中同一版本。回滚同样只更换入口和版本 ID，不重建宿主，也不重写授权或应用专用 token 受众。

`GET /api/apps/{appCode}/context` 的 `application` 也逐请求读取 `runtimeMode/entryType/entryUrl/deploymentVersion/entryVersionId`，并在完成计算后再次确认 token 有效。宿主必须以实时上下文核对缓存 profile 的运行模式，发现模式不一致先刷新 profile，阻止旧 TRUSTED 缓存继续执行。普通发布版本仍通过约 20 秒的 profile 刷新切换；已挂载实例在刷新之前继续使用自己的固定版本入口和资源。运行元数据与授权 hash 分离，版本切换本身不改变角色、菜单、权限、数据授权或 `contextVersion`。

## 验证与兼容

`backend/src/release_tests.rs` 使用根目录 Git 忽略的 `.env.test` 中 `TEST_DATABASE_URL` 连接账号，在随机独立 PostgreSQL 数据库运行迁移，使用 IPv6 loopback HTTP fixture 测试实际网络检查并在完成后清理测试库；不打印或保存数据库凭据。运行命令：

```powershell
cargo test --manifest-path backend/Cargo.toml release_tests -- --ignored --nocapture
```

2026-10-02 已在 Ubuntu WSL 完成 8/8 项真实测试，包含 7 个独立 PostgreSQL 库和 IPv6 HTTP fixture，所有库均清理成功。该桌面环境的 Windows 原生回环连接受限，WSL 的外部 PostgreSQL 大 TCP 段也发生传输丢失；以仅测试进程的 `TCP_MAXSEG=1000` 恢复传输后，保持相同 SQL、HTTP 数据和断言，全部测试用时 25.76 秒。此验证调整没有改动系统网络或防火墙，也没有加入产品运行配置；普通 Linux CI/部署环境按上面的标准命令执行。`RELEASE_TEST_TRACE=1` 可在排查时显示不含 token 的方法/路径进度。

0021 不修改历史迁移与现存授权表。原生后端 `STATIC_DIR` 模式仅为无文件扩展名的宿主 HTML 导航返回 SPA index；缺失 API/OAuth、微应用资源前缀和 JS/CSS/其它文件保持 404，现有静态文件仍按实际 MIME 返回。回退应用代码前应先导出/记录运行模式和部署指针，再按受控数据库变更流程移除新表、触发器及字段；不能修改已经执行过的 SQLx 迁移校验和。文件存储、网关版本路径和缓存规则见独立部署接入文档。
