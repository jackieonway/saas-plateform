# qiankun ZIP 版本与节点分发

ZIP 是 qiankun 静态产物的交付格式。浏览器仍通过宿主加载 HTML/UMD，平台安装文件、记录版本并执行预检；多节点分发将规范文件集合复制到固定节点。它与环境 URL 代理、受管镜像部署分别配置。

## 1. 同机 ZIP 的前提

平台需要已有、可写的绝对目录 `MICRO_APPS_DIR`，以及独立静态服务对其版本树进行只读发布。安装形成 `/sub-apps/<app>/versions/<version>/` 的不可变目录；完整包包含入口及所有静态依赖，不携带配置秘密或包管理凭据。

基础 `compose.yml` 将 `MICRO_APPS_DIR` 固定为空，没有版本卷与 micro-static 服务。启用时必须通过已审阅的 overlay 显式传入环境、挂载本地目录，并部署静态服务和网关路由。仅修改 Compose env 文件中的 `MICRO_APPS_DIR` 不会改变基础模板的空值。

静态服务可参考 [nginx/micro-static.example.json](../nginx/micro-static.example.json) 和 `Dockerfile.micro-static`。网关 `zipStatic` 保留完整 `/sub-apps/` 路径，micro-static 随后剥离此前缀；其 `microRoot` 内直接保存 `<app>/versions/<version>/`，映射到平台或节点的实际版本根。缺资源必须 404，私有目录、点文件和元数据不公开。

## 2. 准备多节点组

多节点为可选能力，当前组有 2–8 个必需节点，全部成功才就绪。每个节点使用独立本地目录、独立 token、回环 HTTP agent 与固定 HTTPS 代理，见[节点代理](../tools/artifact-agent/README.md)。平台与节点不共享产物卷。

平台设置 `ZIP_DISTRIBUTION_CONFIG_FILE`，配置顶层 `groups`。结构示意如下；`policy` 应与各节点完整一致，替换准确公开来源、应用、节点和私有路径：

```json
{
  "groups": [{
    "policy": {
      "schemaVersion": 1,
      "id": "zip-local",
      "name": "门店静态节点",
      "publicOrigin": "https://platform.example.com",
      "applications": ["stores"],
      "nodes": [
        {"id":"node1","origin":"https://node1.example.com"},
        {"id":"node2","origin":"https://node2.example.com"}
      ]
    },
    "nodes": [
      {"nodeId":"node1","caRef":"/run/secrets/node1-ca.pem","tokenRef":"/run/secrets/node1.token"},
      {"nodeId":"node2","caRef":"/run/secrets/node2-ca.pem","tokenRef":"/run/secrets/node2.token"}
    ]
  }]
}
```

每节点 CA/token 为真实绝对私有路径，拒绝符号链接；token 使用不同的 32–512 字符 ASCII 凭据，Linux 权限不允许组或其他用户读取。来源使用正常 HTTPS 校验。平台需要显式透传变量并只读挂载配置及凭据，默认 Compose 不做这些工作。

`publicOrigin` 与平台 `MICRO_GATEWAY_ORIGIN` 一致。同一应用和公开来源不能同时归 ZIP 分发组与受管镜像模板管理。修改拓扑、策略或身份会影响历史候选回执，先评估在途任务和已发布版本。

## 3. 网关门禁

[zip-distribution.conf.example](../nginx/zip-distribution.conf.example) 演示 `auth_request` 就绪门禁与固定 HTTPS 节点。需目标 Nginx 支持相应模块，并合并到正确的 `http`/`server` 层级。按真实节点配置 CA、SNI、静态转发和网络访问控制。

门禁向平台发送原始 `request_uri`，防止解码差异绕过版本检查。保留已有私有路径拒绝、CSP 与准确来源策略，受管镜像精确路由仍有更高优先级，不将 ZIP 门禁覆盖到镜像路径。

节点 `/agent/*` 仅供平台调用，公共静态文件经受控网关提供。节点可服务自己的本地目录，不能让平台或其他节点挂载同一写入根。

## 4. 发布顺序

1. 在应用管理上传正式 ZIP，平台完成安全解包和不可变内容登记，受管分发候选保持待就绪状态。
2. 有权的平台根管理者选择版本及固定分发组，使用稳定 `requestId` 提交分发意图。结果未知时复用原意图查询。
3. 后台 worker 按 attempt、nonce、lease 领取任务，先校验已有内容，仅对明确缺失版本安装；摘要冲突和损坏目录不自动覆盖。
4. 所有必需节点提供有效回执后进入就绪。预检、激活和回滚重新核对文件树、真实静态 URL，并在指针事务中复核。
5. 通过真实宿主浏览器验证 qiankun 首次加载、卸载重进、组织切换、权限和静态资源，随后保存版本与节点证据。

平台和节点都按路径组件拒绝 `.npmrc` 等私有文件，包括嵌套位置及大小写变体。健康 200 或任务 READY 不能替代 HTML/JS 真实加载。

## 5. 故障和保留

节点离线、缺文件、错误摘要、错误 CA/token 和策略改变会阻止新切换。超时任务可以重新领取，旧 attempt 不能完成新任务。正常已有内容在重试中可重验复用；冲突目录保留供调查，不能先删除再强行重试。

删除分发配置不解除历史候选门禁。active/previous 与已打开旧页面所需资源按保留要求继续提供；当前没有节点远程删除版本接口。备份需覆盖所有实际引用文件、节点策略、路由和数据库记录，并在独立节点故障后做恢复演练，见[升级与保留](upgrade-and-retention.md)。
