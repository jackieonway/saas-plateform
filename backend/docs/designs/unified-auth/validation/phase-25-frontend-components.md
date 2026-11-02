# Phase 25 当前前端候选与组件验证

日期：2026-10-29。组件、构建和明确 HTTP fixtures 验证 **PASS**；专属临时工件清理被自动审批审核拒绝，仍未完成。本报告不替代当前版本实际跨服务 HTTPS、身份权威或正式发布验收。

## 结果与证据

[聚合 JSON](phase-25-frontend-components.json) 记录 10 个当前项目、28 项门禁、**886/886 个 Node 测试通过、0 skip**，并产生 8 份新生产 bundle。764 项当前源码／测试／配置输入逐 SHA-256 核对；已安装直接依赖的版本逐项与各项目的锁文件匹配。Node 为 24.22.1。构建使用任务专属源码副本与新 `dist`，没有覆盖现有业务 `dist`。

| 项目 | 当前测试通过 | 其他实际检查 |
|---|---:|---|
| 平台前端 | 474 | typecheck、严格 lint、SDK 固定工件、生产构建、qiankun delivery |
| 客户门户前端 | 56 | typecheck、严格 lint、生产构建 |
| 门店前端 | 149 | 完整 build/static 测试、空 npm cache 独立安装、生产构建内 typecheck |
| 工单前端 | 90 | 完整 build/release 测试、生产构建内 release gate |
| 微应用 SDK | 30 | 在受控临时目录重新生成并逐字节核对当前交付材料 |
| WebSocket 示例 | 9 | typecheck、生产构建与实际 HTTP 静态交付 fixture |
| Vue 示例 | 12 | 当前生产构建、静态交付及生命周期 fixture |
| jQuery 示例 | 12 | 当前生产构建、静态交付及生命周期 fixture |
| 报表中心示例 | 8 | typecheck、当前生产构建 |
| 独立标准 OIDC RP／MACHINE 示例 | 46 | 静态接入检查、严格校验 CA/hostname 的 HTTPS issuer fixtures |

此外，[当前 Chromium HTTP fixture 报告](phase-25-ui-focus-fixture.json) 覆盖统一客户端管理页面 12 个场景，包括新增的移动端键盘焦点约束与 Escape 恢复。它使用真实 React/Vite 页面和生产会话门禁，认证与 API 响应为明确的隔离 HTTP fixtures；没有冒充当前真实身份权威验证。

## 保留的失败及修复

[第一轮报告](phase-25-frontend-components-r1.json) 保持 `FAIL`：门店／工单的“拒绝非系统临时目录输出”负例在位于系统 Temp 的镜像中失去了测试前提；门店独立安装测试还要求 npm 11.16.0，而系统 npm 为 11.19.1。两项构建也因未显式提供宿主来源而按预期关闭。

[定向第二轮](phase-25-frontend-components-r2.json) 在未改变源码的正常仓库路径运行这两组测试，各测试只向其专属临时目录写入。任务单独下载 npm 11.16.0，验证 npm registry 发布的 SHA-512 integrity 与实际版本，没有修改系统 npm。门店／工单的新 bundle 显式绑定 `https://console.saas.test`，149+90 个测试及两项构建全部通过。

新增浏览器回归实际重现了产品焦点缺陷：[修复前证据](phase-25-ui-focus-regression-before.json)。客户端编辑框由外部按钮控制，没有 Radix `DialogTrigger`，关闭时默认处理器无法恢复原按钮焦点。当前实现记录创建／编辑的来源元素，仅在该元素仍有效可用时恢复焦点。[第三轮](phase-25-frontend-components-r3.json) 在修复后重跑平台全部 474 个测试、typecheck、lint、固定工件检查、生产构建与 delivery gate，全部通过；新增浏览器回归连同其余 11 个场景均通过。原失败记录没有改成通过。

## 实际 HTTPS UI runner

新增 `backend/tests/unified-auth-https/phase25-ui-browser.mjs`，静态语法检查通过，实际运行结果由根执行器另行记录。它核验当前集群／数据库归属、冻结源码与 Windows runner hash，使用真实 HTTPS 和 CONNECT 代理，不绕过 TLS，不拦截响应，不保存 trace、storage-state 或秘密截图。场景包含桌面／移动端、键盘／焦点、长弹窗内部滚动、XSS 文本呈现、开放重定向拒绝、Cookie 缺失及独立 BFF 会话保持。

必须在管理员 TOTP enrollment／密钥恢复前运行，环境保持 SOURCE。XSS 测试需根环境 owner 仅在本次专属 fixture 中设置精确恶意名称与同租户多客户成员；缺少前置时 runner 明确记录 `PENDING` 并返回非零，不能视为完整通过。

## 资源与清理

构建 owner 已将 8 份新 bundle 复制到 `source-snapshot-phase25-apps-r2`，逐文件 hash 和 170 项工件内来源引用均已核对。聚合 JSON 保持原字节，SHA-256 为 `ecfb5d1552e4f013ca435954e1f9a7c3945492c4ec5866e49d5152055e134884`。

[独立清理记录](phase-25-frontend-cleanup.json) 为 `AUTOMATIC_APPROVAL_REJECTED`。自动审批审核在命令启动前拒绝了对 3 份新前端镜像与单独 npm 运行时的 OWNER／精确路径检查后定向删除，仅返回 `blocked by policy`；没有换 shell 或拆命令绕过。仍保留 **4706 个普通文件、61,607,240 个逻辑字节、917 个目录和 30 个依赖 junction**，实际删除 0。junction 指向现有依赖目录，后续清理必须仅移除链接并保留目标。

2026-10-29 的[收窄清理重试](phase-25-frontend-cleanup-retry.json) 先以只读检查再次核实四个准确 Temp 根目录、OWNER 和 30 个 junction 的类型/原依赖目标；随后提交仅非递归移除已记录 junction、核验原目标仍存在、确认无剩余 reparse point 后再删除四个精确目录的原生 PowerShell 命令。自动审批审核再次在进程启动前拒绝，仍仅返回 `blocked by policy`。命令没有执行，删除 0；未执行的完整后代计数不冒充已重新测量。已停止进一步清理尝试，完整资源清理验收保持阻塞。

本组 Docker 创建／复用均为 0，没有数据库或 WSL 环境操作；临时 Vite／Chromium 已由测试退出处理关闭，复核未发现对应 Vite 进程。现有依赖目录与正式 `dist` 被保留；HTTP 浏览器 fixture 可能刷新已有 Vite optimizer cache，因此不声称 `node_modules` 下每个缓存字节完全未变。未声称 Windows VHDX 或独占物理空间已回收。
