# 发布日志

## 未发布 · 2026-10-24 · 门店组织模型

- 组织类型新增 `STORE`（门店），组织管理统一维护门店身份、名称、编码、层级、地址及成员；其他组织类型和默认公司类型保留。
- 增加应用受控组织目录，按显式 `organizationLookups` 用途、应用权限、数据范围和有效 STORE 校验返回短期组织投影。已知组织元数据不代替业务记录的 SELF owner 校验，SDK `2.0.0` 和 MicroProps 无需扩展。
- 门店管理 CLI 同步九个菜单、七项功能权限及对应清单，移除独立成员分配与解除用途。组织类型通过增量迁移 `0011` 扩展，已有组织不自动改为门店。
- 现行接口和本机接入方式见[应用组织目录](backend/docs/application-organization-directory.md)。本次源码改动需要重新构建交付，2026-10-23 的迁移候选和验收快照保留其原始含义。

## 1.0.1 — 2026-10-23

本次为 Node.js 安全维护版本：宿主前端 `saas-platform-ui` 和浏览器检查工具 `saas-platform-browser-tests` 从 `1.0.0` 升级为 `1.0.1`，对应锁文件同步更新。

- 将 Node.js 基线从受影响的 `24.18.0` 升级至安全修复版 `24.22.1`，同步 `.nvmrc`、`.node-version`、两个包与锁文件的 `engines.node`（`>=24.22.1 <25`）、前端 Docker 构建镜像及当前开发/CI 文档。CI 从 `frontend/.nvmrc` 读取版本。
- 根据 [Node.js 2026 年 10 月安全公告](https://nodejs.org/en/blog/vulnerability/october-2026-security-releases)及 [24.22.1 发行说明](https://nodejs.org/en/blog/release/v24.22.1)，升级覆盖 Node.js 24.x 涉及的 8 项漏洞：CVE-2026-105482、CVE-2026-105483、CVE-2026-106069、CVE-2026-111497、CVE-2026-105481、CVE-2026-106072、CVE-2026-106074、CVE-2026-106073，涉及 DNS、TLS 证书校验、Brotli 解码、HTTP keep-alive、随机数缓存及权限模型。
- Node.js 用于前端开发、构建、仓库脚本和浏览器测试。生产后端由 Rust 运行，前端运行镜像由 Nginx 提供静态文件。现有开发机、CI 与测试环境需切换到修复版本；重新构建并部署前端镜像后，新构建基线才会进入实际交付。

npm 继续使用项目声明的 `>=11.16.0 <12` 范围及现有锁定依赖；Rust 包版本、固定 SDK 和历史发布产物保持各自版本。

验证使用 Node.js `24.22.1`、npm `11.19.1`（符合项目 engines 范围），复用现有锁定依赖：

- `npm run sdk:check`、`npm run qiankun:check`、`npm run typecheck` 通过。
- `npm test` 通过 396 项测试，无失败或跳过；`npm run lint` 无错误，保留 47 条既有警告。
- `npm run build` 通过，构建后的 qiankun 交付门禁检查 389 份文本产物。
- 根目录运行 `node --test .github/tests/repository-structure.mjs .github/tests/sdk-consumer-contract.mjs backend/tests/nginx-render-policy.mjs`：20 项通过，1 项真实 Nginx 集成测试按现有条件跳过。
- 包版本、Node 版本文件、Docker 基线和两个锁文件的根元数据一致性校验通过。Docker 镜像构建及部署未执行；额外的 Edge 浏览器入口在通知套件通过 8 个场景后主动停止，完整浏览器入口未完成。
