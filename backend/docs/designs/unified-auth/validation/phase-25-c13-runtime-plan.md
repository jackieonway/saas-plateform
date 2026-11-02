# Phase 25 C13 当前宿主、双标签页与通知最小验证方案

本文件是基于当前源码的只读方案，不是执行证据。它只覆盖 C13-S03/S05/S06/S07 所缺的实际宿主通信事实；C08/C12 的业务权限负例由独立业务 runner 负责。P/S/U/X/B 的证据边界见[层级审查](phase-25-evidence-layer-audit.md)。无需增加容器、数据库引擎或应用服务实例。

## 现有可复用入口

| 入口 | 直接用途 | 限制 |
| --- | --- | --- |
| [phase25-ui-browser](../../../../tests/unified-auth-https/phase25-ui-browser.mjs) | 冻结版本/集群绑定、真实员工登录和身份选择、共享 BrowserContext、多页面、CA/HTTPS观测、退出清理 | 当前不做微应用mount、实际宿主Socket切换 |
| [phase25-business-browser](../../../../tests/unified-auth-https/phase25-business-browser.mjs) | 真实BFF/context-token、应用session-token、实际业务API、真实通知发布及Engine.IO polling | 24447专属listener与手工polling不能证明宿主hook已连通 |
| [宿主MicroAppContainer](../../../../../frontend/src/components/micro/MicroAppContainer.tsx) | 当前 `/api/apps/session-token` → `/api/apps/{code}/context` → qiankun生产装配 | 不能用测试代码直接调用mount并伪造MicroProps替代 |
| [真实通知hook](../../../../../frontend/src/hooks/use-notification-socket.tsx) | 固定同origin `/socket.io/`，完整context捕获、switch时同步断开、回调代际复验 | 先尝试websocket，再polling；现有网关没有WS透传 |
| [BFF浏览器协调器](../../../../../frontend/src/lib/bff-session.ts) | `platform-bff-session-v1` BroadcastChannel、switching/settled、每次请求与focus复核authority | 只能由实际UI切换触发有效通知，不能手工伪造BroadcastChannel消息作为通过证据 |
| [当前工单真实UI](../../../../../../saas-work-orders/frontend/src/app.mjs) | `[data-testid="work-orders-app"]` 的 `data-org-id`、`data-context-active`，真实工单列表/详情及组织标题 | 用合成业务标题与DOM布尔结果观察，不导出token或MicroProps原文 |

## 最小fixture补齐

复用当前任务 `environment.ids.employee`、`organization`、`secondOrganization` 和 business manifest 中的 work-orders application/resource/menu。现有prepare只在首组织建立员工membership；business fixture的组织准入、角色、菜单和data grant也只在首组织。需由唯一环境owner在任务已验证的同一源库增加一个真实第二组织员工membership，并在第二组织建立当前工单app的准确准入、角色、user_roles、菜单授权与所需数据grant。若C12已有相同fixture，复用其准确ID，禁止另造重复membership。

首组织保留独立publisher BrowserContext，可用当前admin的首组织会话为该employee首组织发布合成通知。新组织employee使用新能力为自己新组织发布通知。这样同一user在两个真实member/org来源之间切换，无需改动既有用户、秘密或正式发布绑定；通知收件人必须由实际平台发布协议解析，不直接插入假事件。

工单详情还需要显式隐藏 `MENU`：`/micro/work-orders/tickets/:id`，绑定 `ticket:read`。宿主的 `microRouteAccess` 明确不允许父菜单隐式授权后代页面；现有business fixture只有根MENU和操作BUTTON。C13 control单独生成一个唯一详情菜单ID及其准确tenant、首/次组织、现有工单角色和本次新角色链接，finally精确删除该新菜单并核验级联链接残留为0；不放宽宿主路由权限。首次实际运行已通过宿主/真实微应用/双页/原生同origin WebSocket，但详情步骤超时，原FAIL报告保留；本补齐仍需当前新工具冻结后的真实重跑确认。

至少准备首/次组织各一条无敏感数据的工单记录，用于验证旧组织内容从真实微应用DOM移除、新组织内容出现。数据写入继续使用业务API，不由浏览器route拦截返回假列表。C08/C12 runner负责其自身权限与收缩负例；本方案只记录实际宿主访问使用了正确新应用上下文。

## 当前任务网关必须补齐的路由

当前[gateway.py](../../../../tests/unified-auth-https/gateway.py)把public console上非`/auth/`、`/api/`、`/micro-proxy/`路径作为宿主静态资源；因此 `/sub/work-orders/index.html`、`/work-orders-api/*` 和 `/socket.io/` 目前会得到宿主index，不能真实装配。单独[24447 business listener](../../../../tests/unified-auth-https/phase25-business-gateway.py)不能修复固定同origin的宿主hook。

| 精确public console路径 | 真实目标 | 约束 |
| --- | --- | --- |
| `/sub/work-orders/*` | 同一已冻结候选的工单frontend/dist | 只GET/HEAD；严格路径归属/无穿越；不存在资源404，不回退宿主index；不读取当前工作区dist |
| `/work-orders-api/*` | 已记录的本次工单进程127.0.0.1:23004 | 保留真实响应/状态，不生成身份声明；只转发有限请求/响应 |
| `/stores-api/*` | 已记录的本次门店进程127.0.0.1:23031 | 仅实际挂载门店时需要，最小工单方案可不增加 |
| `/socket.io/` 的Engine.IO query | 已记录的本次通知进程127.0.0.1:23003 | GET/POST/OPTIONS polling，且支持真实WebSocket升级透传；保持准确Origin，不伪造authenticated/event |

身份/平台私有proof继续只在24444，公开入口拒绝不变。通知宿主hook固定`io(window.location.origin)`，已有Python代理会移除Upgrade/Connection并缓冲完整HTTP body，不能直接用于WebSocket。需要任务专用、受控的WS升级与双向字节透传，或本机已可用且纳入资源记录的真实代理；不得修改生产前端transport或用response fixture绕开。WS代理必须有限连接、超时、明确关闭与finally清理；未实现该布线时C13-S07实际宿主通知仍应PENDING。

## 最小执行序列及逐条断言

1. 在既有SOURCE状态、TOTP enrollment/密钥或恢复破坏性步骤之前，使用同一BrowserContext建立tab A和tab B，真实employee登录首组织。两个页面都加载冻结宿主 `/micro/work-orders`。等待实际session-token/context、微应用静态脚本和业务HTTP成功，DOM中`data-org-id`准确等于首组织、`data-context-active=true`，通知铃铛出现`aria-label="实时推送已连接"`。记录当前BFF/member/version与应用旧凭据仅在进程内，用布尔值或不可逆摘要输出。
2. 在tab A通过真实顶部按钮`aria-label="前往认证中心切换租户和组织"`启动集中切换，暂时停在真实identity候选页面，不拦截响应、不伪造消息。tab B收到生产BroadcastChannel switching后，应显示“正在切换组织...”并移除/冻结旧微应用，旧通知socket断开，期间不能发起新业务写入。此时原BFF可能仍返回旧来源，页面仍须冻结，不能被一次旧GET提前解冻。
3. tab A在真实identity页面选第二组织的准确employee membership并完成回调。两个tab最终都必须通过真实 `/auth/session` 重验同一新membership/version；微应用显示新`data-org-id`，旧组织合成记录消失，新组织记录可见。检查新的session-token/context与实际业务请求使用新准确来源，旧app bearer经实际API拒绝。不要把contextEpoch变化引起的正常unmount/remount强行称作同实例update。
4. 在同一组织内通过真实工单列表进入已有工单详情再返回列表，保留宿主挂载容器identity，同时观察实际工单DOM视图变化和必要HTTP。此路径由真实宿主routePath向qiankun lifecycle.update传递，补充mount后update证据。若验证token/context自动刷新，等待实际context续期请求及新的截止/签名布尔变化即可；不得导出完整props或注入生产模块私有对象。
5. 由独立首组织publisher发布首组织合成通知，随后用新employee app能力发布次组织合成通知。旧socket不得重新认证或向新页面展示首组织通知；两个tab的新socket应经实际context-proof认证，只接收次组织准确recipient事件。页面和真实HTTP收件箱同时核对；断言旧标题不存在、新标题出现及通知数量来自服务端，不靠人工socket.emit或DOM塞消息。
6. tab A真实用户菜单选择“退出本平台”，tab B必须失去业务与微应用访问、断开通知，后续旧标签页操作不能覆盖/复活已退出Cookie。若还需验证旧迟到HTTP成功，优先使用已有可控promise组件回归支持C13-S04；不要为了它扩大真实网关的响应替换能力。

步骤2的“停在身份选择页”是自然且确定的switch窗口，避免用任意sleep或拦截真实成功响应制造冻结。DOM/网络/服务端计数应分别记录，失败时保存固定脱敏错误码；不写Cookie、token、verifier、私钥、完整props、storage-state或原私密日志。各页面/Context/browser在finally关闭，本方案新产生的临时端口、代理、lease和日志由同一环境owner纳入清理。

## 验收层级与未覆盖项

上述实际运行可分别支持C13-S03冻结、S05真实双tab/Cookie、S06当前宿主/SDK/micro装配和S07实际通知来源变化；它不代替C13-S01门户本地提交失败的跨库恢复，也不以一条业务读成功替代C08/C12全部授权矩阵。C13-S02的持久幂等、S04迟到回调算法继续使用当前精确PG/组件证据，报告同时保留各层级与输入绑定。
