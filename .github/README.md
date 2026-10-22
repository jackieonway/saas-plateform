# 本仓库持续集成

workflows/ci.yml 只检出本仓库，执行：

1. frontend 的依赖安装、SDK、代码检查、测试与构建。
2. backend 的 Rust 格式、检查与测试。
3. ops/monitoring、ops/recovery 的独立测试。
4. frontend、backend、ops/monitoring 的 Docker 构建。

contracts 任务单独运行 .github/tests 中的仓库结构、跨端 SDK 检查，以及 backend/tests/nginx-render-policy.mjs。前端测试与浏览器工具保存在 frontend/tests，后端不再承载 acceptance 目录。

工作流不发布镜像、执行部署或读取生产配置。数据库和真实浏览器专项验收需准备专门环境，见 [工程与验证](../backend/docs/engineering-baseline.md)。

retired-paths.json 与 scripts/cleanup-retired.ps1 只用于限定退役路径的维护核对，不参与构建；脚本默认预览，清单内 105 个路径已核对不存在。
