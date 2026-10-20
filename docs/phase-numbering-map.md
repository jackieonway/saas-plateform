# 阶段编号对照（v0.20）

2026-10-16（Asia/Shanghai）按用户要求重新编号。当前以[架构优化台账](architecture-optimization-ledger.md)为准；本表只用于理解v0.1–v0.19和既有验证记录，不构成第二套执行顺序。

主线连续为Phase 1–34，默认按编号执行。Phase 35/36是主线完成后的独立可选扩展；Phase 37仅在Phase 22得出`NATIVE_WS_REQUIRED`时插入22→37→23，并成为Phase 34总验收的前置。条件未触发时保持NOT_STARTED，不阻塞主线完成。

Phase 1–25及37编号不变；旧35/36变为新26/27，旧26–32顺延为新28–34，旧33/34变为新35/36。已完成Phase 1–15及其证据路径完全保留。尚未创建的未来`docs/validation/phase-XX.md`按新编号创建；历史Decision Log、Change Log及`docs/validation`内已有文件保留原文/原摘要，文中编号按旧列对照，避免篡改验收证据。

| 当前编号 | 旧编号（≤v0.19） | 类型 | 阶段名称 | 当前状态 |
| --- | --- | --- | --- | --- |
| 1 | 1 | 主线 | 冻结实施基线、工具版本与验证口径 | COMPLETED |
| 2 | 2 | 主线 | 将静态兼容示例和根前端入口迁为 Vite | COMPLETED |
| 3 | 3 | 主线 | 恢复 Nginx 部署基线并交付宿主静态镜像 | COMPLETED |
| 4 | 4 | 主线 | 建立项目受管的分层 CI 与隔离测试环境 | COMPLETED |
| 5 | 5 | 主线 | 完善 ZIP 兼容静态包的原子安装与恢复对账 | COMPLETED |
| 6 | 6 | 主线 | 版本化 SDK 与公开授权契约 | COMPLETED |
| 7 | 7 | 主线 | 隔离可信微应用永久挂起的故障 | COMPLETED |
| 8 | 8 | 主线 | 分离展示更新、授权续期和版本重挂 | COMPLETED |
| 9 | 9 | 主线 | 统一菜单查询、路由策略与应用导航 | COMPLETED |
| 10 | 10 | 主线 | 协调多标签页的会话轮换与上下文 | COMPLETED |
| 11 | 11 | 主线 | 修复普通 OAuth 原子签发与实时失权 | COMPLETED |
| 12 | 12 | 主线 | 限制登录内存和密码计算成本 | COMPLETED |
| 13 | 13 | 主线 | 使关键管理变更与审计同事务 | COMPLETED |
| 14 | 14 | 主线 | 提供受控数据 grant 管理 API | COMPLETED |
| 15 | 15 | 主线 | 实现数据 grant 管理界面与授权闭环 | COMPLETED |
| 16 | 16 | 主线 | 建立 Rust 工单服务并接管业务库基线 | NOT_STARTED |
| 17 | 17 | 主线 | 实现 Rust 工单授权及只读/导出契约 | NOT_STARTED |
| 18 | 18 | 主线 | 实现 Rust 工单写入、幂等和短事务 | NOT_STARTED |
| 19 | 19 | 主线 | 迁移 Rust 工单成员目录与分配校验 | NOT_STARTED |
| 20 | 20 | 主线 | 迁移 Rust 工单提醒到平台通知API | NOT_STARTED |
| 21 | 21 | 主线 | 交付 Vue + Vite 工单前端与未知结果恢复 | NOT_STARTED |
| 22 | 22 | 主线 | 验证 Rust 实时协议兼容并冻结选型 | NOT_STARTED |
| 23 | 23 | 主线 | 迁移 Rust 通知服务并实现有界分区调度 | NOT_STARTED |
| 24 | 24 | 主线 | 拆分 OAuth 演练为 Vite 页面和 Rust 流程API | NOT_STARTED |
| 25 | 25 | 主线 | 以 Rust CLI 替换平台登记与授权工具 | NOT_STARTED |
| 26 | 35 | 主线 | 建立简单门店 Rust API 与独立业务库 | NOT_STARTED |
| 27 | 36 | 主线 | 交付门店 Vite 微应用与双消费者契约实证 | NOT_STARTED |
| 28 | 26 | 主线 | 交付工单/门店镜像与统一外部 URL 接入 | NOT_STARTED |
| 29 | 27 | 主线 | 收敛宿主加载、失败恢复和高复杂页面边界 | NOT_STARTED |
| 30 | 28 | 主线 | 优化授权查询链并收敛 Rust 能力模块 | NOT_STARTED |
| 31 | 29 | 主线 | 交付受支持升级与 dry-run 保留策略 | NOT_STARTED |
| 32 | 30 | 主线 | 建立真实指标、告警与容量基线 | NOT_STARTED |
| 33 | 31 | 主线 | 恢复加密备份与隔离新库恢复工具 | NOT_STARTED |
| 34 | 32 | 主线 | 完成独立示例交付与全仓目标栈验收 | NOT_STARTED |
| 35 | 33 | 条件 | 条件扩展：Rust跨机产物分发 | NOT_STARTED |
| 36 | 34 | 条件 | 条件扩展：平台受控镜像部署与容器生命周期 | NOT_STARTED |
| 37 | 37 | 条件 | 条件分支：原生 WS 前后端协议联合适配 | NOT_STARTED |

本次只整理文档编号和引用，未执行新阶段；当前执行指针仍是Phase 16，NOT_STARTED。
