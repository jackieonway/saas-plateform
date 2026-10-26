# 本地开发

先按[全新安装](fresh-installation.md)准备平台共享库、门店库、工单库和客户门户库。平台唯一结构文件为 `0001_init.sql`，不运行旧升级链。数据库结构初始化与账号/RP/业务授权登记分开处理。

平台与身份服务使用准确且不同的 HTTPS origin，由本机网关与可信 CA 提供 TLS。身份服务负责账号认证与 OIDC；平台登记 PLATFORM_CONSOLE RP，客户门户另登记 CUSTOMER_PORTAL RP。准备精确 callback、独立 secret、RS256 keyset 和 BFF AEAD keyring，不能复用凭据。

平台需配置 `DATABASE_URL`、`JWT_SECRET`、`PUBLIC_APP_URL` 及[SSO 必需配置](sso-only.md)。独立身份部署还需身份服务 URL、CA、ACCOUNT_COMMAND 与 OIDC_ADMIN 服务登记，并在启动 runtime 前完成 owner/ACL 交接。按各仓库 README 设置身份、业务 API、客户门户与通知服务配置；平台不会代为启动它们。

在平台 `backend` 运行 `cargo run --locked`，在 `frontend` 运行 `npm ci`、`npm run dev`。后端默认 3001，宿主默认 3000，身份服务默认 3041。开发服务器只是内部端口；浏览器通过配置的 HTTPS 入口访问。

登录流程：访问平台 → SSO → 平台回调 → 明确选择员工成员 → 返回原页面。旧 BFF 模式开关、平台密码接口和公开 refresh 已删除。SSO 不可用时保持失败关闭。

新库不包含旧账号、会话或业务记录。受控初始化可用明确的管理员 bootstrap 配置建立最小平台数据；开发和生产首次 bootstrap 均需显式管理员配置；正常启动不再自动写入旧演示账号、租户或 OAuth 客户端。

成员、客户能力和双身份写入通过当前显式功能开关配置；三元唯一键已在基线中建立，不使用旧 rollout receipt。

修改后运行[工具链与验证](engineering-baseline.md)中的相应门禁。数据库测试要使用任务自有空库，记录实际通过范围，并清理本次运行资源；旧阶段报告不作为当前通过证据。
