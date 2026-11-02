# qiankun 微应用接入

平台当前只接受 `HTML + TRUSTED`，独立应用使用 `/micro/{code}/*`。宿主以 `loadMicroApp` 手动挂载，由现有路由和授权运行时决定何时启动、更新与卸载。接入前先确认子应用代码可信；qiankun 沙箱不提供独立浏览器进程的隔离。

## 构建契约

新接入交付 HTML 和 classic/UMD JavaScript，公开导出 `bootstrap`、`mount`、`update`、`unmount`。源码可以使用 ESM、TypeScript、React 或 Vue，最终 HTML 不能保留 `script[type=module]`。平台预检和浏览器加载都会拒绝该类入口。当前宿主还保留缺少 `update` 时销毁重挂的兼容处理，新应用应实现四个生命周期以支持原位上下文更新。

qiankun 传参的职责如下：

| 字段 | 使用规则 |
| --- | --- |
| `props.container` | 仅在本次容器内查找挂载点、创建和销毁视图 |
| `props.platform` | MicroProps 2.0：应用 token、用户/租户/组织、菜单权限、数据授权、路由主题、期限、signal 和回调 |
| `props.services` | 独立宿主能力，如受权限及数据授权约束的 CSV 下载 |

公共 SDK 为 3.0.0；应用授权上下文和通知协议仍为 1.0。消费者使用自己仓库固定发布物，不运行时读取相邻 SDK 源码。SDK 只协助读取上下文和发送请求，业务后端仍须逐请求授权。

挂载只初始化当前容器；更新处理 token、组织、主题和路由变化；卸载清理请求、定时器、DOM监听与 Socket。请求绑定当前 signal，避免旧上下文响应覆盖新页面。业务请求使用应用 Bearer，凭据不放 URL/持久存储；静态资源请求省略凭据。

## 参考工程

| 编码 | 源工程 | 默认开发入口 |
| --- | --- | --- |
| `work-orders` | 工单 `frontend` | `http://127.0.0.1:3013/index.html` |
| `stores` | 门店 `frontend` | `http://127.0.0.1:3014/index.html` |
| `vue-workshop` | 示例 `micro-vue` | `http://127.0.0.1:3005/index.html` |
| `legacy-jquery` | 示例 `micro-jquery` | `http://127.0.0.1:3006/index.html` |
| `report-center` | 示例 `report-center` | `http://127.0.0.1:3009/index.html` |
| `websocket-chat` | 示例 `websocket` | `http://127.0.0.1:3007/index.html` |

各工程单独启动，平台不自动创建示例应用或授权。宿主旧 `public/sub-apps` Vue/jQuery 副本已删除；示例使用上述独立工程构建的产物。门店历史包保留原字节，本地发布工具只新增并验证 qiankun 版本，不覆盖历史目录，详见[当前初始化与部署](fresh-installation.md)。

## 后台接入流程

1. 在应用管理新建微应用，填写名称与不可变编码。默认生成 `/micro/{code}` 和首页菜单；图标、描述、排序等按需设置。
2. 选择目标环境，配置直接 URL 或发布固定版本。环境名是小写字母、数字、下划线/中划线，长度 1–32，首位为字母/数字。
3. 检查入口及页面，再指定租户、组织、普通角色与菜单，确认追加授权。
4. 为实际用户分配对应组织角色，使用普通用户验证访问。业务调用另外配置功能权限、数据授权、独立 APP_RESOURCE 服务凭据和业务 API。

接入面板选择的环境用于配置与检查，实际宿主消费环境由服务端 `APPLICATION_ENVIRONMENT` 决定。

## 两种入口管理方式

**直接 URL** 适用于随上游部署变化的环境地址。保存完整 HTTP(S) HTML 文件或以 `/` 结尾的目录，origin 先列入服务端 `MICRO_ALLOWED_ORIGINS`；默认开启静态代理。该环境优先使用 URL override，移除后恢复版本/登记入口。直接配置就绪只证明格式、来源、权限正确，没有执行完整 release 资源预检。详见[环境 URL](environment-url-entries.md)。

**固定版本** 适用于需预检、激活和回滚的交付。可以上传 ZIP/tar.gz，或登记已部署 HTML 地址。version 为 1–64 个字母/数字/点/下划线/中划线，首位字母/数字，禁止 `..`；入口必须包含完全相同的版本目录段。例如版本 `1.0.0` 使用 `/sub-apps/crm/versions/1.0.0/index.html`。

服务端分析 HTML 的脚本、样式等资源，无需手填 JS/CSS 列表。动态计算资源和额外健康地址可补充检查项；服务端不会执行任意业务 JS 来枚举所有依赖。上传方式还验证包内完整清单；ZIP 分发和镜像方式需要相应节点配置。

登记后预检，预检通过后明确激活。激活/回滚清除同环境 URL override，切换固定版本指针。登记成功但预检失败应重试原记录；结果未知时先刷新核对。历史 iframe/ESM release 只读，不能重新预检、激活或回滚。

## 页面、功能和数据授权

菜单访问是租户、组织、角色三层交集；同一个有效入口菜单必须通过完整链路。接入面板只追加本应用选中的菜单与授权，不撤销其他授权。托管 `tenant_admin`、`org_admin` 由平台维护，不作为普通角色手工编辑。

菜单可见不自动赋予业务数据访问。独立后端核对应用身份、功能 permission、资源/动作数据授权、组织边界和期限；APP_RESOURCE 私有服务凭据只保存在业务服务端。配置入口不等于所有用户可以使用应用。

| 平台接口 | 用途 |
| --- | --- |
| `GET /api/applications/{id}/setup?environment=...` | 基础入口和菜单检查；追加 tenantId/organizationId/roleId 可核对特定目标 |
| `GET /api/applications/{id}/setup/organizations?tenantId=...` | 最小有效组织候选 |
| `POST /api/applications/{id}/setup/access` | 提交 tenantId、organizationId、roleId、menuIds 追加访问授权 |

基础检查需要平台根组织的 `system:application`。配置目标范围还需 `system:tenant`、`system:authorization`、`system:role`。事务在持锁和提交前复验会话/权限，审计失败回滚。

## 资源与部署验收

资源 publicPath 与业务路由 base 分开：`/micro/{code}` 是用户导航，`/micro-proxy/*` 或版本目录是资源入口。根绝对路径 `/assets/*` 不会自动改写到子应用。图片、字体、动态 chunk 需正确 base 或内联，见[静态代理](micro-url-proxy.md)。

qiankun 的动态脚本执行需要与宿主 CSP 兼容，网关和宿主 Nginx 示例已默认支持可信 HTML 求值，并保持外部来源限制；上线前仍按[部署文档](deployment.md)核对最终响应头。跨域业务 API 的来源是宿主 origin，配置必须显式填写该来源，`Origin:null` 不再是活动接入模式。

至少验证普通用户挂载、同应用路由更新、组织/token替换、撤权、故障重试和卸载。适配器专项不能代替真实宿主、真实子应用与最终网关一起联调。
