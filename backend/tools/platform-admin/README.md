# 平台管理 CLI

saas-platform-admin 是独立 Rust 工具，只调用平台正式管理 API。支持工单与门店，不创建客户端、不执行 SQL、不负责构建或启动容器。

## 构建

在本 platform-admin 目录执行：

```sh
cargo build --release --locked
cargo test --locked
cargo run --locked -- --help
```

可使用 target/release 下二进制的绝对路径，或由操作者安装到 PATH。Windows 产物带 .exe。

## 连接配置

远程操作读取 PLATFORM_API_ORIGIN 和 PLATFORM_ADMIN_BEARER；inspect/bootstrap 还需要 PLATFORM_CLIENT_ID，私有 CA 使用 PLATFORM_CA_FILE。秘密通过受控进程环境提供，不放命令行、清单或日志。

默认环境 production，只允许受信 HTTPS；隔离回环开发须显式 --environment development --allow-loopback-http。--app-code 为 work-orders 或 stores，--app-id 可指定真实 UUID，整条命令共享 --timeout-ms 预算。

## 命令顺序

| 命令 | 用途 |
| --- | --- |
| plan 或无参数 | 离线显示内置应用登记计划，不联网 |
| inspect | 核对应用、客户端绑定和菜单 |
| bootstrap | 默认计划，--apply 才创建缺项并绑定现有未绑定客户端 |
| grants-plan --manifest FILE | 校验并预览明确数据授权 |
| grants-apply --manifest FILE | --apply 才经正式管理 API 执行 |
| artifact-bind、artifact-verify | 用 release/artifact/inventory 文件校验本地交付声明 |
| register、release-reconcile | 按 release-file 登记或确认未知登记结果 |
| releases、deployments | 查询真实版本和部署指针 |
| preflight --release-id UUID | --apply 才提交预检 |
| activate --release-id UUID | --apply 才启用已通过版本 |
| rollback | --apply 才请求预检后回退 |
| retention-preview | 只读保留预览，没有 apply 能力 |

部分产物命令要求绝对 --release-file、--artifact-file、--inventory-dir；精确组合见 --help。离线绑定只证明材料声明一致，不证明平台元数据、容器就绪或真实 URL 可访问。

## 使用示例

二进制已放入 PATH 后，先读取再明确变更：

```sh
saas-platform-admin inspect --app-code work-orders
saas-platform-admin bootstrap --app-code work-orders
saas-platform-admin grants-plan --manifest /srv/saas/config/work-orders-grants.json
saas-platform-admin releases --app-code work-orders
saas-platform-admin retention-preview --app-code work-orders --minimum-age-days 30 --keep-latest 5
```

清单从所属业务 backend/manifests 取得，使用实际租户/组织/成员/应用 UUID。读写权限和 DATA_GRANT_POLICY 仍由平台实时限制；角色/用户集合类变更应先读完整现值，避免覆盖无关授权。

提交结果未知时先用相同意图对账，不能换文件或请求编号重复写入。完整接入见 [业务交付](../../docs/independent-business-delivery.md)，数据授权见 [管理契约](../../docs/data-grant-management.md)。
