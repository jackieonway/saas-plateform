# 实时通知协议

独立通知服务使用 Rust Socket.IO，当前锁定 socketioxide 0.18.7、engineioxide 0.17.7，产品客户端使用 socket.io-client 4.8.4。当前没有切换原生 WebSocket 协议。

## 连接与事件

传输路径 /socket.io/，namespace /。连接后客户端发送严格对象 authenticate {token}；握手 auth 不代替事件认证，也不允许第二次认证替换绑定身份。

服务调用平台 GET /api/notifications/socket-context，绑定用户、租户、组织及 session。authenticated 仅公开 contractVersion、userId、tenantId、organizationId、expiresAt；不发送 token 或内部 sessionId。

通知事件使用固定 notification 1.0 DTO。unauthorized 包含 SESSION_INVALID 与固定提示并断开；身份变化、期限到达或无法确认授权时按失效处理。逐条 APP 提示也需要当前投递授权，不能只依赖连接时认证。

## 丢失与恢复

实时信号只提示客户端获取已经落库的消息。队列或连接过载可以丢提示，但数据库收件箱仍是权威来源。认证和重连后通过 HTTP 补拉列表、未读数与偏好，不能为补提示再次发布通知。

客户端 transports=[websocket,polling] 不等于初始 WebSocket 失败必定自动回退 polling；当前没有开启 tryAllTransports。服务端主动拒绝与普通网络断连的重连行为不同，失效会话不能无限重试复活。

## 部署与测试

通知仓库独立构建；平台通过可选 Compose 和明确的网关 /socket.io/ 上游接入。来源需精确配置，数据库用专用只读身份，私有指标不暴露公网。操作步骤见通知仓库 README 和本平台 [部署指南](deployment.md)。

聊天示例使用根传输路径 / 及 join/message 等独立事件，无平台会话或持久收件箱。通知、聊天和真实浏览器/数据库验证分别进行，单元通过不代替协议全链路验证。
