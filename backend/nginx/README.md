# HTTPS 网关

本目录构建独立 Nginx 网关。默认通过私网代理平台 API 与宿主静态镜像，证书和配置以只读文件挂载。网关以 101:101 运行，监听容器 8080/8443，对外端口由 Compose 决定。

## 配置与生成

按 [config.example.json](config.example.json) 创建私有 JSON，在 backend 执行：

```sh
node nginx/render.mjs /srv/saas/private/gateway.json /srv/saas/config/nginx.conf
docker build -t saas-platform-gateway:local nginx
```

publicOrigin 是准确公开 HTTPS 来源；backend 默认 backend:3001，frontend 默认 frontend-web:8080。notification、workOrders、stores、zipStatic、versionRoutes、应用来源和镜像来源必须按真实接入显式配置。未知字段或非法上游会被拒绝，输出文件按 renderer 规则生成，不直接覆盖未审阅配置。

stores 字段受支持，需在私有 JSON 中显式补充。Nginx 配置文件、证书绝对路径与固定网关地址还需写入私有 Compose 环境文件。受信代理应使用实际地址，不能用通配范围代替。

## 可选静态与分发

Dockerfile.micro-static 与 micro-static.example.json 为独立静态版本服务，基础 Compose 不会自动挂载其版本目录。zip-distribution.conf.example 是 ZIP 节点分发的可选网关门禁模板，需要配置固定节点、正常 TLS 和 auth_request 模块。

修改后先在实际镜像中检查 Nginx 配置，再核对证书、API、静态 404 与来源策略，由部署流程明确重启或 reload。完整步骤见 [平台部署](../docs/deployment.md)。

## 平台加两个业务的配置示例

以下为同一受控 edge 网络内的配置，域名、版本和来源按实际替换；默认上游白名单之外的静态服务显式加入 allowedServices：

```json
{
  "kind": "gateway",
  "publicOrigin": "https://platform.example.com",
  "backend": "backend:3001",
  "frontend": "frontend-web:8080",
  "workOrders": "work-orders-api:3004",
  "stores": "stores-api:3031",
  "notification": null,
  "allowedServices": ["backend", "frontend-web", "work-orders-api", "stores-api", "work-orders-web", "stores-web"],
  "trustedProxyIps": [],
  "versionRoutes": [
    {"app":"work-orders","version":"1.0.0","upstream":"work-orders-web:8080"},
    {"app":"stores","version":"1.0.0","upstream":"stores-web:8080"}
  ],
  "applicationOrigins": [],
  "imageOrigins": []
}
```

versionRoutes 必须匹配前端真实构建版本与平台登记 URL，旧版本需要不同静态服务或独立版本存储继续提供，不能把所有历史 URL 都指向同一个会变化的 web 容器。通知启用后还需加入其服务白名单和对应上游。
