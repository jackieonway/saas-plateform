# 身份感知双库恢复作业

当前自动演练严格绑定部署 UUIDv7、专用 PostgreSQL cluster、精确新库名及其 comment。它不会接管 preview/UUID 专项数据库。此文件描述操作和限制；实际通过与否以当轮安全 receipt 和阶段验收记录为准。

## 执行职责

恢复由唯一的受控离线作业执行。它需要能够恢复全部原 owner 和 grantor 的独立管理连接；现行数据库 migration owner 是 NOLOGIN，且不继承 identity/platform owner，不能把普通 migration owner 登录或任一 runtime 当成跨 owner 恢复管理员。不得临时授予 runtime owner、SET ROLE、CREATE、TRIGGER 或原始认证表权限。

执行前由部署维护人停止接流、暂停自动重启，排空并停止 identity、platform、portal、notification 和候选实例，关闭所有指向当前部署库的非管理连接。恢复期间入口保持关闭。脚本只使用普通 DROP DATABASE，并检查全部目标的 cluster、数据库名、comment、owner/ACL 和零连接。生产恢复还需要真实资产登记、明确维护人、持续完整的库外 deny/highwater 日志与独立恢复授权；本任务绑定及一次本地演练不替代这些输入。

## Checkpoint 与库外材料

`capture_checkpoint(context, catalog_module)` 在所有写入者已停止后，分别运行 PostgreSQL17 custom-format `pg_dump`，并记录安全数据 projection、两库 database owner/comment/ACL 与 schema/relation/column/function/default-privilege 指纹。停写窗口内的顺序双库 checkpoint 不被描述为 PostgreSQL 跨库原子 snapshot。

RS256 keyset、console/portal 两 AEAD keyring、platform JWT key、各独立内部 service/RP/resource 凭据、数据库 runtime 连接配置、CA/网关材料和明确进程配置进入同一个加密材料包。原始 dump、PHC、密码、私钥与 env 只存在于内存或权限为0700/0600的任务私有目录。

每个 artifact 使用 AES-256-CBC 与 PBKDF2-HMAC-SHA256（210000次），再用独立 HMAC-SHA256 key 认证绑定 metadata 和全部密文。元数据包含任务、cluster、两库、issuer、checkpoint、原文大小/摘要和用途。解密前先认证；manifest 另有独立认证 sidecar，并由外部 witness 绑定其 SHA256。篡改密文会在解密之前拒绝。

恢复库外材料时复制到恢复专用私有目录，不覆写 原始 keyset。先核验恢复字节与 checkpoint 所含材料一致，再轮换平台 JWT、两 BFF keyring、service/RP/resource client secret，以及全部四个数据库 runtime 密码和 notification metrics token。OIDC kid/public material仍受现有 immutable 约束，不把旧 kid 改为不同材料。

## 不可回退的外部 witness

`WitnessStore` 使用独立 HMAC key、任务/cluster/checkpoint/两库/固定issuer绑定、序号链与认证 CURRENT。文件模式0600、目录0700，逐级 anchored open + O_NOFOLLOW；文件 fsync 后发布，CURRENT 原子替换后 fsync 父目录。内容/tag篡改、有效旧 witness 回放、旧 CURRENT 回放、缺失链、额外孤儿文件、并发写入和 highwater 降级/遗漏均失败关闭。

payload保存完整的16类安全 inventory、账号 generation highwater、恢复epoch与业务 marker 摘要。成员删除通过完整 ID inventory 的缺失表达，不能仅快照当前“启用行”而漏掉删除。恢复只应用删除、禁用、集合交集、版本 highwater 和权限撤销，不重建备份后的新增对象。

任务将 witness 与其 key 保存在不会被本次数据库/备份 restore 覆盖的位置；外部目录不进入加密备份。整个外部目录如果也被回滚，仍需调用方独立保留 minimum_sequence。局部文件系统持久化不等于异地副本、不可变存储、完整生产控制日志或灾难恢复能力。无法确认日志完整时，入口继续关闭并由明确维护人对账。

## 恢复与失效顺序

1. 在任何 DROP 之前认证、解密全部两库 dump 与材料包，核对原文大小、摘要和 PGDMP 格式，并检查两个源目标的归属及零连接。
2. 在原名重建两个库，保留空默认 public schema，恢复原 database owner/comment/ACL，真实运行 `pg_restore --exit-on-error --single-transaction`。不使用 no-owner/no-acl，不重新 provision，也不修改历史迁移。
3. 在任何 fence/DML 之前严格比较两库全部原 owner/有效 grant 与安全数据指纹。列ACL的NULL/空数组只表示没有额外列授权；关系 owner 默认ACL按 PostgreSQL 内建语义展开，真实 grantor/grantee/privilege/grant-option 差异仍拒绝。
4. 所有恢复账号先改为 DISABLED，并将密码置为不可验证的恢复标记。该批量操作要求精确行数并复查，不接受静默 trigger 抑制。
5. 随后单独写入严格高于 dump 与库外 highwater 的账号 session_version。不能与禁用合并：现有 BEFORE status trigger 会把 ENABLED→DISABLED 的版本重写为 OLD+1。
6. 保持全部 trigger 启用。SSO、OIDC grant/access/refresh/code/interaction、platform session/refresh/ticket、legacy OAuth含机器token/code/device、两 BFF session/flow/browser、customer opaque、pending account/OIDC intent全部失效并检查没有残留 live 行。安全 audit/receipt保留。
7. 根据认证 witness 应用 deny-only reconciliation。成员精确按ID删除/停用，immutable身份不变；策略和授权取交集；祖先拓扑无法安全表达时禁用。未投影完整策略的 legacy client，只要其policy generation或client ID发生变化就保守禁用。grant删除先于membership删除，避免FK级联打乱精确行数核验。
8. 只有明确允许、dump与最新snapshot均启用且没有retirement tombstone的现有任务账号，才能由受控作业设全新密码。Debian Argon2 CLI只从stdin接收密码，PHC只留在内存；使用Argon2id v19、32MiB、t2、p1，不弱于当前默认策略。其他账号维持禁用。
9. 写入并认证外部 start receipt，绑定最新witness sequence、checkpoint、fenced inventory/floor与固定issuer。`assert_start_allowed`在恢复进程启动前再次核验外部链、receipt和数据库状态，拒绝回退。原deployment ready marker不替代该门禁。
10. 重启同一已验证binary和固定issuer拓扑；先验旧密码/旧凭据/旧命令拒绝，再完成全新SSO、员工/客户两RP与业务准入。任务临时600秒旧载体窗口必须在最终新登录前恢复45秒并再次重启，保证后续46秒浏览器刷新检查使用正常期限。

原密码、旧RP/service/machine secret、旧数据库runtime密码均有拒绝检查；旧OAuth/app token使用恢复前成功的同一正常端点验证失效，不能用本来就不接受该token种类的API制造负例。机器token用新client secret内省旧token应inactive，并验证新机器grant仍可用。

任务 PostgreSQL 的 loopback HBA 使用 trust，不能据其连接成功断言密码有效或轮换生效。四个 runtime 使用已绑定任务容器地址 `172.20.0.2`，由现有 catch-all SCRAM 规则验证密码；启动前逐一验证正确密码成功、错误密码失败，恢复后在同一地址验证旧密码失败和新密码成功。管理／迁移连接保留原路径，不修改共享 cluster 的 HBA。恢复 helper 只额外允许这一固定任务地址，不接受任意远端或其他私网地址。

OIDC admin creation receipt保留：轮换service key后，旧service secret应403，新secret查询原creation receipt应409，不能从旧receipt重建或替换secret。原pending account operation不能借新版本/新deadline执行，原pending OIDC intent不会重新创建client。旧ID Token仍可能被缓存公钥离线验签；验证的是它不能作为业务Bearer、原grant/session在线授权已经失效，不声称清理数据库改变签名数学有效性。

## 记录与验证

纯测试不连接数据库或启动服务：

RTO从故障/停止接流算到外部对账、全部旧拒绝、新配置重启与新双RP业务全部完成。RPO使用checkpoint和故障前实际业务marker及其真实丢失情况，主动废弃认证状态单列。隔离任务24小时/4小时候选目标的实测通过，不被扩写为生产15分钟/60分钟、PITR、持续WAL、异地回读、固定备份频率或30日99.9%可用性。若生产要处理备份后的签名key吊销、全量策略或其他新安全对象，必须把相应完整生命周期加入外部deny日志并证明覆盖，不能直接套用本任务受控变更清单。
