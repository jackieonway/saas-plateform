# Phase 32：真实指标、告警与容量基线

状态：**COMPLETED**。三项原验收均为PASS；候选性能未达标不转换为生产容量或商业SLA。仅执行Phase32。

## 输入与执行边界

- HEAD：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`；906个源码输入，SHA-256：`583f7f47d7275e734d38fd5f99d7f6fc832288fc2f3395e91ab15b0738f8c650`。当前工作树/冻结快照/两受管正式门禁输入一致；源代码未在门禁期间改变。
- 基线621ea69d5326f9b043cde3a01bf6ccbd2dd454179abb6b71efc78a76847f339e；本阶段76个新增/修改输入；9个历史迁移未变。平台/门店/工单锁文件323/252/253条registry包定义逐字比较保持；139份历史正式证据保持。已有未提交工作完整保留。
- 工具：Rust1.97.0、Node24.18.0、npm11.16.0、实际Nginx1.28.0、PostgreSQL17；精确镜像digest、ELF SHA、raw argv、运行日志SHA、owner及清理证据见runtime.json。API为locked debug ELF诊断样本；宿主及微应用为实际生产Vite dist。
- 机器：model name	: AMD Ryzen 9 8945HX with Radeon Graphics；MemTotal:       15967960 kB; MemFree:         7911224 kB; MemAvailable:   12762308 kB；cgroup CPU `max 100000`，memory `max`。未设置生产硬件预算。

## 交付与指标含义

平台/工单/门店通过纯std operational-telemetry crate共享固定service/stage/outcome词表；HTTP/SQL适配器与业务模型、数据库、发布和锁文件保持各自独立。固定直方图覆盖双应用context、authorization、platform_http、business_sql、pool_acquire、密码排队/真实执行、entry/READY/mount；在途、DB池和数据库Lock等待分别观测。business_sql包含池/事务/解码，不能当成服务端纯SQL执行时间。业务metrics默认关闭，使用独立且有界的运维凭据；生产run装配路径已通过真实进程验证。

浏览器发送认证、限速、最大60000ms的TIMING提示，词表固定；与失败事件分开配额，耗尽30个TIMING后仍可接收失败诊断。浏览器Performance measures只保留各固定stage最近一项。客户端耗时不用于安全判断。entry/ready是从挂载开始累计，mount是READY后操作确认耗时；它们与LCP/Event Timing分别测量。

持久收件箱当前行数使用gauge `platform_notification_committed_rows`，实际插入1条owned探针后读到1且observation_up=1。通知`emit_accepted_total`保留旧delivered_total兼容，表示传输接受，不能当作用户收件或已读。通知分区/队列/验证并发/LISTEN独立观测；恢复由实际收件ID、列表、未读/已读和双页检查验证。

独立Rust monitor按连续失败/恢复样本确认状态；初始健康静默，UNKNOWN不当作UP。真实门店schema故障连续两轮输出各一次DOWN/RECOVERED；两个业务库真实持表锁时，私有指标观察到Lock等待，释放后原API恢复200。backup_age/run/verify均只接收有序健康事件、拒绝旧sequence，采集凭据不能提交事件；没有实现备份。外部接收渠道未配置，仅证明本地JSON事件。

## 正式门禁

| 门禁 | 步骤/数量 | 结果 | 原始报告 |
| --- | --- | --- | --- |
| services | 40 | PASS | target/phase-32/formal-services-r3/result.json |
| browser | 20 | PASS | target/phase-32/formal-browser-r3/result.json |
| host | 26项宿主测试 | PASS | target/phase-32/formal-host-r3/result.json |

Rust实际执行199项，选取及计数见runtime；业务四组HTTP/SQL协议51项+门店SQL1项，密码池8项、平台配置7项、安全8项包含在内。CI/监控报告契约64项、生命周期契约56项；通知真实平台/PG34项，双页1父测试/6具名检查；真实浏览器双应用冷/暖四次导航。门禁没有必需跳过。全仓历史strict fmt/Clippy债务没有扩散修复；本阶段独立crate及两业务all-target严格检查、平台窄lib严格检查和all-target编译/文档均有本轮证据，不把它们写成全仓fast已通过。

## 同口径API负载

5万工单+5万日报，另有4个父门店和665,923条实际平台地区参考记录。四组跨组织/租户数据，工单状态/优先级/时间分布、5%软删除、不同日期/金额及有界正文；日报从1990年起，每组12500天。未添加索引或修改历史migration。

一个主体/一个底层平台会话，200个不同应用token由驱动持有；“逻辑在线”不表示200个浏览器或WebSocket连接。活跃会话100/150/200，每会话顺序读5次，stores/work-orders交替各半；实际客户端在途峰值分别为100/150/200。工单combined、门店legacy；每次读取包括最新平台授权、业务SQL及完整响应体。GET工单列表、GET指定门店日报列表。服务/驱动在私有工具容器loopback，PG在独立owned Docker网络；无WAN模拟、无独立预热、无持续soak。后续档位更暖，快速503不能解释为性能改善。

| 活跃会话 | 应用 | 请求数 | 全响应p95 ms | p99 ms | 200 / 503 / 429 | 非预期5xx/传输错误 | 候选 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 100 | stores | 250 | 663.70 | 819.33 | 84 / 166 / 0 | 66.40% | 未达标 |
| 100 | work-orders | 250 | 1332.28 | 1503.74 | 137 / 113 / 0 | 45.20% | 未达标 |
| 150 | stores | 375 | 169.65 | 225.73 | 142 / 233 / 0 | 62.13% | 未达标 |
| 150 | work-orders | 375 | 339.53 | 411.78 | 122 / 253 / 0 | 67.47% | 未达标 |
| 200 | stores | 500 | 161.66 | 205.91 | 118 / 382 / 0 | 76.40% | 未达标 |
| 200 | work-orders | 500 | 343.74 | 441.01 | 159 / 341 / 0 | 68.20% | 未达标 |

成功响应单独的p95/p99、状态全集和吞吐保存在performance.json。所有档位的候选失败保留，不能据此声称当前系统支持100并发，更不能从debug、单主体、短突发推导生产release持续容量。固定32槽授权准入、平台认证背压/429及CPU在该样本中可见；没有通过长正向缓存或放宽撤权语义“优化”数据。

| 活跃会话 | 独立服务 | CPU秒 | 单核等效平均CPU | RSS/HWM（原始kB） |
| --- | --- | --- | --- | --- |
| 100 | platform | 4.18 | 175.93% | VmHWM:    60096 kB / VmRSS:    60096 kB |
| 100 | stores | 1.83 | 77.02% | VmHWM:    28872 kB / VmRSS:    28872 kB |
| 100 | work-orders | 1.87 | 78.70% | VmHWM:    26176 kB / VmRSS:    26176 kB |
| 150 | platform | 4.78 | 373.77% | VmHWM:    63668 kB / VmRSS:    63668 kB |
| 150 | stores | 1.59 | 124.33% | VmHWM:    31568 kB / VmRSS:    31568 kB |
| 150 | work-orders | 1.21 | 94.61% | VmHWM:    29396 kB / VmRSS:    29396 kB |
| 200 | platform | 4.61 | 300.92% | VmHWM:    64156 kB / VmRSS:    64156 kB |
| 200 | stores | 1.44 | 94.00% | VmHWM:    34556 kB / VmRSS:    34556 kB |
| 200 | work-orders | 1.34 | 87.47% | VmHWM:    30696 kB / VmRSS:    30696 kB |

CPU由实际/proc user+system ticks差值、getconf CLK_TCK=100和本档wall时间计算。generator另列；PG与工具容器Docker CPU/内存/网络采样按时间和步骤保存。HWM是进程累计峰值，Docker内存包含缓存与PG tmpfs，不能混称纯进程RSS。

## 真实浏览器基线与撤权

156.0.8078.4 / Playwright 1.64.1；1540×1120，CPU倍率1，顶层CDP延迟20ms/下载10Mbps/上传5Mbps，真实Nginx HTTPS及owned受信CA。独立iframe目标继承没有另行强制，Rust→平台私网链路不整形；每次单页面测量，不宣称100浏览器并发下的CWV。浏览器也实际生成5万工单+5万日报（同样配方与组织/子树/其他租户分区）。冷上下文与同上下文再导航各一次；原始entry/ready/mount及成功上报的私有metrics已归档。

| 应用 | 缓存 | 宿主LCP ms | iframe LCP ms | 短交互延迟上界 ms | Event Timing样本 |
| --- | --- | --- | --- | --- | --- |
| stores | cold-context | 1272.00 | 676.00 | 24.00 | 3 |
| stores | warm-context | 264.00 | 184.00 | 16.00 | 3 |
| work-orders | cold-context | 1288.00 | 476.00 | 32.00 | 6 |
| work-orders | warm-context | 256.00 | 204.00 | 32.00 | 6 |

Event Timing有16ms阈值及8ms粒度；短序列使用最大交互/被省略事件16ms上界，不能标成现场长期INP。候选200ms在本短序列中的结果按每次记录保留。

撤权提交后直接发起的新请求403。浏览器被动感知：22秒观测窗内未检测到宿主状态或微应用读取权限拒绝信号，20秒候选未达标。分别检查宿主状态与微应用当前读取grant的拒绝提示，不能仅以“运行中”容器状态推断数据授权。

同一个有效运维凭据在平台/双业务私网均200，在公开Nginx均404；另对7条公开metrics路径实际请求并确认无Prometheus内容。通知私有metrics凭据与真实只读角色由通知联合回归另验。live、业务DB ready、授权HTTP和真实收件语义分开。

## 失败、兼容与限制

开发/旧正式失败全部保留在runtime.history：离线初始monitor索引不可用；首轮冷库地区COPY超过5s；生产run未接入metrics凭据；严格Clippy发现测试锁/handler位置；浏览器夹具缺pg驱动；旧工单受控夹具只实现legacy却未显式声明模式。修正后原协议断言保留，真实平台仍覆盖combined。formal-services-r2全部步骤通过，但收尾补齐native白名单时host源码变动，被正式输入保护拒绝采纳；最终只采纳本版三份一致报告。

通知/浏览器夹具冷库初始化显式使用60000ms SQL预算，实际导入完整参考目录；API容量先在测量外初始化目录，再按默认5000ms运行。生产默认没有改变。该硬件首次冷库初始化默认5s可能失败，须独立审查bootstrap预算/步骤。

原生与CI白名单、平台/业务Docker COPY和业务源码build context已带入纯指标库；离线Cargo真实验证快照路径依赖可解析。当前没有执行默认Docker源码builder、发布监控镜像、外部registry推送、外部告警发送或生产部署，Compose解析不当作这些行为已完成。既有SDK/业务协议和迁移保持；关掉业务metrics token或停用monitor profile即可停止新增私有采集，旧平台拒绝TIMING不影响业务授权。备份恢复和生产政策属于后续明确授权。

## Self-Check

| 原验收 | 结果 | 证据 |
| --- | --- | --- |
| 双应用context/DB锁/通知分区/entry→ready可观测，标签无token/user/完整URL | PASS | 固定指标枚举、真实三库/锁/浏览器私有采集、标签和秘密检查 |
| 真实故障可重复DOWN/RECOVERED、补拉完整、公开Nginx隔离 | PASS | 两轮schema故障、34通知联合+双页6检查、有效凭据200/404对照 |
| 同口径负载/数据/硬件/网络、p95/p99/错误/资源齐全，未达标如实记录 | PASS | 100/150/200、10万业务记录、API/CWV/资源原始数据及明确候选失败 |

## Suggested Task Ledger Update

Phase32: COMPLETED。输出为私有Rust监控、固定指标/告警模板、真实基准脚本和本报告及inputs/runtime/performance.json。Phase32无未完成验收；Q09正式SLO/资源预算及外部接收渠道继续待定。下一推荐Phase33：恢复加密备份与隔离新库恢复工具，NOT_STARTED，须新的明确请求。完整建议见phase-32-ledger-update.md。
