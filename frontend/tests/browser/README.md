# 平台浏览器检查

本目录维护独立的 Playwright 1.64.1 和 Socket.IO 4.8.4 工具依赖。它们不进入宿主生产依赖。测试只使用 SSO／OIDC BFF 会话，使用真实 Vite/React 页面和隔离 HTTP fixture；通知场景另外创建隔离 Socket.IO 服务，不连接 PostgreSQL 或启动业务后端。

Phase 18 专项入口为 `npm run test:auth:context:browser`，也包含在通知完整入口中。专项同时运行两个标签页、当前宿主与真实 qiankun/SDK3，覆盖切换期冻结、卸载重挂载、transport 忽略 Abort 的迟到结果、保留的旧写回调、通知新来源、跨账号和员工／客户隔离，以及取消时原来源有效／无效两条路径。它模拟身份选择和 BFF HTTP 权威，不能替代平台／工单的真实数据库私有通知测试或最终全服务 HTTPS 验收。

以下命令均在 `frontend` 目录运行，使用 Node 24.22.1、npm 11.16.0：

```powershell
npm ci --ignore-scripts --no-audit --no-fund
npm ci --prefix tests/browser --ignore-scripts --no-audit --no-fund
node tests/browser/install.mjs chromium
node tests/browser/run.mjs --browser chromium
```

Linux 下载浏览器时可追加 `--with-deps` 安装系统依赖。项目下载缓存为本目录下的 `.browsers`，已由 Git 忽略。运行工具拒绝任意外部 `PLAYWRIGHT_BROWSERS_PATH`，仅接受项目缓存，或 Linux 固定 CI 镜像层 `/opt/ci/browser/.browsers`。

## 浏览器与完整入口

`--browser` 支持 `chromium`、`chrome`、`msedge`、`firefox`、`webkit`。Chrome、Edge 使用产品通道，需要事先安装对应浏览器；运行时不会自动降级为其他产品。已安装 Edge 时可以直接执行：

```powershell
node tests/browser/run.mjs --browser msedge
```

完整入口依次运行九套检查，每套最长 300 秒；任意套件失败会让入口失败。入口会清除通知、路由和导航的诊断筛选变量，避免局部筛选被误认为完整通过。

| 套件 | 单独入口 | 主要范围 |
| --- | --- | --- |
| 通知 | `npm run test:notifications:browser` | HTTP/Socket.IO 上下文、通知交互 |
| 平台设置 | `npm run test:platform:browser` | 配置页面与微应用上下文联动 |
| 应用导航 | `npm run test:auth:navigation:browser` | 应用选择和授权导航 |
| 登录会话 | `npm run test:auth:login:browser` | SSO 跳转、认证中心表单、平台回调、员工选择、返回路由、Cookie 刷新恢复及退出 |
| 地区位置 | `npm run test:regions:browser` | 地区及位置交互 |
| 实体类型 | `npm run test:entities:browser` | 实体表单与类型 |
| 部门上下文 | `npm run test:departments:browser` | 组织切换与部门操作 |
| 路由权限 | `npm run test:auth:routes` | 宿主、微应用、外链路由授权 |
| qiankun | `npm run test:micro:qiankun:browser` | 真实 qiankun 加载与撤销 |

单独入口通过环境变量选择浏览器，默认是 Chromium：

```powershell
$env:SAAS_TEST_BROWSER = 'msedge'
npm run test:micro:qiankun:browser
```

## qiankun 专项

`tests/micro-qiankun-browser.mjs` 启动自己拥有的 Vite 服务，在隔离浏览器上下文中使用真实 qiankun 和 DOM，覆盖四组行为：

1. HTML/经典脚本加载、`platform`/`services` props、`update`、撤销与卸载清理；
2. 同一入口首次 503 后修复资源并重试，拒绝复用失败缓存；
3. 动态脚本请求不携带平台 Authorization/Cookie，撤销后迟到资源不能执行；
4. 动态资源通过属性或 `setAttribute` 设置地址时，授权租约替换和重挂载不能复用旧租约脚本。

该脚本直接测试 qiankun 适配器，使用 fixture 构造 MicroProps。`npm test` 中另有容器级回归，执行实际容器 effects、授权 runtime 和生命周期队列，覆盖完整 pathname/search/hash、query/hash 单独更新、授权等待期间导航和权限隔离。两类检查的覆盖范围分别记录。取消场景只允许预期的取消/AbortError，其余页面错误会导致失败。

### 独立仓库的真实产物

下面的显式入口额外验证六份真实 HTML/UMD 产物的渲染、更新与卸载：Vue、jQuery、报表、WebSocket、工单、门店。

```powershell
$env:SAAS_TEST_BROWSER = 'msedge'
node tests/micro-qiankun-browser.mjs --all-apps
```

要求 `saas-plateform`、`saas-examples`、`saas-work-orders`、`saas-stores` 位于同一父目录。先依照各仓库 README 安装前端依赖，并构建 `saas-examples` 中的以下产物：

| 示例 | 预先存在的产物目录，相对于工作区父目录 |
| --- | --- |
| Vue | `saas-examples/micro-vue/dist` |
| jQuery | `saas-examples/micro-jquery/dist` |
| 报表 | `saas-examples/report-center/dist` |
| WebSocket | `saas-examples/websocket/dist` |

工单和门店会由脚本调用各自 `scripts/build.mjs`，把本次宿主来源加入构建参数并输出到临时目录，结束后清理。宿主已移除 `public/sub-apps` 示例副本，该入口仅读取独立仓库产物；通过不证明数据库授权、业务 API 或正式网关已经接通。默认单仓库 CI 不执行这个跨仓库场景。

## 应用接入专项

```powershell
$env:SAAS_TEST_BROWSER = 'msedge'
npm run test:applications:browser
```

该专项不在九套完整入口之内，需要单独执行。它使用隔离 API fixture 验证 qiankun 应用创建、直接地址、资源发现、发布激活、授权配置的忙碌状态、缺项提示和移动端布局；实际授权事务由后端 PostgreSQL 测试覆盖。

## 具体成员与准入策略专项

以下两个显式入口不包含在上面的九套完整入口中。报告目录必须为可写的绝对路径，脚本拒绝把报告写入源码、测试或依赖目录。

```powershell
$env:SAAS_TEST_BROWSER = 'msedge'
$env:IDENTITY_MANAGEMENT_BROWSER_REPORT_DIR = Join-Path (Get-Location) 'target/identity-sso-phase05/members'
npm run test:identity:management:browser
$env:ADMISSION_POLICY_BROWSER_REPORT_DIR = Join-Path (Get-Location) 'target/identity-sso-phase05/admission'
npm run test:admission:policy:browser
```

成员专项运行实际 `UsersPage` 与成员操作组件，验证分视图、创建开关、账号绑定、独立启停、第二身份／替换、旧版本拒绝及员工 bulk 保护。准入专项运行实际 `ApplicationsPage`、宿主 session gate 和 QueryClient，验证应用成员集合、版本冲突、影响确认，以及保存后立即撤下旧授权投影、清空缓存并中止活动请求。两者均保留 React StrictMode，生成 JSON 结果和桌面／手机截图。

这些场景使用隔离 HTTP fixture。后端成员与角色、真实策略撤销、迁移及通知服务之间的安全结论，分别以 PostgreSQL 和真实双服务证据验收，见[当前初始化与部署](../../../backend/docs/fresh-installation.md)及[当前初始化与部署](../../../backend/docs/fresh-installation.md)。

## 用户资料与员工信息编辑专项

```powershell
$env:SAAS_TEST_BROWSER = 'chromium'
$env:USER_EDITING_BROWSER_REPORT_DIR = Join-Path (Get-Location) 'target/user-editing-browser'
node tests/user-editing-browser.mjs
```

该独立专项运行生产 `UsersPage`、`CustomersPage`、详情与编辑对话框，使用隔离 HTTP fixture 验证姓名、邮箱、电话、性别保存与清空，失败后草稿保留及重试，员工部门／岗位维护、客户关系保留、角色分配、详情转编辑，以及新增员工仍绑定既有账号。客户页另以 390px 视口确认资料可编辑且不出现员工关系或角色操作。脚本保留 JSON 报告和桌面／手机截图；报告目录须为可写绝对路径，不能覆盖源码、测试或依赖目录。其 Vite 和浏览器进程在 `finally` 中关闭，不创建 Docker 资源。该专项不包含在九套完整入口中，也不替代真实后端、数据库或部署验证。

## 统一客户端注册簿

```powershell
$env:SAAS_TEST_BROWSER = 'chromium'
$env:UNIFIED_CLIENTS_BROWSER_REPORT_DIR = Join-Path (Get-Location) 'target/unified-clients-browser'
npm run test:identity:clients:browser
```

该检查运行实际统一客户端页面、生产 SessionStatusGate 与 QueryClient，API 使用隔离 fixture，不连接数据库或业务后端。覆盖地址筛选与服务器分页恢复、详情、USER/MACHINE 表单边界、失败刷新保留旧记录、同帧重复提交、409 版本冲突、390px 编辑器滚动、机器客户端创建、未知凭据结果的原 operation 查询，以及 secret 不进入 URL、localStorage、sessionStorage 或 Query 缓存。运行后保留浏览器 JSON 报告与桌面／手机截图；报告目录须为可写绝对路径且不得指向源码、测试或依赖目录。

## 合作方接入与账号邀请

```powershell
npm run test:identity:intake
$env:SAAS_TEST_BROWSER = 'chromium'
$env:IDENTITY_PHASE19_BROWSER_REPORT_DIR = Join-Path (Get-Location) 'target/phase19-intake-browser'
npm run test:identity:intake:browser
```

Phase 19 专项运行实际统一注册簿与接入面板，复用生产会话协调和 QueryClient，使用独立 HTTP fixture。覆盖最小账号 scope／精确回调申请、独立审核退回与测试批准／验收、申请人不可自批、缺审核权与二次认证失效时的控制、生产责任未绑定的关闭状态、409 草稿保留和明确载入最新状态，以及 390px 申请／邀请表单、邀请签发／撤销和未知结果的原 operation 恢复。邀请页面只接收严格的管理元数据，拒绝 token、密钥及投递材料字段进入页面或 Query 缓存；客户端凭据轮换继续使用原统一入口。

平台重新验证专项核对同源固定入口、唯一返回地址、离开页面前的草稿提示、取消后草稿保留，以及首次 TOTP 绑定与平台重新验证的不同用途。HTTP fixture 不执行最终导航、SSO 登录或 TOTP，也不证明新 BFF 会话与认证来源已建立；这些正向流程须由真实 HTTPS 环境单独验收。

报告目录必须是可写绝对路径，不能覆盖源码、测试或依赖目录。专项创建一个本地 Vite 进程与临时浏览器上下文，并在 `finally` 关闭，不启动容器或业务服务。报告保留 JSON 与截图；HTTP fixture 的人工证明引用和生产门禁数据属于测试输入，不构成实际域名核验、运营人员绑定、数据库授权或邀请投递证据。

## 结果解释

脚本输出浏览器产品、版本和 Playwright 版本。保留实际执行命令、产品版本、PASS/FAIL 和失败日志，再报告覆盖范围。`npm test` 是另一组 Node 测试；CI 当前只运行 Node 测试和构建，不安装浏览器、也不执行本目录完整入口。浏览器 fixture 通过、真实产物通过、数据库通过和正式部署通过需要分别记录。
