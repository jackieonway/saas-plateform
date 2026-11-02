# Phase 25 运行资源预查

日期：2026-10-28。状态：**静态预查完成；没有运行本页测试、构建、Docker 命令或服务。** Phase 24 尚未实施，21／22／23 仍会改变源码；正式 Phase 25 冻结候选后必须重新检查实际入口。本报告只回答“这些入口是否会间接创建容器、怎样维持零容器验证”，不是新版本功能或全新安装的通过证据。

计划由协调方复用已有 task-owned 原生 PostgreSQL `127.0.0.1:55494`，不复用业务数据库或默认 `5432`。预期 Docker 容器创建／复用均为 **0**。实际 cluster identity、端口、已有进程、目录归属和资源占用要由运行 owner 在执行前重新核对；本轮没有连接数据库或查询 Docker。正式报告和源码不能由清理脚本删除。

## 检查范围与静态结论

读取八仓主入口和被引用脚本，包括 15 个非 vendor Cargo manifest、14 个当前非 release/vendor Node package manifest、Cargo 锁文件、Node lifecycle hooks、Rust `Command`／Node child_process 调用、`.github/workflows`、Compose、Dockerfile 和本地启动脚本。搜索包含隐藏目录，排除 `.git`、node_modules、target、dist、版本 release/vendor 及生成的历史 JS；对实际会进入门禁的源码再逐项读实现。本轮没有发现 Testcontainers、bollard、docker-api 或 dockertest 依赖，也没有发现 `sqlx::test` 自动外部容器机制。

14 个当前 Node manifest 没有 pre/post install/test/build/check/lint/start 或 prepare lifecycle hook。若需要重新安装依赖，仍按已有 CI 使用 `npm ci --ignore-scripts --no-audit --no-fund`：锁文件的 `hasInstallScript` 包括 esbuild 和平台可选 fsevents，不能把“本仓无hook”理解为 npm 安装默认不会运行第三方脚本。必要的本地二进制应使用已审阅、锁定的安装方式，不临时开放所有 lifecycle hooks。

唯一仓内 Cargo `build.rs` 位于平台 backend，只输出 migrations／ops/identity 的 rerun-if-changed；不执行 Docker。八仓不是根 Cargo workspace，不能通过在工作区根调用 `cargo test --workspace` 假装覆盖全部项目。以下“可执行”均指资源机制已静态查明，并不保证所有历史测试会适配 Phase24 最终结构。

## 必须隔离的真实容器入口

| 入口 | 当前实现证据 | 零容器 Phase25 的处置 |
| --- | --- | --- |
| 平台 `backend/tools/deployment-agent` 可执行程序或 library `run/execute` | `src/engine.rs:25` 启动 `docker --host unix:///var/run/docker.sock`；`deploy` pull、inspect、create、start、exec；`routing.rs::reload` 会 exec 网关 nginx | **不运行部署代理 daemon／run，也不提交会被真实代理领取的部署任务。** 可编译、Clippy和运行当前普通单元测试；这些测试只校验policy拒绝及临时路由文件，没有调用成功DEPLOY／reload。将来新增成功部署测试须重新审核，不能仅按crate名放行 |
| 平台宿主 `frontend/deploy/prepare-history.mjs` CLI 或不带依赖注入的 `exportHistory` | 默认 `run=spawnSync`；`docker pull`、`image inspect`、`create --network none --entrypoint /bin/false`、cp、rm。**即使从未start，create仍计容器** | 不执行真实历史镜像导出；`npm test`中的 `deploy/tests/static-image-policy.mjs` 可执行，正向／失败导出均注入 `fakeDocker`，只模拟argv与临时文件 |
| 门店 `artifacts/start-local-api.ps1` | 查询既有 `codex-stores-api-local` 后可能 `docker start -a`，否则 `docker run -d`；读取用户私有运行配置 | **不运行**，不读取或接管该私有配置；Phase25直接启动task-owned Rust二进制与新库 |
| 门店 `frontend/deploy/local/start-gateway.ps1` | `$dockerArgs` 以run开头，固定 `codex-stores-gateway-local`、host网络；脚本仅建议stop而没有完整资源清理 | **不运行**；复用本次专属原生TLS gateway，不接管已有门店gateway |
| 任意仓 Compose up/run/create、Docker build、镜像运行或本地CI `act` | 平台compose含应用及notification/monitoring overlays；门店／工单backend、旧playground各有compose；多个CI的images job调用docker build | **不运行这些编排／image job。** 只做文本／渲染策略断言。若后续决定需要实际镜像验收，须先改变资源计划，按AGENTS核对context、预算、标识和完整清理，不能暗中扩大零容器方案 |
| CI的service containers | `saas-stores/.github/workflows/ci.yml` backend-postgres-contracts声明PostgreSQL18；平台CI backend-integration声明PostgreSQL17并映射默认5432 | 不运行整个CI编排。挑选实际测试命令，显式改用55494及相应所有权guard；既有5432不操作。CI文本不是本机验证授权或当前schema验收结果 |

没有把 `backend/tools/artifact-agent` 误列为Docker入口：其实现明确不启动子进程，负责受限节点文件安装；platform-admin artifact命令处理本地声明／文件及HTTP，也不是Docker CLI。它们若执行实际发布仍属于后续明确的运行操作，本页只允许其普通验证，不自动启用运维写动作。

## 可纳入零容器方案的门禁

所有命令都应从表中真实项目目录执行，沿用锁文件和已准备工具链。Rust普通门禁前清除与本轮无关的数据库／部署环境变量，防止测试偶然使用现有服务；PG专项目录／变量逐项提供，不以广泛继承的 `.env` 作为任务所有权证明。Linux Cargo继续使用协调方约定的非root用户与共享target，避免重复缓存和权限冲突。

| 项目／目录 | 普通静态／测试门禁 | 本地资源及限制 |
| --- | --- | --- |
| saas-identity | `cargo fmt --all -- --check`；`cargo check --locked --all-targets`；strict Clippy；普通 `cargo test --locked --all-targets`；doc；`node --test tests/invitation-ui.test.mjs`等确切UI单元入口 | 当前普通Rust无需Docker；真实PG测试大多明确ignored；delivery transport测试可能启动本测试可执行文件／原生TLS peer。不要把ignored数量当通过数量 |
| saas-plateform/backend | 同类fmt/check/clippy/普通all-targets/doc；Node `backend/tests/nginx-render-policy.mjs`；明确所需的crates/tools/ops各自manifest普通测试 | backend主manifest不自动覆盖全部工具crate。普通命令不会启动deployment-agent。Nginx测试在显式 `PLATFORM_TEST_NGINX/OPENSSL` 下会启动原生Nginx、生成临时cert；无Docker，但要记录／回收进程和临时目录 |
| saas-plateform/frontend | `npm run sdk:check`、`npm run qiankun:check`、lint、typecheck、`npm test`、`npm run build` | build为tsc+Vite+交付检查；默认test包含fakeDocker静态镜像策略；没有调用真实prepare-history导出。输出dist属候选构建工件，归属核对后由总owner决定保留或清理 |
| saas-plateform/frontend/tests/browser及显式浏览器suite | 固定Playwright浏览器的 `node tests/browser/run.mjs --browser chromium`，以及未被此runner包含的统一clients／intake／Phase18等明确入口 | 启动Vite、HTTP／Socket.IO fixture和Chromium进程，无Docker；runner当前仅含9个suite，不自动包含所有Phase19/18新增suite。不能以该runner通过宣称真实authority/TLS矩阵通过 |
| saas-customer-portal/backend、frontend | backend独立fmt/check/clippy/普通all-targets/doc；frontend lint/typecheck/test/build | portal测试的真实PG入口需显式owned变量；frontend普通测试无Docker。portal服务必须有独立新库，不能用shared库代替 |
| saas-stores/backend、frontend | backend独立fmt/check/clippy/lib及普通all-targets；frontend typecheck、test、build、verify:release | `frontend/scripts/test.mjs`只启动Node测试；build启动本地tsc/Vite。真实PG使用下表显式loopback opt-in。不要调用local gateway／API容器脚本 |
| saas-work-orders/backend、frontend | backend独立fmt/check/clippy/lib及普通all-targets；frontend test、build、release:check | frontend测试／build操作本地工件；后端可选 `WORK_ORDERS_TEST_NODE` 仅启动受控HTTP peer。reminder PG契约自建task库／pool，result-first和未知结果测试无真实Docker |
| saas-notification | fmt/check/clippy/普通all-targets；显式 `identity_isolation` PG/Socket.IO专项 | PG专项启动当前 `CARGO_BIN_EXE_saas-notification-ws`及受控authority HTTP peer；不是容器。应使用本次二进制和新库，禁止接管既有 `.local-run` notification进程 |
| saas-micro-app-sdk | `npm run check`、`npm test` | check校验20个产物而不写发布内容；test为Node。`npm run build`会写生成物，不是验证产物一致性所必需；不创建容器 |
| saas-examples/oidc-rp（Phase21进行中） | `npm run check`、`npm test` | 当前test支持代码启动原生HTTPS peer，并经sh/WSL调用openssl生成短期CA，trap清除临时私钥；无数据库／Docker。真正对canonical issuer互操作必须单列运行，不能用fixture测试替代 |
| saas-examples其余当前保留示例 | micro-vue／micro-jquery／report-center／websocket各自test/build；example scripts输入／provenance／delivery检查；各Rust API独立manifest测试 | 普通脚本用Node/Vite/VM/受控HTTP，无Docker。websocket `test:browser`启动Vite preview与浏览器，拦截测试websocket；不连接真实chat端点。旧playground若24删除，须从矩阵移除；若保留则先按当前支持能力更新 |
| saas-examples/oauth-playground（若24后仍保留） | frontend test/build；root `npm test`；backend普通测试；`test:nginx`只在确切需要时运行 | root dev.test注入假的run/spawnChild，不启动服务；test:nginx启动两原生Nginx、openssl和回环API，有finally清理，但fixture请求使用自签TLS豁免，**不是canonical严格CA HTTPS通过证据**。不要为运行它而改成Docker |
| platform tools/crates/ops | 按需要对15个manifest清单中的独立工具执行fmt/check/clippy/test；deployment-contract／telemetry及artifact-agent普通测试无容器 | 编译部署代理不运行其daemon；ops recovery的普通bundle/catalog测试与实际restore分开。真实恢复会运行psql/pg_dump/pg_restore/findmnt并写隔离目标，须用Phase23最终canonical工具／清单，不能直接执行旧恢复配置 |

当前15个非vendor Cargo manifest：identity、platform backend、portal backend、stores backend、work-orders backend、notification、examples websocket backend、examples oauth-playground backend；platform下deployment-contract、operational-telemetry、platform-admin、deployment-agent、artifact-agent、ops/monitoring、ops/recovery。两份business vendor telemetry是源码依赖，不能为了“多跑一个crate”创建额外服务。

## 原生 PostgreSQL 与子进程专项

| 测试入口 | 当前所有权输入／行为 | 执行时的资源边界 |
| --- | --- | --- |
| identity `src/test_support.rs` currentcanonical测试 | `IDENTITY_PHASE20_TEST_DATABASE_URL`、`IDENTITY_PHASE20_TEST_CLUSTER_ID`；校验loopback admin `/postgres`和实际cluster ID；只创建`identity_phase20_core_*`库 | 运行owner从现有55494获取真实值，经受保护环境提供；测试finish关闭pool并定向删库。失败需独立确认残留，不能只信stdout |
| identity/platform其他ignored PG测试 | `IDENTITY_PHASE01..20_TEST_DATABASE_URL`及对应cluster/marker变量随suite不同；platform `identity_deployment_tests`按传入变量读取 | 逐suite读helper后指定，不能统一 `--include-ignored` 并猜变量。旧protocol fixture在24后会删除／替换，25只执行最终current suites |
| portal `src/tests/pg.rs` | `IDENTITY_PHASE11_TEST_DATABASE_URL`与`CONTEXT_SELECTION_TEST_CLUSTER_ID`，独立portal新库／roles | 复用PG进程，独立新库并验证LOGIN ACL；不是复用平台shared库 |
| stores `tests/postgres_contracts.rs` | `TEST_DATABASE_URL`、`CI_TEST_POSTGRES_OWNER`需ci-前缀；`STORES_TEST_LOCAL_POSTGRES_OWNER`必须精确`<owner>@127.0.0.1:55494`；拒绝默认5432 | 只创建唯一`phase26_stores_sql_*`数据库，关闭pool后DROP；测试标识中的旧phase号只是可执行名称，不等于实际Phase26操作 |
| work-orders `tests/{migrations,read_contracts,write_contracts,member_contracts,reminder_contracts}.rs` | `TEST_DATABASE_URL`、ci-owner及`WORK_ORDERS_TEST_LOCAL_POSTGRES_OWNER`的精确loopback绑定；每组创建唯一库 | 共享PG55494，各自task库、受控HTTP responder和pool要全部清理；reminder结果不自动证明真实platform/identity互通 |
| notification `tests/identity_isolation.rs` | `IDENTITY_PHASE04_TEST_DATABASE_URL`与`CI_TEST_POSTGRES_OWNER`；精确loopback、端口及admin `/postgres`约束；启动当前notification子进程 | 使用owned新库，不传既有业务URL；Drop/finally关闭子进程、HTTP peer和pool。此测试未启Docker |
| platform `tests/context_selection_database.rs` | `CONTEXT_SELECTION_TEST_ADMIN_URL`、cluster ID／baseline／keys等显式输入 | 当前会自建owned测试库和局部角色，无Docker；legacy上下文suite是否保留由24决定 |
| `backend/scripts/context-selection-postgres.sh` | 原生initdb/pg_ctl，在`/tmp/rust-saas-context-selection.*`另建cluster，默认55437，finish/lease/trap清理 | **本次不运行**，55494已足够；零容器不代表可重复创建多套原生PG。只在协调方证明共享cluster不可用并重做资源计划时考虑 |
| `backend/tests/unified-auth-https/postgres.sh` | 原生PG supervisor；若有Docker CLI，只调用context/ps/volume/network/system df盘点，未create/run；要求新task路径且不允许root | **不再次start**现有55494 cluster，也不把可复用的OWNER目录传给新建脚本；现有supervisor由总owner管理，结束需先释放其namespace/service/proxy leases |
| unified-auth HTTPS environment／run-service／gateway／collector／browser proxy | Python原生gateway与collector、mount namespace、受控三个Rust service、Windows CONNECT proxy、Chromium | 不创建容器，但仍是完整临时资源集合：记录PID+startTime+namespace、端口、private ENV/keys/logs、数据库／roles。网关严格CA、不改global hosts、不接管既有Windows443。本页没有启动它们 |

推荐普通门禁清除 `TEST_DATABASE_URL`、`IDENTITY_PHASE*_TEST_*`、`CONTEXT_SELECTION_TEST_*`、`DEPLOYMENT_CONFIG_FILE`及真实服务运行秘密；只在选定PG／TLS子任务中明确提供所需值。不要用shell批量打印环境或`.env`来证明配置正确。`MICRO_PROXY_TEST_NODE`、`STORES_TEST_NODE`、`WORK_ORDERS_TEST_NODE`、`OAUTH_TEST_NODE`应是已核实的Node可执行路径，不能指向包装Docker的任意脚本。

## 执行前后需由总owner完成的核对

1. Phase24最终候选冻结后，对实际选中脚本重新做docker/testcontainers/child_process搜索，并列明正在执行的项目和suite。21／22／23新增工具必须纳入本表，不能以本页静态结果自动放行未来改动。
2. 重新核对WSL真实Docker context及`docker system df`（只读），记录已有资源；按当前计划创建／复用容器为0，不运行任何Docker编排、镜像build或deployment worker。实际需要镜像gate时先明确差距，不能以fake冒充实际镜像验证。
3. 对nativePG核对实际cluster ID和task OWNER；记录原有服务，分配唯一DB/role前缀、PID记录、动态端口和临时目录。不要触碰5432、既有443或notification进程。
4. 每组测试保留正式sanitized报告，退出处理关闭临时子进程／pool、定向删除库／role／临时key/log／测试目录；独立查询残留。共享PG/cache在后续仍需使用时由总owner保留，阶段报告明确未在该阶段释放。
5. 全目标完成或中断时，由总owner停止本次所有Cargo/Node/service后释放共享PG、外置cache租约和task目录，按OWNER核实绝对路径后清理；保留正式报告。Docker占用与WSL虚拟磁盘文件缩小是两件事，本页不宣称Windows空间已恢复。

本轮仅执行文件读取、`rg`搜索和JSON解析，新增本报告并按主代理裁决修正Phase24准备文档。没有执行功能测试、构建、Docker CLI、数据库连接、进程启动或资源清理。
