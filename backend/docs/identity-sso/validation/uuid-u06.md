# UUID-U06：最终切流与新写后恢复

阶段：`COMPLETED`（用户明确接受本轮RTO偏差）。最终只读审计已通过。真实 final 切换、T0 后新写、T1 增量恢复和 recovery 新 epoch 已执行，当前预览使用 recovery。保留数据RPO=0的逐行证明通过；恢复窗口909.035秒超原RTO≤600秒目标，用户已明确接受本轮偏差并要求完成U06。原超时测量、失败和计时保留，不视为后续SLO放宽。机器摘要见 [uuid-u06-summary.json](uuid-u06-summary.json)。

## 1. 实际范围

仅本地 WSL Ubuntu PostgreSQL 18.6，入口 `http://127.0.0.1:13000`。远程、生产、外部 RP 回调及身份 0013／Phase 2 排除。正式冻结 migrator 为平台12、门店3、工单4版。

| 用途 | 平台 | 门店 | 工单 |
|---|---|---|---|
| U05 新鲜源 | qiankun_platform_uuid7_u04_20261024 | qiankun_stores_uuid7_u04_20261024 | qiankun_orders_uuid7_u04_20261024 |
| final | qiankun_platform_uuid7_final_20261024 | qiankun_stores_uuid7_final_20261024 | qiankun_orders_uuid7_final_20261024 |
| recovery | qiankun_platform_uuid7_recovery_20261024 | qiankun_stores_uuid7_recovery_20261024 | qiankun_orders_uuid7_recovery_20261024 |

六个目标创建前确认不存在，由正式迁移独立初始化。九库 OID／owner、各代 raw schema、共享 semantic schema及十二个固定 runtime 名称统一登记，并与实时 pg_database 独立核对。登记 SHA 为 `c1b2c7936af2b29bf5b1015adc8fd7c3b7365ac350a64f84a746a7c0f087600f`。已有合法 v7 身份原值保持，原 v4 三库和旧包继续保留。

## 2. 数据与准备门禁

54 张表使用 PostgreSQL 原生 row_to_json text 封存，JSON 列另存原生 text，保留精确 numeric、SQL NULL／JSON null、时间及 JSON 词法。六类旧凭据表只进入加密源证据而不恢复，metadata由正式迁移重建；其余业务、账号、审计、发布、通知、计数器和幂等回执保留。外键和业务 trigger全程启用。

平台正式迁移的 settings singleton仅完整默认值和安装时间匹配后事务替换；空 store_staff不发空INSERT；既有层级DML使用受审pg_catalog,public搜索路径。完整 raw schema分代pin，跨代仅规范内部FK trigger系统OID名称。

准备验证：真实codec及Rust166项；真实54表 real-03为21项／166SQL，紧急续跑修补后real-04为29项／218SQL，九fixture库／九owner清理零失败、无业务库或源行使用；runtime17、原U05 6、HTTP10、业务修补后9、control7、switch12；epoch根因回归12及隔离真实PG17 SQL／ACL fixture通过。隔离PG17结果不等同生产PG18.6验证。关键路径均独立只读复审。

最终数据引擎SHA：`48dff22fbfac575d334443e0e45af78bfd25cf6b325d39d9f128463a8a3b9fc8`。工具与证据见[数据回归](uuid-u06-data.json)、[操作合同](../../../ops/recovery/scripts/uuid-u06-control.md)、[数据合同](../../../ops/recovery/scripts/uuid-u06-data.md)、[HTTP合同](../../../ops/recovery/scripts/uuid-u06-http.md)。

## 3. 第一次真实切换与故障

精确停止七个U05容器、四runtime角色NOLOGIN、排空连接与prepared。A快照666,972行，按合同保留666,765行，其余为排除凭据或重建metadata。final三库导入及完整行／序列／schema／迁移对账通过，再独立reread后设置有认证快照摘要的marker。

| 操作 | 实测秒数 |
|---|---:|
| U05冻结 | 10.254 |
| A快照认证封存 | 26.714 |
| final导入与完整对账 | 229.147 |
| 独立reread及交接 | 26.245 |
| final服务启动完成，窗口累计上界 | 561.960 |
| 完整123次HTTP检查完成，窗口累计上界 | 614.864 |

epoch准备器首次在无epoch目录或新角色落地前以LIVE_RAW_SCHEMA_MISMATCH停止。真实只读核对确认raw catalog受搜索路径影响，Runner原默认public与数据工具pg_catalog不同。固定Runner为pg_catalog／UTC后真实prepare／activate通过，补根因回归；原失败及原计时保留。

先封存final新epoch的T0，再开放已核对final，并在服务可用期间独立预热recovery。暖restore238.776秒、独立verify25.292秒，均在第二次停写前通过。T0认证包不变，期间所有新写由T1增量覆盖。

## 4. T0后真实新写

final实际两次门店同值version PATCH、创建工单、原幂等重放、工单描述及状态更新、完整读取。门店版本3→5。首次业务探针已提交PATCH，但全响应比较因organization.expiresAt失败；它是每次目录请求生成的三秒lease。修补探针逐响应验证UTC／三秒有效窗口，仅对该动态元信息单独处理，其他完整字段、owner、version、时间和组织revision／contextVersion仍严格比较。首次已提交更新／审计不删除，全部进入T1。

成功final业务9次API，新工单1张，可见inbox1行，只GET不发布。失败和成功私有回执分别保留。

## 5. 新写后恢复与受控续跑

精确冻结final并获取T1，保留666,815行，比T0多50行。认证delta53个变化：平台audit42 INSERT、notifications4 INSERT、users1 UPDATE；门店audit2 INSERT和stores1 UPDATE；工单counter1 UPDATE、创建幂等1 INSERT、work_orders1 INSERT。

首次replay平台全T1核对后已提交，门店因净版本3→5违反每次UPDATE恰加1的guard回滚，工单未执行；所有recovery库保持隔离，未直接回切旧库。补丁仅对精确受审stores／日报纯guard表、版本差2，在同一事务内做一次不可见version-only递增，再应用完整认证后像；其他差值拒绝。两条真实业务审计仍由原delta追加，不制造已提交历史，不关闭trigger。

显式resume_after_kinds=[platform]先用独立READ ONLY会话完整核对平台T1行／seq／schema，平台0 DML、无setval／commit，再恢复仍精确T0的门店／工单。普通重复重放仍拒绝。29项真实回归实际覆盖部分提交、错误续跑拒绝、平台xmin／ctid／seq不变和正确续跑。真实续跑及三个库T1全量再核对通过。

recovery新epoch后完整读回final门店／工单／时间／版本／owner／原始幂等响应／通知行与links，再新建并读回一张v7工单，7次业务API通过。原始窗口至核心读写完成累计上界909.035秒，超过600；计时包含故障调查／修补且没有重置，RTO失败。

## 6. epoch、协议及实际产物

final／recovery各自从空六凭据表建立新JWT key、轮换全部12个confidential secret、所有用户session_version各加1；四runtime先NOLOGIN，数据和材料核对后才启用。opaque client_id和既有v7主体保原，唯一外部callback seed仍DISABLED且其它字段保持，无外部回调请求或owner确认冒领。

recovery150 HTTP通过：11启用confidential正向、1停用正确新secret仍拒绝；source／final共24旧secret拒绝，14旧app请求401；七应用、新session／refresh、普通OAuth／PKCE／CC／device通过。旧JWT真实签名／新key拒绝、六表排除、session边界和运行角色围栏分别核对，不改写旧JWT。

60秒code及部分ticket／app／device在长窗口后已自然过期，原始分类计数保留，natural_expiry_is_epoch_proof=false；不把过期单独当作epoch证明。未过期session／platform refresh／普通OAuth access和refresh的真实重放另行记录。

recovery网关实际HTTP回读8个前端253文件，长度及SHA全匹配r2／SDK3.0.0，216历史SDK文件保原。本轮未重跑U05的66项浏览器检查，不借历史结果冒充本轮浏览器验收。

## 7. 保留与限制

原v4、U05 v7源、含新写final以及认证A／T0／T1／delta均保留，密文／密钥分开，root目录0700／文件0600，完整明文快照仅私有tmpfs。RPO=0针对保留合同数据，六类明确作废凭据不复活；冻结T1一直可读，本轮不是任意介质丢失／WAL／PITR保证。未配置deployment／ZIP agent和空任务表按最终只读事实记录，不宣称未接触系统的一般exactly-once。临时notification/publish授权未重新启用。身份主台账及独立身份新库不修改，用户接受本轮偏差后解除本专项UUID前置门禁；后续身份工作仍按其台账核对范围，原SLO不因此放宽。

## 8. 验收

| 条件 | 当前结果 |
|---|---|
| 精确目标、停写、正式迁移及完整业务对账 | PASS |
| 全系统v7、新epoch、旧凭据及消费者一致性 | PASS：九库最终只读审计、HTTP／业务／静态均通过 |
| RPO=0及恢复时限验收 | PASS（用户明确接受本轮RTO偏差）；RPO PASS，原RTO测量仍FAIL：909.035／600秒 |
| 历史证据保留及身份阶段边界 | PASS |

用户于2026-10-24明确回复：“接受本轮909秒偏差，按实际结果完成U06”。据此本轮U06完成，无剩余必做项；[closure决策](uuid-u06-closure.json)单独记录原600秒目标、909.035秒实测和单次例外范围，未重演或重置计时。

最终只读审计见[uuid-u06-final-audit.json](uuid-u06-final-audit.json)：三代各147UUID列／67FK，invalid=0、orphans=0；recovery非空UUID3,176个全为v7／RFC variant。原v4及source／final围栏、恢复runtime OID／ACL、历史外部client身份及唯一DISABLED、publish授权0、四任务表0均通过。audit-01的CREATEDB-only谓词误报与诊断保留，修补后audit-02通过，未为消除误报改任何角色或ACL。

## 9. 本轮关闭决定

用户接受仅限本次本地U06演练。四项验收已完成，其中RTO以明确接受的偏差关闭，原测量不改为达标。历史[最终审计](uuid-u06-final-audit.json)原字节与SHA保持，里面的completion_eligible=false／未接受状态是审计当时事实；当前完成决定以[closure](uuid-u06-closure.json)和本报告为准。原v4／U05／final／认证包保留，当前恢复服务配置不变；身份Phase2没有在这次关闭中执行。
