# 工具链与验证基线

当前工具链为 Rust 1.97.0、Node 24.22.1、npm 11.16.0，具体版本与依赖以各独立仓库锁文件为准。数据库使用 PostgreSQL 17 或更新版本，UUIDv7 由应用生成。

Rust 仓库执行 `cargo fmt --all -- --check`、`cargo check --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked --all-targets`。前端执行当前 package.json 中的类型检查、lint、单元测试和 build；平台另运行 SDK/qiankun 门禁及浏览器套件。

数据库用任务自有的空库验证首次初始化、重复启动、checksum/结构漂移拒绝、UUIDv7 和身份约束、BFF/OIDC、账号/成员撤销、真实运行角色权限及业务读写。测试不得重用生产库或以伪造迁移记录掩盖历史冲突。

文档不固定记录跨版本的测试通过数量。实际结果、跳过项与失败必须写入本次验证报告；HTTP fixture、进程内真实处理器、真实数据库和独立部署浏览器验证分别说明。

开发验证资源遵循工作区 AGENTS.md。只删除本次或用户明确授权的资源，正式报告保留；Docker 回收与 WSL 虚拟磁盘压缩分开核查。
