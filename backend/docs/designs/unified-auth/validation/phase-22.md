# Phase 22：认证监控与有界故障处理验收

日期：2026-10-28。技术验收：**PASS**。本报告覆盖当前源码、组件验证、专属原生 HTTPS 实测和本阶段资源清理；不代表生产 SLA、30 分钟容量目标、真实值班接收或生产发布已经确认。

## 验收结论

| 台账验收项 | 结论与证据 |
|---|---|
| 认证／平台／数据库失联、队列满、超时及连接池饱和有确定语义 | PASS。实际 HTTPS 证明数据库故障 503、存活 200／就绪 503、原 Cookie 恢复；真实平台暂停触发约三秒私有调用 503，并恢复同一上下文；Identity 停机由公共网关返回 502，重启后原 Cookie 继续使用；八个密码请求均受控返回 429。连接池关闭、占满和真实客户端表锁超时另由 canonical PostgreSQL 用例证明。 |
| 多实例正确性由数据库保证 | PASS。两个独立 `IdentityState::new`、独立预算和连接池同时等待真实数据库锁；code 仅兑换一次、refresh 竞争检测复用并只撤销原 family、相同 logout operation 仅有一次审计／receipt／outbox。是同一测试进程内的两个独立状态，不冒称多进程部署。 |
| 明确限额的专属压力与容量／瓶颈报告 | PASS。一个原生 Identity、一个原生平台、共享测试 PostgreSQL 上的独占新库，固定连接／worker／队列／请求期限。只计合法成功，报告 busy 丢槽、实际 RPS、p95/p99、CPU、RSS 和池占用；实测未达到 Q9 的目标速率，不承诺 SLA。 |
| 无秘密日志及异常／永久失败追踪、告警 | PASS。metrics 独立 Bearer、固定维度和私网入口；实际本地接收六条 DB／queue／TERMINAL 状态通知。TERMINAL 保留数据库事实，仅内存确认，不假报恢复。最终六份日志与私有材料在内存比较，泄漏 0；容量轮另比较 2436 个候选秘密，泄漏 0。 |

## 当前实现

公开认证请求先按真实 TCP peer 选择来源；只有明确 `TRUSTED_PROXY_IPS` 中的 peer 可转交唯一合法 `X-Real-IP`。来源、客户端和总请求槽位都有硬预算。请求体在进入授权 handler 前最多 32768 字节并有单独期限；没有给已可能提交的授权事务新增整体 HTTP 取消期限。TCP 接入及空闲连接、数据库池 acquire／statement／lock 等待也有界。额度属于单进程防护，不能替代数据库授权裁决。

`/healthz`、`/livez` 不查询数据库；原 readiness 路径保留数据库、角色及 issuer 校验。依赖不可用保持 503，不伪装成错误密码、清 Cookie 或采用旧授权。私有 `GET /internal/identity/metrics` 使用独立运维凭据，公开网关即使带正确 token 仍返回 404；私网缺少凭据返回 401，正确凭据返回 200、no-store。

指标只使用固定路由模板、method、status、阶段和结果。密码 queue、实际 execution、调用方 result 分开计时；结果超时不会提前释放仍在执行工作的 permit。数据库观测使用一次池连接和只读事务，500ms 总期限、400ms statement／250ms lock；失败不会输出健康空队列。mandatory audit 失败另有计数，outbox 的 pending 年龄、过期租约、TERMINAL／DEGRADED 及无 job 的 source 降级均可观测。

未设置／空 `RUST_LOG` 默认 INFO。启动要求 `saas_identity::delivery`、`saas_identity::delivery::worker` 和 `saas_identity::password` 均允许 WARN。独立审查发现父 delivery target 可被屏蔽的 P2，已修复，并通过五种 subscriber 组合及两个真实二进制拒绝启动检查。

配置默认值、精确范围、指标与运营限制见[监控合同](../phase-22-monitoring-contract.md)、[容量驱动合同](../phase-22-capacity-driver-contract.md)及[固定条件告警合同](../phase-22-fixed-alert-pipeline-contract.md)。Prometheus 配置和 13 条候选规则只是可部署模板；本次未运行 PromQL 引擎或 Alertmanager。

## 验证与证据边界

| 门禁 | 实际结果 |
|---|---|
| Linux 最终全库 `cargo test --lib --offline --locked -j2` | **87 PASS，54 explicit-PG ignored，0 failed**；3.86 秒。包含两项真实 socket 验证：连接预算／idle，以及慢 HTTP body 408、不进入 authority、释放准入。 |
| 二进制 subscriber 测试 | **1 PASS**，内含五种过滤配置；独立于 library 数量。 |
| 最终 strict Clippy | `--all-targets --offline --locked -j2 -- -D warnings` PASS。 |
| 独立状态 canonical PostgreSQL | **6/6 PASS，0 ignored**；[报告](phase-22-independent-state.md)及[JSON](phase-22-independent-state.json)。 |
| 观测模块 | 七项纯／路由测试已包含于 87 项；两项显式 PostgreSQL 测试单独通过，证明聚合只读、一次连接和受压有界。不能再把九项全数叠加到 87。 |
| Node 负载／告警组件 | **15 + 11 = 26/26 PASS**，含早醒、取消、busy 丢槽、collector 丢确认去重、认证和关闭；[证据](phase-22-harness-selftests.md)。 |
| 最终实际 HTTPS 故障与接收 | **8/8 PASS**；[runtime JSON](phase-22-runtime.json)。其中容量项引用已经完成的独立组件证据，不再次计为新压力测试。 |

所有 Rust 构建／测试均为 WSL Linux 的 `jackie` 用户，Rust 1.97、离线锁定依赖、并行 2、同一个任务 target cache。没有使用 Windows 当前不可用的 native loopback 结果代替实际 socket／TLS 证据。

实际正常登录使用生产默认 10000ms 的密码执行和总验证期限，实测成功耗时 805ms。在本就需要的 Identity 故障重启时，私有测试配置切换为 execution 100ms、queue 500ms、running 1／queue 1，使用合法 m=65536、t=3、p=1 高成本测试 PHC。八项请求均为 429，真实 WARN 可见。队列告警实际 FIRING 后 RESOLVED；permit 的取消／超时持有性质由组件用例证明。生产默认值和密码强度没有改变。

最终接收器实际接受六条固定载荷：`DATABASE_UNAVAILABLE` 的 FIRING／RESOLVED、`PASSWORD_QUEUE_PRESSURE` 的 FIRING／RESOLVED、`NEW_TERMINAL_DELIVERY` 的 FIRING／ACKNOWLEDGED。TERMINAL 来自明确标注的 owner 合成 SECURITY outbox，由 canonical trigger 产生 job 并在同一事务首次置终态；本次不声称 worker 执行过该投递。worker 的真实永久失败与 WARN 另见 [Phase 20](phase-20.md)。确认仅推进内存 baseline，数据库 TERMINAL 行保留，没有发送 TERMINAL RESOLVED。这是受控回环固定条件链路，不证明持久运营接收、升级流程、PromQL 或 Alertmanager。

## 实际容量

[独立容量组件 JSON](phase-22-capacity.json)来自完成全部时长的真实 TLS 轮次。USER code／refresh 的预置输入由 owner 写入 canonical 事实，不能算浏览器授权发行性能；授权码保留 60 秒期限并在服务健康后一次性生成。内省 token 通过真实 `client_credentials` 签发，使用独立 MACHINE／SERVICE／NONE 客户端和 `phase22.read` 资源，不包含人员 scope。

| 工作负载 | 请求预算／worker／时长 | 有效成功 | 实际成功 RPS | 成功 p95／p99 ms | skippedBusy |
|---|---|---:|---:|---:|---:|
| CODE | 25 RPS／8／30 秒 | 566 | 18.65 | 444.733／469.803 | 184 |
| REFRESH | 25 RPS／8／120 秒 | 2889 | 24.05 | 359.811／384.559 | 111 |
| INTROSPECTION | 200 RPS／16／120 秒 | 10525 | 87.62 | 376.692／408.105 | 13449 |

三类 invalid、业务拒绝、429、503、网络失败和未知结果退役均为 0；worker 忙时直接跳过，没有积累无限 pending。各类时长不同，不能相加成一个合计吞吐。

120 个私有指标采样全部成功；Identity 平均约 145.12% 单核 CPU，RSS 18,214,912 → 峰值 24,961,024 字节；数据库池上限 4、实际最大 4、idle 最低 0，所有负载的 worker 达到各自上限。该配置下出现明确的池占用与 worker 受限丢槽；没有隔离数据库、网关与协议成本，不能据此宣布生产最大瓶颈。构建为 dev，共享宿主的原有服务继续运行；三个负载共用一个 Node 进程，其 CPU／RSS 不能相加。

Q9 的 code/refresh 合计 50 RPS、内省 200 RPS、30 分钟持续压力未被本次证明。后续正式发布仍需真实运营责任、目标和部署条件确认。

## 固定输入、失败保留与清理

Identity 冻结源码快照 SHA256：`03878de6bb331ada97977d67bb82ed0c93f75ea01a20c68a5556df8abd4de1ad`；实际二进制：`7e3bda906d785b35ce8c65261ba3242b5cf0222b4a5276d9dfa13616d65bfe48`。见[构建见证](phase-22-build-attestation.json)。平台复用此前已验证的原生二进制；本次成功经过其产品 startup verify 后才运行故障链。容量复用项再次比较当前 Identity 二进制摘要，并在报告中标记独立 fixture／时间和没有发起新负载。

早期夹具路径、启动探针、Cookie 驱动和短测试预算问题均保留为失败证据；没有放宽产品验证，也没有重开已消费的 code／refresh。首轮驱动最终 sleep 提前约 1ms 导致完整时长断言失败，保留[原始失败](phase-22-runtime-before-deadline-fix.json)，修复为单调时钟循环／ceil 余量并新增三项纯时钟回归后才形成当前容量组件。[容量完成后的 Cookie 驱动失败](phase-22-runtime-before-cookie-fix.json)整体仍是 FAILED；其中已经完成的容量项通过明确的组件报告引用。后续只跑新的故障 fixture，避免反复压力。

[最终清理 JSON](phase-22-cleanup.json)为 PASS：删除专属 17,323,711 字节测试库和六个角色，24 个私有文件／日志 49,920 字节；tracked process、lease、cleanupErrors 均为 0。独立 catalog 复核集群 `7701548153213487841`，本阶段库／角色／活动连接均为 0；WSL 23441、23001、23443、127.0.0.2:443／24444 均关闭，Windows 24445 无监听，原 Windows 443 PID 5044 保持。另删除一个本阶段 Python cache 文件。

容量轮专属库 24,306,191 字节和六角色也已删除，见[该轮清理](phase-22-capacity-final-cleanup.json)。所有失败和成功轮次最多复用一套并发原生环境；没有创建或复用 Docker 容器，Docker context `default`，清理前后镜像／容器／卷／build cache 均为 0。

最终轮 `/tmp` 总 used 1,068,404,736 → 1,068,453,888 字节，承载共享 cache 的文件系统 used 33,460,285,440 → 33,460,289,536 字节。数据库／私有材料删除不等于共享 PostgreSQL WAL、后续仍需的 cache 或 Windows WSL 虚拟磁盘已缩小。共享测试 PostgreSQL、OWNER、公开工具、冻结快照和构建 cache 按总任务的后续阶段保留；本阶段没有遗留临时业务库、角色、服务、collector、租约或私有运行材料。

最终交付另有[运行与来源见证](phase-22-runtime-attestation.json)及[机器自检](phase-22-self-check.json)，包含精确来源摘要、独立 catalog／端口复核和相对链接检查。

执行者建议：由总任务维护者将 Phase 22 四项技术验收记为完成并推进 Phase 23；Q9 未证实的生产目标继续作为发布门禁保留。
