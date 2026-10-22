# 平台前端

React / TypeScript / Vite 微应用宿主，使用 Node 24.18.0、npm 11.16.0。本目录独立安装和构建。

## 本地运行

先按 [平台开发指南](../backend/docs/development.md) 启动后端，再在本 frontend 目录执行：

```sh
npm ci
npm run dev
```

访问 http://127.0.0.1:3000。/api 与 /oauth 默认代理到 http://127.0.0.1:3001，BACKEND_PROXY_TARGET 可在启动前覆盖。业务 API 和静态资源的额外代理见 vite.config.ts，不会自动启动对应服务。

## 检查和生产构建

```sh
npm run sdk:check
npm run lint
npm run typecheck
npm test
npm run build
```

npm ci 只安装锁定依赖；build 先检查 TypeScript，再生成 dist。npm test 运行本 frontend/tests 中的前端与协议测试，不读取后端源码。浏览器工具需要另装依赖，见 [浏览器验收](tests/browser/README.md)。仓库结构、SDK 跨端一致性与后端网关检查由独立 CI 任务执行。

## 镜像

仍在本目录执行；提交和构建标识替换为实际值：

```sh
docker build --build-arg SOURCE_REVISION=<完整提交SHA> --build-arg HOST_BUILD_ID=<构建标识> -t saas-platform-web:local .
```

运行镜像由非特权 Nginx 提供 8080，健康路径 /healthz。旧资源保留与发布元数据见 [静态交付](deploy/README.md)，正式入口与配置见 [部署指南](../backend/docs/deployment.md)。
