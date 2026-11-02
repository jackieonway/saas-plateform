# Phase 25 前端与标准客户端逐断言审查

日期：2026-10-29。本文只补充主矩阵的U/S判定，不改主矩阵，不把当前组件成功提升为实际identity互操作。依据为[层级审查](phase-25-evidence-layer-audit.md)、[frontend886](phase-25-frontend-components.json)、[当前页面12个浏览器fixture](phase-25-ui-focus-fixture.json)、[身份DOM13](phase-25-identity-dom.json)、[协议组件6](phase-25-protocol-components.json)及本轮新增[独立RP受众/issuer四项](phase-25-rp-audience-components.json)。

本轮逐文件核对下列当前源码与frontend886的输入SHA完全一致：`UnifiedClientsPage.tsx`、`unified-clients-browser.mjs`、`oauth-client-policy.mjs`、`bff-integration.mjs`、`notification-context.mjs`、独立RP `oidc.test.mjs`、`support.mjs`。没有从886总数推断矩阵已覆盖。

| 子例 | 已明确观察的事实 | 主矩阵建议与未覆盖边界 |
| --- | --- | --- |
| C03-S08 错误issuer | RP46的`standard client rejects actual HTTPS issuer failure without creating a session or disclosing upstream material`实际篡改回调iss，callback400、session未建立；新增`incorrect-issuer-valid-signature-no-session`只改签名JWT的iss，jose先验证签名有效，实际RP callback400、UserInfo0、session未建立；准确issuer对照callback303、session建立 | S可PASS：错误回调iss和错误签名issuer分别有直接负例，保留各自证据；不能称实际identity返回错iss |
| C03-S09 错误nonce | RP46同系列nonce子例令fixture签出错误nonce；实际SDK callback400、`session.authenticated=false`，响应无上游材料 | S可PASS。有效签名错误nonce真实进入标准SDK/RP，不是只测字符串比较 |
| C03-S10 错误ID Token audience | 新受众回归使用相同fixture RSA私钥与准确JWT类型，保持原claims只改aud；先用jose验证错误aud token签名有效，实际RP callback400、UserInfo请求0、session未建立；正确aud对照callback303、UserInfo1、session已建立 | S可PASS。证据名为“controlled signed token at SDK transport port”；不属于实际identity返回错aud，也不属于back-channel token |
| C07-S01 PUBLIC无secret的U | `oauth-client-policy.mjs`实际调用生产表单函数，验证SPA/NATIVE映射PUBLIC、auth method none；后端PG另有client_id-only成功与credentials=0 | U仍PARTIAL：当前真实DOM12只创建MACHINE，没有逐项断言PUBLIC页面不展示secret、不提供轮换、不向API发送secret。表单函数单元测试不能改称真实DOM观察 |
| C09-S01 首次确认的U | 身份DOM单组织场景等待`#permissions`可见、点击同意，断言consent后approve顺序 | 可记“首次panel可见且需同意”的U事实。未直接读取应用/用途/resource/scope四类文本，完整行仍PARTIAL |
| C09-S02 后续复用的U | `remembered-single-identity-needs-no-clicks`无额外点击直接approve；`remembered-approval-survives-reload-search-does-not-auto-select`断言permissions不可见，搜索后posts仍空 | U可PASS；与当前同意PG规则组合后判整行。不是完整真实identity链B |
| C09-S03 scope扩大再确认的U | 当前页面12中`390px consent group management versions members and warns about reconfirmation`检查管理侧版本与重确认告知 | 管理告知不是用户登录页扩大scope后的实际再次prompt；完整U断言仍PARTIAL。PG版本拒绝可另行PASS |
| C13-S04 旧成功/错误不覆盖新上下文 | 当前474中的`a BFF member switch retires an actual API response even when transport ignores abort`控制生产API请求promise，切换后迟到成功以AbortError退役；`old HTTP body and old HTTP errors cannot cross into a new organization when transport ignores AbortSignal`分别控制迟到成功和错误，均拒绝且新organization保持 | 这是准确的UNIT_JS/生产transport组件事实，可按层级审查P+U的“可控promise/transport组件”规则组合。不能改称真实浏览器双标签页、实际通知socket或跨服务延迟 |
| C16-S03 键盘 | 页面12的390px editor用Enter打开、Tab/Shift+Tab循环、Escape关闭，实际DOM焦点始终在modal内 | 这些键盘子事实PASS。未见身份radio纯键盘或登录/授权全关键步骤的当前fixture断言，严格按准备表整行保守PARTIAL；不要把API `.check()`等同键盘Space |
| C16-S04 焦点 | 同390px editor场景明确量测`document.activeElement`，Tab与Shift+Tab圈定焦点，Escape后回原opener；关闭没有POST | modal trap/restore的U事实PASS。没有错误后的焦点恢复直接断言时保留PARTIAL；新真实UI runner的潜在断言要等实际执行结果 |
| C16-S05 弹窗滚动 | 页面12的`390px editor fits viewport and creates a valid machine client`实际检查dialog bounds、页面无横向溢出、scrollHeight>clientHeight、滚至底部后创建按钮可见 | 长client form的U事实PASS；不自动推断另一个长consent form也被滚动验证 |
| C16-S07 XSS | 身份DOM的`customer-and-hostile-names-render-as-text`输入真实恶意组织名称，检查img数量0及准确textContent；测试服务发送CSP | 组织文本安全的U事实PASS。应用名/scope等其他显示位置与CSP响应头没有逐项断言时，完整准备表仍PARTIAL；不能把发送了CSP当作已断言策略 |
| C16-S01/S02/S08 | 桌面/移动布局与实际Cookie BLOCK已有局部fixture事实 | 按层级审查仍属于当前真实流程B：完整实际服务、四类入口流程、跨域Cookie/恢复效果未运行前继续PENDING，不能由U/S总数置PASS |

## 新受众回归的执行与保留记录

新增脚本为[phase25-rp-audience.test.mjs](../../../../tests/unified-auth-https/phase25-rp-audience.test.mjs)。它不修改RP生产或support文件，通过openid-client的`customFetch`测试port显式替换成功token response中的ID Token，签名材料仍只存在原fixture闭包中。为了保持真正的正负对照，同一签名helper分别生成aud和iss的负例/正例：每一负例只改变该单独字段，其他claim按排序后的结构逐项相等。

[r1](phase-25-rp-audience-components-r1.json)保留FAIL：最初harness用JSON属性顺序比较相同claims，错误地认为claims被改变；此时没有将任何受众负例算为通过。改为按结构排序比较后，[r2](phase-25-rp-audience-components-r2.json)实际2/2 PASS，独立RP callback/session受众对照闭合。新增签名issuer正负对照后，[r3](phase-25-rp-audience-components-r3.json)实际4/4 PASS。[汇总](phase-25-rp-audience-components.json)保留三份原报告及[前汇总](phase-25-rp-audience-summary-r2.json)的字节摘要，并验证所有RP生产/support输入与frontend886基线相同。新测试文件与各报告已记录SHA。

四个fixture组的RP及issuer TLS listener均在finally关闭，临时证书目录由既有fixture trap删除；没有创建数据库、容器或实际SaaS应用进程，没有保存原token/nonce/secret。此清理只覆盖本组新TLS fixture，不代替身份DOM、此前frontend临时目录或完整环境的清理结论。
