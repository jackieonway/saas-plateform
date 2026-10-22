# 公共协议与生成材料

## 协议来源

contracts/schema.json 定义公共结构。x-release-contract、x-release-limit 从 release.json 解析。生成器输出 JSON Schema Draft 2020-12、TypeScript 声明和独立 Rust serde 模块，消费者保存固定版本副本。

在 SDK 源仓库根使用 npm run build 生成，npm run check 校验，npm test 验证行为。消费者只校验自己固定的材料，不在构建时运行其他仓库的生成器。

## 解析边界

字段来自 schema 的 $defs。AbortSignal、生命周期回调、Rust 三态可空字段及有界解析使用显式模板，不能把它当作完整编译所有 JSON Schema 关键字。完整结构校验使用生成 schema，期限需要消费者解析并结合可信时钟确认。

Rust 模块只依赖 serde、serde_json，不含 HTTP、SQL、令牌刷新和业务模型。带令牌的类型不实现 Debug，解析错误统一为 PUBLIC_CONTRACT_INVALID，不保存敏感输入。

## 组织范围与授权

organizationScopeIds 有三态：缺失表示旧上下文仅限当前组织；null 表示平台根上下文；数组必须包含当前组织并符合限额，不能截断。OptionalNullable 保留序列化往返的三态。

解析器不能证明字段发送者身份。业务后端先验证可信平台传输和身份绑定，再逐请求校验令牌受众、用户/租户/组织、期限、permission、资源动作数据授权及记录归属；SQL 继续落实租户与组织范围。结构合法不等于已授权。

## 测试与固定副本

fixtures/public-wire.json 是非秘密示例，覆盖版本、结构往返、可空状态、限额、通知目标和公开/内部上下文等协议场景，不代替实际后端或 Socket.IO 验收。

固定消费副本的协议与可执行字节按来源摘要校验。中文文档可有单独本地修订记录，原始文档摘要保留在 SOURCE.json，不将翻译误认为上游协议版本升级。
