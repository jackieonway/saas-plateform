# 工单与门店的固定镜像交付

本目录把两个已经独立的前端和 Rust API 装入独立镜像。构建和测试可以使用 Node；生产请求只由 Nginx 与 Rust 处理。平台登记公开 HTML URL、预检并切换发布指针，不拉镜像、不创建容器、不生成入口路由，也不自动授予用户任何应用权限。

## 构建输入与镜像

`Dockerfile` 从仓库根部的干净源码快照构建，`APP_CODE` 只接受 `work-orders` 或 `stores`。Node 24.18.0/npm 11.16.0 安装该应用自己的 lockfile，再执行已有独立构建脚本；不会复制开发机的 `node_modules` 或 `dist`。四个公开参数为 `APP_CODE`、`APP_VERSION`、`HOST_ORIGIN`、`API_BASE`，均不是秘密。

镜像包含完整静态字节，根目录为 `/usr/share/nginx/html`，内部端口 8080、UID/GID 101。入口 Nginx 复用 `deploy/gateway/render.mjs` 的 `versionRoutes`，剥离公开的 `/sub-apps/<app>/versions/<version>/` 前缀后代理镜像根目录。镜像不需要平台产物卷。HTML 不缓存，版本资源 immutable，错误 no-store；不存在资源是真实 404，不能用 HTML fallback 吞掉 chunk 错误。

两份默认 Rust Dockerfile 在各自 `business-apps/<app>/backend` 的干净上下文使用 Cargo.lock 编译正确的独立 crate，运行阶段只有 Rust ELF、系统 CA 和探针工具，以 UID/GID 10001 执行。工单端口 3004、门店端口 3031，健康路径分别为 `/work-orders-api/health/ready`、`/stores-api/health/ready`。`NODE_ENV=production` 是现有 Rust 配置识别的变量名，不代表 Node 运行时。

CI 也支持先在本次隔离 Linux 工具容器按锁编译两份 API，再用 `Dockerfile.api-runtime` 分别装入同一已核验 ELF。该模式要求 ELF SHA-256、机器架构、源码 SHA、实际输入摘要逐一匹配；候选报告明确标为 `verified-prebuilt-ELF`。它证明独立锁编译及 runtime 装箱，不声称执行了默认 Dockerfile 的 Cargo builder。默认模式报告为 `locked-Docker-source-build`。两条路径都不能复制未验证的旧 target 文件。

候选入口：

```sh
node scripts/ci/business-images.mjs --out-dir /absolute/new-candidate --source-revision <full-40-hex-source-sha> --platform linux/amd64 --host-origin https://platform.example.com
```

它构建四个静态镜像和两个 API 镜像，输出 `candidate-result.json`、`source-manifest.json`、`images/*.tar` 与脱敏日志。基础镜像必须预先准备；报告保存实际 image ID/RepoDigest。候选保留准确的源码输入、buildId、应用、前端版本、架构、OCI manifest/config 摘要、archive SHA-256 和 image ID。构建过程若来源发生变化即失败。构建成功不等于 registry 已发布、容器已健康或平台已激活。

daemon 的实际 image inspect `.Id`、saved OCI manifest digest 和 OCI config digest 分别记录。经典 image store 的 `.Id` 可为 config digest，Docker containerd store 的 `.Id` 可为 manifest digest，`daemonImageIdKind` 明确实际含义。registry 查询的 config digest只与已验收 archive 的 config digest比较；目标 pull 的实际 `.Id` 与最终 registry manifest/config 身份对应，不能把 config digest伪造为 daemon ID。若 push 改变 manifest 编码，pull报告分别保留 `candidateImageId` 和真实 `pulledImageId`，容器健康与运行核对使用后者。

## 同域和批准的外部静态来源

同域版本使用 `/work-orders-api`、`/stores-api` 作为 API base。外部静态域名直接提供 HTML 时，应使用主平台的明确绝对 HTTPS API base，或者由该静态域名维护自己的受控代理。相对 API base 会请求当前静态域名，不能猜测它会自动转发到平台。

本阶段演练可采用 v1 同域入口和相对 API base、v2 外部入口和绝对平台 HTTPS API base；两版本始终保留。构建 helper 的 `versionApiBases[appCode][version]` 可以表达两种配置。宿主来源白名单始终是公开 HTTPS 宿主 origin，不能改成容器名。静态 CSP 仅允许该宿主嵌入，并允许明确配置的 API origin；模块/动态 chunk 使用匿名静态 CORS。opaque iframe 的业务请求仍为 `Origin: null`、当前应用 Bearer、`credentials: omit`，Rust 逐请求负责真实授权与业务 CORS。

部署方还须在宿主入口 CSP 的 `applicationOrigins` 与平台 `MICRO_ALLOWED_ORIGINS` 中明确允许实际外部 origin。例子中的 `.invalid` 域名只是占位。TLS 保持证书和主机名验证；私有 CA 通过公开证书链配置，私钥只挂入口。

## 构建、推送和目标部署分别执行

现有 `production-build.yml` 继续只读，普通 PR 不获得发行凭据。新增 `business-publish.yml` 仅允许主分支手工操作：先用只读 GitHub API 核对同一数字 repository ID、精确 managed workflow/main/run/attempt、两个必需成功 job 和唯一未过期候选 artifact，再在 `business-image-publish` 受保护环境中检出已核实 SHA、下载精确 artifact ID。逐文件来源及全部 OCI archive 校验成功后才读取 registry 登录秘密，推送同一已验收 archive，不重新构建。

受保护环境应配置 `BUSINESS_REGISTRY`、`BUSINESS_REPOSITORY_PREFIX` 两个变量，以及 `BUSINESS_REGISTRY_USERNAME`、`BUSINESS_REGISTRY_PASSWORD` 两项秘密。环境保护和允许的 registry/repository 由部署方配置。该 workflow 不连接目标主机，也不执行目标部署；本地提供文件不表示已经创建环境保护、触发 Actions 或发布过外部镜像。

`publication-result.json` 从真实 push 和 registry manifest 查询记录最终 `repository@sha256:...`、config identity、源码和 archive 关联。registry manifest digest 可能与 saved OCI archive manifest digest 不同，不能互相冒充。生产环境使用 publication 清单的固定 digest；短期候选 artifact 与 90 天 publication artifact 只是传递证据，权威发行绑定须另行保存于持久 inventory。

## Linux Compose 与健康门禁

复制 `deploy/compose.business.env.example` 到检出目录之外的受限文件，并填入六个真实固定 digest、两个独立业务数据库/最小权限账号，以及各自实际的机密客户端凭据。两个服务不读取平台身份表。平台、API 镜像和业务迁移分别记录版本；前端回滚不隐含 API 或数据库回滚。

```sh
sh deploy/microapps/pull-start.sh /absolute/checkout /secure/platform.env /secure/business.env
```

该 Shell 脚本验证镜像均为固定 digest，执行真正 `compose pull`，再以 `--no-build --pull never --wait` 启动并复查六服务的实际 Docker health。只入口发布 80/443，业务和静态容器均只读、drop ALL、no-new-privileges，不挂源码、静态产物卷或 Docker socket。需要私有 CA 时额外合并 `deploy/compose.business-ca.yml` 执行等价 Compose 命令，挂入 UID10001 可读的公开 CA 链，不能传空 `PLATFORM_CA_CERT_FILE`。

先运行平台及公开 HTTPS 基础入口，再启动业务；不要让入口反向等待通过入口访问平台的服务而形成健康依赖循环。业务 ready 只检查业务数据库/schema，不能据此推断平台授权有效。生产 `PLATFORM_API_ORIGIN` 仍是可验证的公开 HTTPS origin，容器必须能按正常 DNS/网络访问它。

推进新版本使用以下顺序，任一步失败均停止后续步骤：

1. 固定 digest pull 和候选容器启动；检查当前所有目标容器确实 healthy，且运行的 image ID/ref 与清单一致。
2. 使用当前 Rust CLI 对本次 release/artifact 执行 `artifact-bind --apply`，再 `artifact-verify`；所有环境共用同一权威、持久的本地 inventory 目录。它保留 app/version 和公开版本 URL 的不可变来源及 digest，不证明部署成功。
3. 在构建/部署准备机渲染候选 Nginx 配置，保留全部旧版本路由。使用候选配置真实执行 `nginx -t`；再次检查 Docker health 后原子发布配置，并受控重建入口或 reload。若配置文件是单文件 bind mount，host 上原子换 inode 后应重建入口以重新挂入新文件，不能假定旧容器 reload 会读取新 inode。
4. 用带 `--artifact-file` 和 `--inventory-dir` 的 Rust `register --apply` 登记，正文仍只有既有六字段，得到 DRAFT；随后单独 preflight，只有正文 `validationStatus=PASSED` 才允许 activate。HTTP200 加 FAILED 仍是失败。未知写入结果先执行 `release-reconcile` 并读取 deployments，不能自动重发或激活。

`bindBusinessArtifacts` helper 可以实际调用冻结 Rust CLI（也可由受管 executor 在 Linux 工具容器执行），读取 APPLIED/VERIFIED/BOUND、同一 journal sequence 以及无远程写入结果。`promoteBusinessRoutes` 按上述顺序调用真实健康检查、typed bind/verify、配置测试、登记、预检与激活操作；纯单元回调结果只验证控制流，实际通过须由隔离 runtime 报告证明。

平台没有保存镜像 digest，也不能读取 Docker health。健康拒绝由 CI/Compose 记录；入口/资源或配置的 healthUrl 预检拒绝由平台记录。需要分别证明 unhealthy 候选没有推进路由/登记/激活，以及真实 HTTP200+FAILED/激活拒绝时 active 指针保持。

## 保留、回退与隔离 registry 演练

示例 `gateway.example.json` 保留每应用 v1/v2 的独立 service 路由。未知版本继续 404。平台回滚只切 active/previous 指针；两版本容器和路由保持在线，旧页面所引用的动态 chunk 仍从原版本地址读取。仅保留旧镜像而停止旧 URL 不满足要求。本目录没有自动删除版本、chunk、发行 journal、数据库或 registry 数据的操作。

隔离演练允许显式 `scope: isolated` 的随机 loopback HTTP registry，未降低生产 registry 或应用 HTTPS 验证。实际执行 push、读取 registry descriptor、停止候选容器并删除仅本次 owner 的候选 image references，证明候选 image ID 与 fixed ref 都不存在，然后执行 fixed digest pull、核对 RepoDigest/config identity、再启动并访问真正 HTTPS。保留 registry 的 GET manifest/blob 日志、pull 输出、容器健康和应用链证据。

演练沿用同一 daemon，layer/build cache 可能仍然存在，报告固定记录 `MAY_REMAIN_NOT_A_CLEAN_DAEMON`，不能宣称全新 daemon。`docker load` 仅可用于发行工作流加载已验收 archive，不能替代目标端的 registry pull 验收。

轻量策略测试：

```sh
node --test tests/business-images-policy.mjs deploy/microapps/tests/static-config.mjs
```

它们不运行 Docker、Rust 编译或浏览器，不能作为上述运行验收结果。Phase28 的实际证据由本阶段新 harness/report 提供，Phase27 原始与正式证据保持冻结。
