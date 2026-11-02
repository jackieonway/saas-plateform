# Phase 25 验收证据层级独立审查

本报告只审查验收应观察什么、什么证据层级足以支持该行为，覆盖准备矩阵全部 **111/111** 子例；不修改主矩阵，不把任何未执行项目置为 PASS。当前组件新执行的实际 test name、命令、版本、断言及清理由组件执行器另行绑定。原阶段报告只用于依赖审查。

## 依据与判定原则

直接依据为[架构第18节](../../unified-oauth-oidc-sso-architecture.md#18-必须通过的验收矩阵)、[Phase25当前验收标准](../../unified-oauth-oidc-sso-task-ledger.md#phase-25-完成发布候选全新安装与验收矩阵)和[111项准备矩阵](phase-25-matrix.json)。架构原文：**“验证须区分单元测试、HTTP fixtures、真实数据库、真实跨服务 HTTPS、浏览器和外部标准客户端。任何一种不能单独替代全链路验收。”** 这要求完整候选具有全部必要层级，不要求每个后端负例都通过浏览器重新执行。

Phase25要求“方案验收矩阵所有条目有可追溯结果”“从空库完成全部参与服务和外部示例；失败与恢复路径验证”。因此，当前PG/组件证据可以完成对应的安全不变量子断言；真实标准RP、自有BFF、业务API、通知/SDK及恢复的当前全链仍需独立执行并按实际观察关联。不能把所有111项都留给单一大浏览器脚本，也不能把组件总数当作111项已覆盖。

| 标记 | 本表含义 | 不能声称 |
| --- | --- | --- |
| P | 当前生产代码的最小充分后端/数据证据：UNIT、实际router、SMALL_SCHEMA_PG、CANONICAL_PG、真实LOGIN ACL按实际断言分别标注；涉及持久状态/锁/审计时必须真实PG | 小schema、mock port或同进程状态不能更名为canonical、真实跨服务或OS多进程 |
| S | 独立标准SDK实际验证受控有效签名与错误字段，使用明确HTTPS issuer fixture，保留CA/hostname校验 | 不能称为实际identity互操作，helper未调用不算SDK拒绝 |
| U | 当前真实页面/宿主DOM的浏览器交互、渲染、焦点、尺寸与请求计数；明确允许隔离API fixture | 不能称为实际身份/业务跨服务成功 |
| X | 当前冻结版本的真实服务之间HTTPS/标准客户端/业务或Socket通信；明确实际生产handler与数据库 | 不要求每个X子断言重复整套浏览器，但mock producer不能代替 |
| B | 真实浏览器中的当前实际服务主流程、Cookie/多标签页或桌面移动流程 | 两个helper对象/受控API fixture不能证明实际Cookie与跨域行为 |
| R | 当前独占环境中的进程/故障/负载/密钥缓存/实际恢复/告警或日志证据；按行为使用HTTPS，不必都操作浏览器 | 组件成功不能证明应用真实配置、运维恢复或告警送达 |
| A | 本轮资源的实际归属、删除与残留盘点 | 无操作/删除失败不能算清理通过 |

带“+”表示应组合的不同事实，不表示每一组件都要升级为后一层。标记不自动产生通过结论；每项必须有直接断言，不能仅依据候选sourceRef或大测试标题。P证据明确保留small schema/mock port边界，完整空库/ACL安装及总体X/B证据仍单独必需。

## 准备表需要显式校正的过度约束

架构原文对code/PKCE写“code 重放、并发双兑换拒绝”，refresh写“轮换、并发、重用 family 撤销”。准备表C05-S06将其加成“两个真实身份进程”，C06-S02注记又要求实际双进程。当前[phase22 canonical测试](../../../../../../saas-identity/src/oidc/oauth/phase22_tests.rs#L261)使用独立AppState、tokio任务、真实PG事务/行锁屏障；显式等待两请求被同一DB锁阻塞，并断言code只有1个grant/1次消费/1条audit，refresh只有1个成功且reuse撤销准确family并保留其他来源。它证明架构要求的跨实例DB并发序列化；它没有启动两个OS进程，错误消息中的“independent processes”不改变证据层级。

建议主执行器记录一次透明的矩阵边界校正，把上述两个子例恢复为“独立实例共享真实PG并发”并明确同进程限制，同时保留当前全版本真实服务主链。若保留准备表的原字面，则本报告标记R*的两项仍须实际多进程；禁止直接以P填成“两真实进程通过”。本报告未修改该两项。

C03-S08/S09/S10要求实际标准SDK验证错误issuer/nonce/aud，这是S层即可充分测试的客户端验证器行为。错误audience的backchannel测试不是ID Token aud负例；签名issuer与回调iss是两个不同检查。C03-S05的auth_time必须和实际人员认证事实比较，种植新鲜MFA仅证明权限predicate，不能替代主认证。

## 已抽查的当前canonical断言

- [phase15真实runtime能力测试](../../../../src/identity_deployment_tests/phase15.rs#L50)实际provision身份/平台LOGIN角色，逐个变更成员、组织、应用、admission、client/grant/SSO来源，在事务内检查能力拒绝；撤销后重新插入组织准入仍拒绝，且验证不可逆revoked约束和真实行锁后过期。因此C12多项及C10派生能力的后端边界可用当前PG重跑结果支持，无需浏览器重现每条SQL mutation。
- [canonical审核测试](../../../../src/auth_review/tests.rs#L360)直接核对自审拒绝、expectedVersion冲突、原operation重放、receipt/audit计数；默认/缺失release binding关闭、权限撤销后新写与重放失败也有真实PG断言。它们支持C14对应不变量，但调用service方法不自动证明HTTP CSRF/重复Cookie，且人工fixture MFA不证明真实主认证。
- [centralized customer选择恢复](../../../../src/customer_portal_tests/selection.rs#L169)实际检查consume后result字节相同、仅一个customer context、加密存储、重复consume冲突、旧来源/续期不复活及member撤销后恢复拒绝；它使用单平台DB和mock Sources。故C13-S02幂等断言可受P支持，但C13-S01“门户本地提交失败后的跨独立DB恢复”不能单凭该测试置为完整通过。
- [独立标准RP测试](../../../../../../saas-examples/oidc-rp/tests/oidc.test.mjs#L33)实际调用SDK/HTTPS callback并断言state/issuer/nonce/PKCE/signature/UserInfo失败无会话；这是真实标准SDK的S层测试，既不需要浏览器负例逐一重放，也不能充当真实identity主链X。

## 111项逐条层级判定

### C01 统一登记

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C01-S01` 自有与第三方同表 | P | 同表/无隐藏registry由当前schema、路由与PG登记读取证明。 |
| `P25-C01-S02` 一应用多客户端 | P | 两个client的登记、code/client/callback隔离可由真实PG router请求证明；无需重复浏览器登录。 |
| `P25-C01-S03` TEST与PRODUCTION隔离 | P | 环境与admission/scope拒绝属于当前canonical PG策略边界。 |
| `P25-C01-S04` 单一公开权威 | X | 唯一公开authority包含实际网关路由；内部router 404不能单独证明公开入口。 |

### C02 普通外部登录

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C02-S01` 无业务成员账号 | B | 真实零membership账号的ACCOUNT_ONLY成功且不显示业务选择；PG和网络调用计数共同支撑。 |
| `P25-C02-S02` 仅标准OIDC接入 | X | 必须使用真实当前identity与独立标准RP，不以受控issuer fixture顶替互操作。 |

### C03 OIDC一致性

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C03-S01` 正确issuer | X | 准确HTTPS issuer/discovery/回调一致须有当前部署链；签名iss字段可由P/S细验。 |
| `P25-C03-S02` 正确audience | P+S | 签发aud与client对应及RP验证边界可分别由PG签发与标准SDK受控签名fixture证明。 |
| `P25-C03-S03` 不透明subject | P | opaque/pairwise映射由真实PG与签发断言证明，不需要每个sector都浏览器登录。 |
| `P25-C03-S04` 正确nonce | P+S | nonce绑定与拒绝可由真实请求transaction和标准SDK signed fixture证明。 |
| `P25-C03-S05` 正确auth_time | P | 须直接比较真实primary/MFA认证事实与人员auth_time；人工种MFA仅能证明predicate负例。 |
| `P25-C03-S06` UserInfo subject一致 | X | 标准SDK与真实identity的ID Token/UserInfo一致作为当前互操作主链；S覆盖负例。 |
| `P25-C03-S07` 错误client拒绝 | P+S | code/client/UserInfo隔离由PG router，ID Token client验证由SDK fixture；不能混为一条helper断言。 |
| `P25-C03-S08` 错误issuer拒绝 | S | 标准SDK实际消费错误回调iss和错误签名issuer；受控HTTPS issuer足够，无需再做真实浏览器。 |
| `P25-C03-S09` 错误nonce拒绝 | S | 有效签名错nonce经实际SDK callback拒绝且无会话；不能只测试比较helper。 |
| `P25-C03-S10` 错误ID Token audience拒绝 | S | 有效签名错aud须实际进入标准SDK验证；现有backchannel错aud不等于ID Token错aud。 |

### C04 subject隔离

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C04-S01` 同sector稳定 | P | 同sector跨client稳定为持久映射不变量。 |
| `P25-C04-S02` 不同合作方隔离 | P | 不同partner/app/environment不可关联为持久映射不变量。 |
| `P25-C04-S03` 更名稳定 | P | 更名后映射稳定可直接在任务PG更名并再签发/读取。 |
| `P25-C04-S04` 注销不复用 | P | 注销封存/新账号不复用须PG持久映射与新签发比对；恢复组件不独自证明注销行为。 |

### C05 code与PKCE

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C05-S01` 错误callback拒绝 | P | authorize和token真实router的精确callback负例，无需浏览器。 |
| `P25-C05-S02` plain拒绝 | P | 必须请求实际authorize parser，断言plain拒绝且无code；S256变换单测不足。 |
| `P25-C05-S03` 非法challenge拒绝 | P | 实际authorize语法负例，覆盖长度/字符/缺失；无需额外TLS连接。 |
| `P25-C05-S04` 错误verifier拒绝 | P | 真实PG token端点错误verifier拒绝并检查code状态。 |
| `P25-C05-S05` code重放拒绝 | P | 真实PG code重复兑换、grant和audit计数足够证明重放边界。 |
| `P25-C05-S06` 并发兑换单赢家 | R* | 准备表额外要求两个OS进程；按字面须R。架构只要求并发，当前独立AppState+PG屏障足够证明架构不变量，需显式校正表述后才能按P填写。 |

### C06 refresh

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C06-S01` 轮换 | P | 真实PG轮换与原sub/sid绑定；BFF保留来源由当前组件补充。 |
| `P25-C06-S02` 并发 | R* | 准备表写两个节点并要求实际双进程；按现有注记保留R。架构并发要求可由独立状态+真实PG锁屏障证明，不能称为OS双进程。 |
| `P25-C06-S03` reuse撤销family | P | family重用撤销、后代和无关来源计数/调用由真实PG证明。 |
| `P25-C06-S04` 绝对期限 | P | DB clock/实际行锁后期限及BFF截止断言足够；不需浏览器等待最长TTL。 |
| `P25-C06-S05` 空闲期限 | P | 真实PG受控时间边界足够；配置名称不是期限生效证据。 |
| `P25-C06-S06` scope收缩 | P | refresh实际签发scope交集与权威收缩由PG router证明。 |
| `P25-C06-S07` 禁止scope扩权 | P | 实际token负例拒绝扩大resource/scope；无需TLS重复每种scope。 |
| `P25-C06-S08` 在线来源与离线分离 | P | 自然到期与GLOBAL/consent显式撤销分别在PG原来源/离线family验证。 |

### C07 客户端认证

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C07-S01` 公开client无secret | P+U | PUBLIC字段/存储/运行用PG与router；页面不展示/发送secret用当前真实DOM fixture。 |
| `P25-C07-S02` 机密client错误凭据 | P | USER/MACHINE错误或缺失Basic各自PG router负例。 |
| `P25-C07-S03` 凭据轮换 | P | 新旧secret/version/重叠边界可PG router验证；OAuth与私有service凭据不能合并替代。 |
| `P25-C07-S04` 紧急撤销 | P+R | PG持久代际拒绝与当前应用重新启动后旧凭据拒绝；若仅重建AppState须准确标明层级。 |

### C08 资源隔离

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C08-S01` ID Token拒绝API | P | 实际业务handler/内省router的ID Token Bearer负例；另需全版本资源主链X，不能把userinfo拒绝当业务API拒绝。 |
| `P25-C08-S02` 错误audience拒绝 | P | 两个实际业务handler的错误audience负例；当前跨服务X主链另行覆盖实际proof布线。 |
| `P25-C08-S03` 跨资源scope拒绝 | P | PG资源/scope tuple及实际handler拒绝可证明绑定；不得只跑字符串helper。 |
| `P25-C08-S04` 错误主体拒绝 | P | 各endpoint主体与member限制需当前handler/PG负例；不要求每个负例浏览器执行。 |
| `P25-C08-S05` 机器不冒充人员 | P | machine签发声明及USER能力/客户会话负例可PG router证明；至少一个真实MACHINE互操作另列X。 |

### C09 同意

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C09-S01` 首次确认 | P+U | 首次同意展示应用/用途/resource/scope要真实DOM；未批准不能签发要PG；UI可用明确HTTP fixture。 |
| `P25-C09-S02` 后续复用 | P+U | 同意复用规则由PG，正常页面无需重复确认由DOM交互证明；无需为每组重复全链。 |
| `P25-C09-S03` 新增scope再确认 | P+U | 扩大scope/group版本的PG拒绝与重新提示页面断言；不能只断言consent行version。 |
| `P25-C09-S04` 跨组不继承 | P | 跨组/app/environment不继承为PG同意边界。 |
| `P25-C09-S05` 同组受控共享 | P | 显式同组资格与新增成员version为PG约束/服务边界。 |

### C10 撤销与删除

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C10-S01` active不可直接删 | P | DELETE失败且grant/receipt不变须PG事务断言，无需browser。 |
| `P25-C10-S02` 撤销来源均失效 | P+X | PG逐来源撤销计数加当前BFF/业务能力实际拒绝主链；单一身份小schema不能证明所有消费者。 |
| `P25-C10-S03` 删除不复活 | P | 隐藏/删除后旧token和墓碑代际由PG持续拒绝证明；恢复另属C18。 |
| `P25-C10-S04` 旧删除与重授权冲突 | P | 旧expectedVersion DELETE与重新授权并发冲突须真实PG锁/行状态断言。 |

### C11 上下文

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C11-S01` 零候选 | P | 零候选拒绝及无默认member由实际router/PG证明；页面错误可U补充。 |
| `P25-C11-S02` 单租户 | P+U | 单租户多组织的UI步骤及后端精确候选检查；UI fixture不冒充当前跨服务来源。 |
| `P25-C11-S03` 多租户 | P+U | 先租户后组织且不默认首条须真实DOM交互，PG负责资格。 |
| `P25-C11-S04` 单组织 | P+U | 单可用成员自动继续用DOM请求计数与PG服务端验证组合。 |
| `P25-C11-S05` 多组织 | P+U | 多成员显式选择与完整成员tuple用DOM和PG；无需每个tuple完整TLS重放。 |
| `P25-C11-S06` 记忆同意单项自动继续 | P+U | 无需额外确认按钮是UI事实；同意/候选资格是PG事实。 |
| `P25-C11-S07` 搜索不自动选 | U | 搜索过滤至一项仍不approve必须真实DOM+网络请求计数；静态源码字符串检查不足。 |
| `P25-C11-S08` 员工客户隔离 | B | 两个实际BFF、Cookie域和员工客户用途隔离要当前浏览器与真实服务；单mock不能覆盖。 |
| `P25-C11-S09` ACCOUNT_ONLY无context调用 | P+X | SelectionPort零调用可P证明逻辑，当前ACCOUNT_ONLY登录/refresh主链网络计数证明真实无调用。 |

### C12 业务权限

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C12-S01` 组织收缩 | P | canonical runtime的组织收缩、恢复后旧能力不复活可PG证明；总体需当前业务X主链。 |
| `P25-C12-S02` 成员收缩 | P | 准确member版本/停用/退休引起失效须PG；总体需当前业务handler验证。 |
| `P25-C12-S03` 应用准入收缩 | P | app/tenant/org admission原事务收缩由PG锁/触发器/runtime验证，无需UI。 |
| `P25-C12-S04` scope不越过用户权限 | P | scope和用户实际权限交集由真实handler+PG授权拒绝证明；X用于核验当前外部token布线。 |
| `P25-C12-S05` 恢复授权不恢复旧token | P | 重启用后旧代际仍失效和新能力成功由PG持久事实证明。 |

### C13 跨库与标签页

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C13-S01` 原结果恢复 | P+X | 平台结果持久恢复用PG；门户提交失败后跨独立DB恢复相同capability/截止须当前跨服务故障路径。 |
| `P25-C13-S02` 重试不重复消费 | P | 同operation只一次消费/审计为PG持久幂等不变量。 |
| `P25-C13-S03` 切换冻结 | U+X | 同步冻结是当前宿主状态/真实DOM事实，微应用与通知连通由实际主链覆盖。 |
| `P25-C13-S04` 旧请求不能覆盖 | P+U | 迟到成功/错误的代际保护由可控promise/transport组件断言；无需真的拖慢网络浏览器。 |
| `P25-C13-S05` 多标签页同步 | B | 同一浏览器两个标签页及共享Cookie竞态必须浏览器实际双页，不以两对象helper顶替。 |
| `P25-C13-S06` 微应用同步 | U+X | SDK宿主生命周期组件可U验证；当前真实宿主交付与旧token拒绝须X交叉引用。 |
| `P25-C13-S07` 通知同步 | P+X | HTTP/socket旧来源取消与recipient隔离由当前服务/Socket.IO主链及DB来源证明；纯mock仅覆盖取消算法。 |

### C14 管理安全

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C14-S01` 查看权限 | P | VIEW元数据最小化与写/审/轮换拒绝由后端权限/PG证明，无需浏览器隐藏按钮替代。 |
| `P25-C14-S02` 修改权限 | P | REGISTER/SCOPES范围和禁止自审由PG服务/路由请求证明。 |
| `P25-C14-S03` 审核发布权限 | P | REVIEW/PUBLISH分离、default/missing binding关闭由canonical PG证明；fixture不构成真实发布授权。 |
| `P25-C14-S04` 轮换权限 | P | ROTATE/KEY_ADMIN独立权限、intent和审计事务由PG证明。 |
| `P25-C14-S05` 越权拒绝 | P | 权限或应用委派收缩后新写/重放拒绝由canonical PG证明。 |
| `P25-C14-S06` CSRF拒绝 | P+S | 真实router的Origin/CSRF/重复Cookie参数负例足够；仅RP验证不能替代平台/身份管理路由。 |
| `P25-C14-S07` 审计失败原子回滚 | P | audit失败/静默抑制的事务回滚需要真实PG故障与mutation/receipt/撤销计数；浏览器不增强原子性证明。 |
| `P25-C14-S08` 幂等 | P | 原operation相同输入恢复、不同输入冲突、一次secret/审批为PG/router持久幂等边界。 |
| `P25-C14-S09` 版本冲突 | P | 乐观版本冲突及新配置/审批/删除事实不变为PG并发边界。 |

### C15 登出

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C15-S01` 当前应用范围 | P+X | 准确grant/其他RP和consent范围由PG；当前应用BFF实际拒绝由X登出主链。 |
| `P25-C15-S02` 全局范围 | P+X | GLOBAL全部原来源/离线family且保留consent由PG；实际RP退出用X闭合。 |
| `P25-C15-S03` 授权撤销范围 | P+X | 同意组派生来源与其他组隔离由PG；当前业务消费者拒绝用X闭合。 |
| `P25-C15-S04` back-channel签名 | P+S | 签名字段负例用真实标准SDK/TLS fixture，sid/sub原来源持久关闭用PG；不需每一负例真实浏览器。 |
| `P25-C15-S05` back-channel重放 | P+S | jti重放与未知sid墓碑/callback/refresh竞态须PG与SDK实际接收路由；普通内存map不足。 |
| `P25-C15-S06` back-channel重试 | P+X | lease/retry/stale completion由PG；真实worker到当前接收方的不可用/恢复至少一例X。 |
| `P25-C15-S07` 降级时效 | P+X | 受控时间验证实际本地/在线截止；当前RP无backchannel的降级策略须真实通信确认，不需睡满300秒。 |

### C16 浏览器

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C16-S01` 桌面 | B | 明确要求当前四种主流程桌面Chromium完成，必须真实服务浏览器证据。 |
| `P25-C16-S02` 移动 | B | 窄屏实际完成登录/同意/上下文/撤销；布局本身可U，但流程完成不能仅靠fixture冒充。 |
| `P25-C16-S03` 键盘 | U | 键盘操作由当前真实DOM浏览器fixture充分验证；总体B主链另列，不必每键重跑真实身份。 |
| `P25-C16-S04` 焦点 | U | focus trap/restore及错误后恢复由当前真实DOM fixture充分验证，静态源码检查不足。 |
| `P25-C16-S05` 弹窗滚动 | U | 滚动/可达/viewport由当前真实DOM和尺寸实际量测，API可以fixture。 |
| `P25-C16-S06` 开放重定向拒绝 | P+U | 服务端精确redirect拒绝用router，浏览器实际不跳恶意目的用DOM；无需给恶意站发送请求。 |
| `P25-C16-S07` XSS拒绝 | U | 恶意文本不成为可执行DOM和CSP需浏览器真实渲染/策略；API名称fixture可用。 |
| `P25-C16-S08` Cookie受限 | B | Cookie禁用/拦截与Secure/SameSite/跨RP域效果由当前真实浏览器/HTTPS环境证明。 |

### C17 故障

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C17-S01` 身份不可用 | R | 实际identity中断及当前BFF/资源恢复原来源；mock错误只证明分支。 |
| `P25-C17-S02` 平台不可用 | R | 当前私有proof超时/拒绝与恢复的真实链；PG权威锁负例提供补充。 |
| `P25-C17-S03` 数据库不可用 | R | 真实PG不可用/恢复的readiness/liveness/503与合法来源保护；组件关闭pool不等于整个DB进程故障。 |
| `P25-C17-S04` 连接池满 | P+R | 真实PG pool耗尽可由canonical PG测试直接证明；当前服务配置/恢复连接行为另需运行态检查。 |
| `P25-C17-S05` 队列满 | P+R | 有界队列算法与worker lease可组件/PG，当前配置下实际饱和/释放至少一个运行态路径。 |
| `P25-C17-S06` 超时 | P+R | deadline与迟到提交组件边界加实际socket/slow body/upstream路径，不能仅改配置标签。 |
| `P25-C17-S07` 时钟偏移 | P+S | 受控时钟/签名token的允许与越界skew由PG/标准SDK fixture，禁止改整机时钟。 |
| `P25-C17-S08` 错误语义与恢复 | P+R | 路由精确错误语义与不确定结果保护用PG/router，真实故障HTTP正文脱敏用运行态证据。 |

### C18 运维

| 子例 | 最小行为证据/必要组合 | 边界与直接观察 |
| --- | --- | --- |
| `P25-C18-S01` 密钥生命周期 | R | 当前signer/JWKS/独立SDK缓存的发布激活退役与紧急撤销真实运行；PG生命周期只覆盖控制面。 |
| `P25-C18-S02` 备份恢复deny事实 | R | 必须实际backup/restore-new/reconcile到新目标、旧载体拒绝；119个mock PG恢复组件不能替代。 |
| `P25-C18-S03` 容量 | R | 实际成功CODE/REFRESH/INTROSPECTION负载与资源/延迟量测，不是单元测试吞吐。 |
| `P25-C18-S04` 限流与告警 | P+R | 预算不可用socket重置可canonical PG证明；实际终态告警送达与确认需要当前告警管道。 |
| `P25-C18-S05` 日志脱敏 | R | 需要扫描本轮实际服务/网关/报告；组件脱敏单测不能证明未运行日志。 |
| `P25-C18-S06` 完整归属清理 | A | 本轮资源前后实际盘点、定向删除及残留检查；当前自动审批拒绝记录保持阻塞。 |

## 回填要求与当前限制

组件执行器可将当前版本的命令、测试全名、精确断言、输入摘要、实际证据层和清理结果映射到各requiredObservation；一个测试支持多个子例时应列出每个独立断言，不能从总测试PASS推定所有子例PASS。混合事实建议在actualEvidence中分层存证；未跑的X/B/R/A事实继续保持PENDING/BLOCKED，不抹去已充分证明的P/S/U事实。

本审查没有执行测试、启动/停止服务、连接数据库、创建容器或删除临时资源。frontend完整清理已在[收窄重试记录](phase-25-frontend-cleanup-retry.json)中如实记录自动审批启动前拒绝，C18-S06仍有明确清理阻塞。Phase26真实人员/运营及专项部署授权继续依[部署计划](phase-25-deployment-plan.md)保留。

## 编码核验

审查读取主JSON时显式使用UTF-8。原始158491字节经严格UTF-8解码成功，无BOM，字面U+FFFD数量为0，SHA-256为`4eb1512cf92285fac95cc8e1fd9baf07e026dd72138e394812a8a960f7d3a7f2`。不存在中文损坏，不重写主矩阵编码；默认系统编码/终端显示误码不能作为修改依据。此摘要绑定审查开始时的准备矩阵，后续主执行器合法回填会改变文件摘要。
