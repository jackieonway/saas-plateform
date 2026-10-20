# 独立业务交付与兼容边界

工单和门店分别交付 Vite 前端、Rust API 和独立 PostgreSQL 业务库。平台经公开 HTTPS API 提供身份、授权、成员目录和持久通知；业务工程不依赖平台实现 crate，不持有平台数据库账号或 JWT 签名密钥。

## 导出与构建

在 Node24.18.0/npm11.16.0 构建环境，从源码根目录执行：

```sh
node scripts/ci/standalone-release.mjs work-orders /absolute/new/work-orders
node scripts/ci/standalone-release.mjs stores /absolute/new/stores
```

每个独立目录保留 `business-apps/<app>/frontend`、`backend`、登记manifest和唯一的本地Rust依赖 `crates/operational-telemetry`，该crate只提供固定指标类型。发行目录README提供独立npm ci/build与cargo build --locked命令。构建无需根node_modules或平台实现；前端携带固定SDK副本。

`release-manifest.json` 记录每个源码文件SHA256和字节数；验证器拒绝内容缺失、多出或更改。导出不覆盖现有目录、不跟随链接，不包含凭据、node_modules、dist、target或退役Node工单后台。源码归档、静态构建、镜像config ID/registry digest与平台release分别核对，不能互相替代。

## 部署与授权

1. 创建平台、工单、门店独立数据库及账号，业务账号只获本业务库权限。配置到平台的正常验证HTTPS、独立机密客户端和允许来源。
2. 平台根上下文中另授grant管理权限，经API/UI配置应用、菜单、普通用户功能权限与直接成员data grants。组织管理员无隐含grant写权，发布不授予业务权限。
3. 镜像主链按[微应用部署说明](../deploy/microapps/README.md)先部署固定digest/URL和路由，再执行artifact inventory绑定、平台登记、预检、激活、普通用户业务验收。平台不持Docker socket、不生成路由。RegisterBody不存digest，digest/inventory核验位于CLI/CI；UI的URL预检不能宣称镜像身份验证。
4. ZIP兼容链按[部署说明](../deploy/README.md)使用Rust可写版本存储、Nginx只读持久卷，未完成暂存不公开。同版本不可覆写；active/previous、环境及旧浏览器引用的chunk保留，未知chunk真实404。
5. 数据库升级和保留按[升级手册](upgrade-and-retention.md)。前端回滚只切指针，不回滚业务库；完整恢复按[恢复手册](../deploy/recovery/README.md)在受控新库执行，失败目标保持隔离。

## 兼容矩阵

| 组合 | 契约和边界 |
| --- | --- |
| 当前宿主 + SDK1.0.0 + 工单Vue/Vite | MicroProps2.0、opaque iframe、Origin:null、应用Bearer；CSV/幂等/版本冲突/处理人/提醒 |
| 当前宿主 + SDK1.0.0 + 门店React/Vite | 同SDK和平台成员契约；资料/负责人、员工、精确十进制日报汇总；未知写入显式读回 |
| v1→v2→v1 | 固定版本URL及旧chunk保留；逻辑版本标签不等同不同SDK major，不保证不同源码字节 |
| 已登记旧SDK/浏览器 | 以SDK compat及实际浏览器版本报告为界，不外推任意SDK或全部稳定版本 |
| 工单Rust/业务库 | 历史0001 SHA256接管、SQLx0002/0003；历史receipt不重写，不承诺成员功能向旧后端回退 |
| 门店Rust/业务库 | SQLx0001及严格catalog fingerprint；PG17恢复仅受限规范化已知约束，不刷新fingerprint绕过校验 |
| 工单combined/门店legacy | 当前平台同时支持，失败不隐式回退，无长期正向缓存；contextVersion不是凭据 |
| Rust通知/Socket.IO | 22选择SOCKET_IO_COMPATIBLE；polling/Upgrade/鉴权/到期/重连按真实联合测试，37未触发 |
| pilot/production-pitr | 试点24h/4h；物理异机、外部密钥和生产15min/1h另验，同VM容器不能证明生产容灾 |

最终结果见[Phase34报告](validation/phase-34.md)及输入/运行索引。历史报告保留原输入和失败，不替代当前源码复验。Phase35跨机ZIP分发、36平台管容器和37原生WS均未触发，不计为已完成。外部GitHub发行、生产部署和外部告警未获本阶段授权。
