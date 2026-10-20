# Suggested Task Ledger Update — Phase20

状态建议：`COMPLETED`。三条台账验收与范围内必需门禁全部通过，最终采用同一r2候选的PG18/18、完整fast53/53且778/0/0、原ELF raw12/12；managed为PHASE20_GATES_PASS、27项检查全PASS，绑定179份原始产物。本文件是结构化台账更新建议，不直接覆盖台账；独立开发观察和平台strict失败原样保留，不由完成结论改写。

## Phase Status Update

Phase20“迁移 Rust 工单提醒到平台通知API”：`COMPLETED`，三条checkbox全部完成。同步阶段摘要表、详情、Expected Outputs、Notes、执行指针与Change Log，保留其他聊天已采纳状态和修改。

下一建议Phase21“交付 Vue + Vite 工单前端与未知结果恢复”／NOT_STARTED，仅建议，不实施。

## Output Summary

- 独立POST remind使用当前普通应用Bearer和同一intro/context，分别检查ticket/read及notification/publish的功能权限、直接data grant和组织硬上限，不修改工单／version、不直写平台通知表或自行广播。
- 新0003 intent按tenant／publisher-org／可信application UUID／user／key固定工单、有效收件目标、完整上游正文和hash；不可更新／删除，无TTL／级联删除／token／成功cache。0001／0002字节和Cargo依赖保持。
- scope内FOR SHARE短事务确认COMMIT并释放连接／锁后，每个入站请求最多一次正式平台POST；已持久intent同key换工单／target／正文409，即使原平台未提交。
- PREPARATION_UNKNOWN503明确本请求零POST，确无持久intent的后续请求可首次claim当前快照；RESULT_UNKNOWN503可能已发布，保留同key／固定原文，用当前有效凭据显式恢复，不自动换键或补发。
- 成功只投影经验证的publicationId、两个1计数、duplicate布尔和RFC3339时间。真实接收者session inbox核对，实时未运行不改判持久失败，删除delivery后重放不复活。
- 跨库不原子，意图提交后工单仍可变化；校验平台200后不再读工单改判失败。当前失权／软删仍阻止后续恢复。
- 原Phase19 strict基线不认识0003，回退须保留intent／receipt／platform publication并采用兼容构建；旧Node runner会迁移但handler不具新intent语义。Phase21前端、实时改造及生产切流不在本次范围。

## Completed Items

- Rust提醒route／用例、双授权、不可变intent、0003安全迁移、受控PG与真实平台联合及CI注册已完成，7份接入文档与采用／回退说明同步。
- 最终采用r2为711files／283 AST edges／42 explicit required／44 registered tests，capture `2026-10-17T05:10:56.421Z`，SHA `4f9a2cb80ece51c86e07a86bdafb98ca765fc2498a30ae5f64e7ce08cec3f449`。PG／fast／raw均绑定此输入，执行source前后稳定、hostDrift=[]；raw原ELF也保持不变。
- Scoped PG outer／child PASS／exit0、18/18步骤：platform76／migration27／read13／write14／member12／reminder12／四live父测试各1，共158父测试，另lib56／doc1及rustdoc／ELF build全部PASS，全部0FAIL／0skip；该层215次测试执行含lib56，不与fast重复计为不同测试。fullPostgresGateExecuted=false，独立notification-live明确未选，不借历史PASS或skip。
- 0003 migration27验证fresh、Node／SQLx v1-v3合法前缀、历史和receipt／intent保持、Node-Rust并发升级重启、增量DDL回滚／重试、不可变intent及application域、坏历史／结构拒绝，真实自有PG清理保留。
- Controlled reminder12及四proof全部通过：稳定原文／单publication／version不变、独立权限与两类grant／SELF／组织硬上限、未知结果同key恢复、意图持久后原平台未提交也不能换target、COMMIT拒绝零POST。平台POST等待响应时业务唯一pool可用／行锁可取；12case assertionsOk／databaseRemoved／poolClosed全部true。
- live20真实密码login／app-token／confidential内省、正式grant预览／写入与真实通知发布，父1／12named全PASS。接收者真实session inbox、同key回执、处理人／owner fallback／SELF、四类缺权、直接成员／目标scope、未提交未知后的目标冲突、已提交响应丢失恢复和实时未启动持久成功通过；删除delivery重放不复活，version不变。两库／pool、进程restart／final stop全部true。live17／18／19另在同候选重跑，各父1／named13／12／10全部PASS且清理true。
- 完整Linux fast outer／child PASS／exit0、53/53步骤PASS，778PASS／0FAIL／0skip；contracts355／CIpolicy100与工单fmt／check／all-target all-feature strict Clippy／lib56／rustdoc均通过。778已含lib56，不重复加总；既有Socket.IO回归不等于Phase23／34完整联合，全局realtime_protocol_result保持NOT_EVALUATED。
- 同PG原ELF67,913,136bytes／SHA `8fb1a06578f379dc92b04942ebdace81c7cd09d2d11d5664665d431640a890cf` 的raw12/12PASS。UID10001、无Node／hostports／workspace mounts，rootfs可写；健康与PGdown恢复、Node0001/2/3和SQLx1/2/3、receipt1／intent1、业务数据与counter保持、启动deadline拒绝／锁释放重试和SIGTERM exit0。平台故意不可达，raw只证明失败关闭；成功提醒由live20证明，不声明生产镜像或只读rootfs。
- PG／fast／raw三路各完成正确phase／CI owner的container／network四种精确label查询，exit0且空；cleanupErrors=[]、unexpectedLeaseExit=false、WSL lease停止exit0。执行driver、snapshot、preflight、原ELF和原日志均绑定归档，不跨候选拼绿。
- fresh正式boundary为BOUNDARIES_PASS、8/8检查、65owned／2 CI fixtures／42required，hostDrift=[]，旧0001／0002与平台／业务Cargo manifests和locks固定。boundary文件SHA `fe544f3756783323b66f185760221067253c38e4d1e376e67ae8e696728ef0ca`；正式inputs文件SHA `ae99baeb1de3183b60158cb9ca96d9e3c767d961ca18c17f9d338d43aecad719`，它们区别于输入清单内容SHA。
- managed PHASE20_GATES_PASS、27/27检查／179产物，audit SHA `e89e1386e674d0b3f2ec8ba188b5dfc669fd47ad0124c739f40ca6244ea6a026`、artifactsSha256 `6951ff90587cd5883c34233f4735cf09b23287b4fb601279b58276d74b19172b`。179是managed绑定产物数，不冒充正式runtime索引自身的产物数量；retainedRuns1、retainedCandidates2、originalOuterFailures0，native／Node观察6与平台strict两FAIL另行保留。
- r1 candidate `4f8b4558d8e0bafdb13981ff7e10f18f8dac0de671a0838dda84804d5d2fd91e`仅capture未dispatch，不存在r1 outer FAIL／取消；tuple alias修复后捕获r2。不能借Phase19失败模式虚构本轮取消。
- 独立开发readiness源 `b14139efb7e2f0878d5b81d6866abab80a0f39a305a682a048a61a106ca874f0` 的migration27／reminder12实测PASS、4proof true、12case清理true；outer PASS／0、执行源稳定，但hostDrift为两份CI README，四资源空查、cleanupErrors=[]和lease0均保留。该集合不计入最终r2数量。
- 独立Node CI readiness100／0／0，来源为alias修复前4f8b…d91e、sourceStable=true，不替代r2完整fast。Native fmt PASS；check r1 FAIL／101（Uuid JSON无Serialize，改to_string且不增依赖）、sourceStable=false记录live测试路径漂移，check r2 PASS／stable。Native lib56为54PASS／2已有HTTP FAIL／stable，不断言根因；工单strict Clippy PASS。
- 全平台strict r1 FAIL／54errors含新live type_complexity1，私有tuple alias修复后r2 FAIL／53errors、phase20 live及main归属0；两轮JSON／JSONL／stderr／wrapper均保留。最终Linux通过也不能改写这些独立观察，工单strict通过不代表全平台strict通过。

## Remaining Items

Phase20范围内剩余：**无**。三条验收、必要真实PG／平台联合、完整fast、原ELF运行与来源／边界／清理审计全部通过。已披露native HTTP失败、全平台strict债务、跨库竞态、未持久准备、0003回退及旧Node handler差异；这些限制不由完成结论改写。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 同事件重试只一条持久通知；工单version不因提醒改变。 | PASS | r2 controlled12／live20父1＋12named验证同key并发、响应未知／重启恢复、单publication／delivery及version不变。 |
| 无读权限/发布权限/数据grant、越界收件人均拒绝；未知结果不自动换键重发。 | PASS | 四条独立permission／grant拒绝、SELF／硬上限、preparation零POST、平台未提交但intent持久后的换target409、unknown proof同key／原文／单publication／无自动重试通过。 |
| 平台收件箱可核对结果，实时服务未就绪不被误判为持久发布失败。 | PASS | 接收者真实session当前组织inbox按publicationId可见，service_started=false仍持久成功，删除delivery后同publication replay不复活，工单version不变。 |

## New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 新增窄不可变intent，采用平台五元唯一域 | 单靠平台幂等无法阻止原平台未提交时旧key目标漂移 | 0003增量迁移；保留原0001／0002和receipt，无队列或成功cache。 |
| COMMIT确认且释放本地连接后才POST | 外部HTTP不能占用业务行锁，准备未知不能冒充已发送 | 每入站最多一次POST；本地preparation unknown与上游result unknown分别报告。 |
| 不承诺未持久准备锁住旧target | `{idempotencyKey}`不携带原快照，未提交数据无法在进程／数据库故障后恢复 | 后续确无intent时可首次准备当前快照；已有intent必须保持同target／正文。 |
| 当前read／publish检查先于恢复，不members预查 | 历史结果不能绕过撤权，已有平台回执不应被新的收件枚举阻断 | 新事件由平台检查recipient，历史重放保留原回执且不补建已删delivery。 |
| 平台200后不以工单再读改判失败 | 业务意图和平台publication不具跨库原子性 | 明确竞态窗口，持久发布与工单后续变化／实时提示分别解释。 |
| 0003须兼容构建回退 | 原Phase19严格基线拒绝新历史和对象，旧Node handler未采用intent | 不删intent／receipt或改checksum；在途先对账，不声称旧handler等价。 |
| 完整门禁采用单一r2，开发观察独立保留 | 不能将readiness、native或仅capture候选的结果拼入最终运行 | r2 PG18／fast53／raw12均同源稳定，managed27项通过；r1无dispatch、不虚构取消历史。 |

## New Open Questions

无新增阻塞性业务开放问题。跨库窗口、未持久准备和平台历史回执边界均已说明；前端恢复、完整实时联合、生产交付与切流仍由后续阶段处理，不因Phase20完成外推。全平台strict债务和native两项HTTP失败按原证据保留。

## Next Recommended Phase

Phase21：交付 Vue + Vite 工单前端与未知结果恢复，NOT_STARTED。本次只完成Phase20并同步终态，保留其他聊天状态；须新的明确执行请求，不自动实施下一阶段。

## Ledger Evidence Note

2026-10-17完成Phase20。独立Rust提醒以同次当前ticket/read与notification/publish功能权限／直接data grant及组织硬上限授权，0003在tenant／publisher-org／可信application／user／key域固定ticket／target／完整正文；短scope事务确认COMMIT并释放连接后最多一次普通应用Bearer正式POST。准备未知明确零POST，确无intent的后续首次准备可用当前快照；发布未知保留同key／原文、不自动换键。持久intent即使原平台未提交也拒绝换target／ticket，提醒不改version；真实收件者session inbox可核对，实时未启动不影响持久成功，已删delivery重放不复活，平台200后不再读工单改判失败。r2 711files／283AST／42required／SHA4f9a2cb80ece51c86e07a86bdafb98ca765fc2498a30ae5f64e7ce08cec3f449，PG outer／child18/18：platform76／migration27／read13／write14／member12／reminder12／四live父各1(named13/12/10/12)、lib56／doc1／rustdoc／build全部PASS；fast outer／child53/53、778/0/0，raw原ELF67913136bytes／8fb1a06578f379dc92b04942ebdace81c7cd09d2d11d5664665d431640a890cf的12/12PASS，三版本双历史／receipt1／intent1与数据counter保留。三路source／ELF稳定、hostDrift[]、cleanupErrors[]、各四资源空查／lease0。fresh boundary8PASS／65owned／2CI fixtures；managed27PASS／179产物、audit e89e1386e674d0b3f2ec8ba188b5dfc669fd47ad0124c739f40ca6244ea6a026。保留develop-readiness27+12／b141…74f0及两README drift、NodeCI100／4f8b源、native check失败→修复、lib54/2与平台strict54→53／归属0原证据；r1仅capture无dispatch、originalOuterFailures0，不拼绿。全PG未选notification-live，realtime_protocol_result仍NOT_EVALUATED，不称完整Socket.IO／生产HTTPS／镜像／部署验收。0003保持0001/2与依赖，原Phase19不能直接回退，保留intent／receipt／publication并用兼容构建；旧Node runner迁移不代表handler等价。三条验收完成、Phase20 COMPLETED；仅建议21 NOT_STARTED，不实施前端、实时或切流。
