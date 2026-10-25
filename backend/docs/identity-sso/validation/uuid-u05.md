# UUID-U05：实际 SDK、配置、凭据与本地运行验收

阶段：`COMPLETED`。范围仅为本地 WSL Ubuntu PostgreSQL 18.6 的三个 U04 隔离目标及其本轮 loopback 服务，不包含远程、生产、U06 最终切流或独立身份新库。本文是 U05 的汇总记录，组件报告中的早期 pending 状态、失败轮次和原始证据继续保留。机器汇总见 [uuid-u05-summary.json](uuid-u05-summary.json)，台账 v0.6 的接续指针为 UUID-U06（NOT_STARTED）。

## 本地运行边界

平台、门店、工单使用 U04 的固定目标和冻结的正常服务二进制，真实迁移历史仍为 12／3／4。平台当前工作树已有独立身份 Phase 1 的 0013，**没有**把它编译或运行到本专项目标。通知、OAuth 演练、chat 分别独立构建，并由摘要固定其实际执行字节。

公共 origin 为 `http://127.0.0.1:13000`，平台、通知、OAuth、chat、门店、工单的内部 loopback 端口分别为 13001、13002、13007、13008、13010、13011。保留原 3000 系列 Windows 服务；新服务容器属于 `qiankun-uuid-u05-*`。API 使用只读容器根目录、cap-drop ALL、独立 0600 日志和清空继承环境的 runner，chat 保持 stdin 生命周期。

三目标数据库的全局默认只读保持 `on`，owner 保持 NOLOGIN。三个业务 runtime 角色对自己的目标有显式可写事务默认和必要 DML；通知角色只有目标平台通知表 SELECT，保持只读。不能把“数据库默认只读”误写成“运行服务无法写入”。源 v4 库及原加密源包、转换包和映射继续留存。

## 配置与新凭据 epoch

真实 policy 通过独立保护的 `uuid-config-rekey` 对认证源包、转换包与类型化映射重验后生成，成功轮次为 `config-u05-02`。两个应用 policy 和一个 management scope 使用目标 UUID；其摘要和认证 manifest 与导入批次绑定。没有把 example grants 当作真实旧授权输入；业务授权由已导入数据和新 API 上下文验证。

旧 state 的完整封闭 DTO 检查失败于未知字段，失败轮次原样保留。没有删除未知字段后强行复用 state，也没有改写旧 JWT；整个旧 state 隔离，新状态由真实新登录和目标 API 重建。

维护事务轮换全部 12 个 CONFIDENTIAL 客户端密钥，7 个账号 session_version 各递增一次；JWT key、runtime role 密码及服务密钥独立重新生成。外部私有 journal 绑定新 epoch、完整导入 marker 与 key commitment，旧 state 不传给 launcher。原六类凭据表在 runtime 前均为零；原源表共 119 条凭据行不进入目标。运行后产生的合法新 session、code、device 与 token 不再要求零行。

配置、角色、epoch 和 253 文件实际 HTTP 回读的摘要见 [runtime 准备证据](uuid-u05-runtime-preparation.json)。保护目录为 `/var/lib/saas-uuid-u05-materials-20261024` 和 `/var/lib/saas-uuid-u05-runtime-20261024`；私有请求、响应、主体映射及秘密不进入公开报告。

## 启动与实际消费者

三个冻结服务使用正确 receipt 均已启动并返回健康 200，通知 ready 也为 200。启动前六个错误 marker／schema 进程均在端口空闲时 exit 1；同一冻结二进制的后续正向启动与独立 SQLx 只读诊断形成对照。平台 bootstrap 将所有 receipt 校验失败统一掩码为 `IMPORTED_DATABASE_VERIFICATION_FAILED`，不能从错误字符串推断底层数据库异常。没有修改 expected hash、schema、历史迁移或权限来绕过校验。见 [实际启动](uuid-u05-startup.json) 与 [独立诊断及其限制](uuid-u05-startup-diagnostic.json)。

初始 8 个前端共 253 个文件已构建并在实际网关逐字节回读。六个完整 vendor 和两个 classic SDK 均对应 SDK 3.0.0，新发布门禁及生成 DTO／来源摘要检查通过，216 个历史文件保原。见 [初始构建清单](uuid-u05-frontend-builds.json)。Node 24.22.1 从实际网关 classic SDK 和冻结 release ESM 各生成 1,000 个真实请求 ID，2,000 个均 RFC UUID v7 且唯一；见 [Node 实际消费](uuid-u05-node-sdk-runtime.json)。

七个微应用通过正式 API 登记新 release、服务端 preflight、activate 和 deployment 读回；显式 preflight 共 25 项，通过激活再检查。门店与五个示例使用 `1.0.1-uuid-u05`，工单使用 `2.0.1-uuid-u05`。原 URL override 通过正式激活清理，不只是修改会被覆盖的 applications.entryUrl。DTO 没有 sdkVersion 字段；SDK 版本由实际加载字节和 props 验证。见 [正式发布证据](uuid-u05-releases.json)。

真实浏览器发现并保留两轮登录故障证据：login/context 均 200，session 已 active，URL 变为 `/`，DOM 仍为登录表单且没有 profile 请求。原因是 SessionStatusGate 的身份 key 重建了 BrowserRouter，而旧登录闭包向旧 history 导航。最小修复将 BrowserRouter 放在 gate 外，保留 gate 的身份 key、旧请求取消和跨组织缓存清理。398 项 Node 测试、63 项会话专项、两个真实源码 UI fixture 登录和一个旧响应迟到的浏览器回归均通过，类型／SDK 检查通过，lint 为零错误及 47 个既有 warning；见 [修复证据](uuid-u05-host-login-fix.json)。这些 fixture 与实际 U05 登录分别记录，没有使用脚本 goto 绕过故障。

新宿主 `host-u05-r2` 在全新候选目录构建、校验和 finalize 后冻结，原 dist、初始冻结副本及七微应用字节保留。新网关通过只读 child bind 加载 207 个新宿主文件，原网关停止并保留。合并的 [r2 清单](uuid-u05-frontend-builds-r2.json) 绑定初始 manifest；8 个应用的 253 文件再次全部 HTTP 回读匹配，见 [实际 overlay 验证](uuid-u05-host-overlay.json)。

实际浏览器第四轮已完成 7 应用 UI 导航、真实 201 应用 token／200 context、新部署版本和 v7 主体、实际 SDK 3 消费、入口 JS／CSS 冻结字节及无静态凭据、7 次卸载信号撤销，66 项检查通过、零 pageerror／transport failure。第三轮只因脚本将顶层管理 context 误当业务 context 而失败，保留记录后改为核对七应用各自权威 context 一致；没有更改业务授权或跳过真实 UI。

在原 policy 恢复后的第五轮稳定配置中再次 66 项通过，实际脚本 134 项、安全 props 观察 16 次、7 次卸载，零 pageerror／transport failure。第四轮一条未知 HTTP 403 console 消息保留原貌；第五轮完整响应诊断实测唯一 403 为 `GET /api/monitor/overview`，业务组织缺少平台监控权限，Dashboard 按既有合同切到 PersonalDashboard，未造成 SDK／挂载失败。不能写成“零 console 错误”，也不反向猜测第四轮记录。私有截图、完整 URL／响应／claims 不公开；见 [真实浏览器证据](uuid-u05-browser.json)。

## 实际凭据与协议验收

`credential-validation-02` exit 0，9 组检查、116 次真实 HTTP 通过，见 [凭据摘要](uuid-u05-credentials.json)。覆盖新登录与目标主体、12 个新机密密钥、9 个真实旧 raw secret 拒绝、另外 3 个旧 hash 变更；2 个旧 session、2 个旧 refresh、7 个旧应用 access 重放；平台 refresh 单次轮换；7 个应用 context／introspection／authorize 及跨应用拒绝；OAuth code、S256 PKCE、callback/state、单次兑换、scope、refresh、机器 opaque 主体与 device 生命周期；真实旧组织／应用 ID 与错误 variant 拒绝。

证据限制保留：9 个旧 JWT 已过期，另独立证明旧 key 签名有效、新 key 不接受；旧 code/device 源表为空，只能记录源空与明确合成负例。没有普通旧 OAuth raw 样本，采用源表排除与 runtime 前目标零行证据，不能把混合 oauth_tokens 的 105 行说成 105 次真实重放。第一轮因 HTTP header 大小写处理错误失败，原记录保留；修复后 Linux 11 项脚本自测通过，不降低拒绝断言，429／5xx 不计作凭据失效。

## 外部引用与授权边界

14 个 OAuth opaque client_id 均保原，客户端行 UUID 使用同一映射。当前六服务选用的是 7 个应用与 3 个专用本地 OAuth 流程；4 个种子客户端未被这些配置选用。审计发现一个仍登记外部调试站点 callback 的种子客户端，不能推断外部 owner 已确认迁移。已在本地固定目标用受控维护事务将它设为 DISABLED，其他字段逐项保原；现有 API 没有 status 更新合同，未发送会被忽略的 status 字段伪造禁用。有效参数的本地 authorize 实测 401 `invalid_client`、客户端已禁用，无 Location，外部请求零次。见 [引用审计](uuid-u05-reference-audit.json)、[围栏](uuid-u05-external-rp-fence.json) 和 [真实拒绝](uuid-u05-external-rp-fence-rejection.json)。外部账号绑定／owner 协同仍未测试；重新启用须另行协调，不能把本地围栏当作外部 RP 迁移成功。

实际 work-orders policy 只有 ticket 的五类 ORG 操作，stores 只有 store/staff/report 的 ORG_SUBTREE 操作，没有 notification/publish 能力。真实工单提醒返回 `403 DATA_DENIED`，HQ 本人 USER 通知发布返回 `403 FORBIDDEN`，符合现有合同。已完成的门店安全更新、真实工单创建／读取／owner／幂等、通知主体认证／旧凭据拒绝和 chat 双连接见 [业务原始范围结果](uuid-u05-business.json)；OAuth BFF 的真实 PKCE、consent、callback、userinfo／introspection、重放、refresh／revoke／logout 另有 10 项、17 HTTP 通过证据，见 [BFF 验收](uuid-u05-oauth-bff.json)。

用户已在本轮明确选择“临时增加仅本轮测试主体可用的最小通知授权，验收后撤回”。独立 policy 扩展只为 work-orders 增加 `notification/publish SELF` 能力，再经正式数据授权 GET／preview／PATCH 只赋给本轮测试的单个 membership；SELF 限制本人及当前组织的 USER 接收目标，组织／租户广播实测拒绝。原认证 policy、manifest、env 及冻结二进制保原。

`notification-validation-02` 的 7 组、31 HTTP 通过：复用原测试工单创建 1 publication／1 delivery，6 个关联 v7 字段、深链接、HTTP inbox 与实际 Rust Socket.IO event 一致，两次幂等重放无额外投递。随后正式 API 撤回唯一 grant，原 5 项 ticket grant 的 ID／scope／关联保持；两个仍可读取 context 的 live token 和撤回后新签 token 都被拒绝发布，排除仅因 token 到期的假通过。第一轮脚本 context 包装假设错误及 finally 成功撤回证据保留；见 [通知正向补充](uuid-u05-notification.json)。

在第四轮浏览器关闭后，先核对正式 grant 撤回和其余授权保留，再停止临时平台并恢复原容器／env。实际运行进程的 policy、JWT 和数据库角色与原 env 对应，不含临时 notification 能力；全表 notification/publish grant 剩余为零，四项健康检查 200。见 [原 policy 恢复证据](uuid-u05-notification-policy-restored.json)。通知补充报告中的 PENDING 是它交接当时的状态，以这份独立后续恢复记录闭合；没有覆盖旧记录或留下扩权。

## 最终数据库复核

真实浏览器第五轮关闭后，使用冻结脚本在三个固定目标和维护 catalog 执行四个有界 `REPEATABLE READ READ ONLY` 事务并 ROLLBACK，exit 0。没有写数据库，也不把每库独立只读快照说成协调跨库快照。原较早快照 [uuid-u05-database-audit.json](uuid-u05-database-audit.json) 保留；最后一轮为 [uuid-u05-database-audit-final.json](uuid-u05-database-audit-final.json)，SHA-256 `ec949bd0d783179245825ddad388a7bf99b01c83dbd76ffa29ab4982ad1c18d8`。

147 个 UUID 列、3,058 个非空值全部为 RFC v7，非法值零；67 个 FK 全部 validated，disabled trigger 零。三库完整原始 schema JSON 摘要与 U04 本代完全相同，包含内部 OID 派生触发器名称，没有规范化掩盖差异。正式迁移历史仍为 12／3／4，success、SHA-384 与精确 legacy allowlist 通过。

三目标默认只读、owner NOLOGIN，三个业务 runtime 角色本库可写、通知只读；跨库 CONNECT、DDL、owner／高权限 SET／USAGE／admin_option 旁路均拒绝。四张外部任务表实时为零；三源库默认只读、活动 backend 与 prepared transaction 为零，两旧服务角色继续 NOLOGIN。该源围栏复核不冒称重新执行 U03 完整导出准入或 U06 新鲜源等价证明，也不从任务表零行推断未接触的外部效果。

## 验收与后续边界

| U05 验收项 | 当前状态 | 证据／剩余工作 |
|---|---|---|
| 实际 SDK/vendor、新请求 v7、门禁和历史字节 | PASS | SDK 3 实际 Node／七应用浏览器消费、253 文件回读、216 历史文件保原；宿主 r2 修复后稳定配置复验通过 |
| 三库、配置、应用、通知、OAuth 引用与主体一致 | PASS | 类型化配置、七正式发布、门店／工单、BFF／通知 v7 深链接／事件及授权撤回／原 policy 恢复；外部 seed client 围栏与未验证范围明确 |
| 旧凭据失效、新凭据及协议有效 | PASS | 116 次真实 HTTP，覆盖限制如上 |
| 登记的历史 v4 保原、错误版本／variant 拒绝 | PASS | 登记扫描、真实输入负例、最终 147 列／3,058 值全 v7，schema／FK／角色和源围栏保持 |

台账建议已按原有全目录执行授权落实：UUID-U05 → COMPLETED，接续 UUID-U06 → NOT_STARTED；本轮临时通知最小授权和撤回记录加入 Decision Log，Q4／Q7 按本地实际范围闭合。

U06 的最终源等价证明、独立最终目标、停写／切流和新写后无损恢复没有在本阶段执行。U05 目标已有测试工单、通知、授权审计和新凭据，后续恢复必须对账新写，不能直接切回旧库丢弃它们。一个外部 callback 客户端继续禁用，外部 owner／账号关联未测试，重新启用须另行协调。身份 Phase 1 已由“执行身份 SSO 任务台账”聊天独立完成，身份指针 Phase 2；本专项不修改其台账或冒领其证据。
