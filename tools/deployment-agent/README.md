# saas-deployment-agent

独立 Rust 静态镜像 worker，薄入口调用私有 engine/routing 适配器。通过正常验证的 HTTPS 领取固定目标任务并发送绑定 attempt/nonce 的回执。平台不持有 Docker 权限，worker 不导入平台实现 crate，不访问平台数据库。

共用的 `deployment-contract` 仅包含封闭 DTO、摘要和固定命名/路由规则；真正的两个消费者是平台适配器和本 agent，没有业务实现依赖。第三方依赖沿用现有 reqwest-rustls、serde/json、sha2 系列，使用独立锁文件。

构建、配置、固定模板、权限、恢复/回滚及演练说明见 [受管部署手册](../../deploy/managed/README.md)。秘密只从操作者提供的绝对文件路径读取；任务不接受 shell、运行命令、端口、网络或卷。配置示例不是可运行生产配置。

新路由使用 target、应用和版本完整身份摘要生成 `route-<sha256>.conf`，避免应用名和版本中的连字符发生文件名碰撞。已有旧格式路由在内容精确匹配时继续复用，部署重试和清理不需要迁移或重命名它们。
