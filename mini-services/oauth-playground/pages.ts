/**
 * oauth-playground 页面模板 (纯字符串 HTML, 原生 CSS, 暗色 + teal 点缀, 无框架)。
 *
 * 注意: 页面内联 <script> 一律使用字符串拼接 (不含反引号与 ${), 避免与 TS 模板字面量转义纠缠。
 * 浏览器直连授权服务器 :3001 (CorsLayer::permissive, 跨源 fetch / 预检均放行)。
 */

type NavKey = "home" | "device" | "auth" | "cc" | "pwd" | "ext"

function layout(opts: { title: string; active: NavKey; body: string; script?: string }): string {
  const nav = (href: string, key: NavKey, label: string) =>
    `<a href="${href}"${opts.active === key ? ' class="active"' : ""}>${label}</a>`
  return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${opts.title} · OAuth Playground</title>
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
html { color-scheme: dark; }
body {
  font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
  background: #0a0f12; color: #d5e2e5; min-height: 100vh;
}
a { color: #2dd4bf; }
.container { max-width: 900px; margin: 0 auto; padding: 20px 20px 64px; }
header {
  display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 10px;
  padding: 14px 0 16px; border-bottom: 1px solid #1c2a30; margin-bottom: 26px;
}
.brand { font-weight: 700; font-size: 18px; color: #e8f2f4; letter-spacing: .4px; }
.brand .accent { color: #2dd4bf; }
.tag {
  font-size: 11px; color: #8aa0a8; border: 1px solid #24343b; border-radius: 999px;
  padding: 3px 10px; margin-left: 10px; vertical-align: 2px; background: #0e171b; white-space: nowrap;
}
nav a { color: #8aa0a8; text-decoration: none; margin-left: 16px; font-size: 13px; }
nav a:hover, nav a.active { color: #2dd4bf; }
h1 { font-size: 22px; margin-bottom: 10px; color: #eef6f7; }
.h1sub { font-size: 12px; font-weight: 500; color: #7d949c; margin-left: 10px; vertical-align: 3px; }
.sub { color: #8aa0a8; font-size: 13px; line-height: 1.8; margin-bottom: 24px; }
.sub code { font-size: 12px; }
.card {
  background: #0e171b; border: 1px solid #1c2a30; border-radius: 12px;
  padding: 20px 22px; margin-bottom: 18px;
}
.card h2 { font-size: 14px; color: #2dd4bf; margin-bottom: 10px; letter-spacing: .4px; }
.card h2.subhead { margin-top: 22px; padding-top: 16px; border-top: 1px dashed #1c2a30; }
.card p { font-size: 13px; color: #9fb3ba; line-height: 1.8; margin-bottom: 10px; }
code, .mono {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace;
  font-size: 12px; color: #a8d8d2; background: #122026; padding: 2px 6px; border-radius: 4px;
  word-break: break-all;
}
pre.mono { display: block; padding: 12px; overflow-x: auto; line-height: 1.6; white-space: pre-wrap; }
button {
  background: #14b8a6; border: none; color: #03211d; font-weight: 600; font-size: 13px;
  padding: 9px 18px; border-radius: 8px; cursor: pointer; transition: background .15s;
}
button:hover { background: #2dd4bf; }
button:disabled { background: #24343b; color: #67818a; cursor: not-allowed; }
button.secondary { background: transparent; border: 1px solid #24343b; color: #9fb3ba; font-weight: 500; }
button.secondary:hover { border-color: #2dd4bf; color: #2dd4bf; background: rgba(45, 212, 191, .06); }
button.copy { padding: 4px 12px; font-size: 12px; }
.btnrow { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin: 12px 0 8px; }
.status { font-size: 12px; color: #8aa0a8; }
.errbox {
  background: rgba(239, 68, 68, .08); border: 1px solid rgba(239, 68, 68, .35); color: #fda4a4;
  border-radius: 8px; padding: 12px 14px; font-size: 13px; line-height: 1.7; margin: 12px 0;
  word-break: break-all;
}
.okbox {
  background: rgba(45, 212, 191, .07); border: 1px solid rgba(45, 212, 191, .3); color: #99f6e4;
  border-radius: 8px; padding: 12px 14px; font-size: 13px; line-height: 1.7; margin: 12px 0;
}
.kv { display: flex; align-items: baseline; gap: 10px; padding: 7px 0; font-size: 13px; flex-wrap: wrap; }
.kv .k { color: #7d949c; min-width: 210px; font-size: 12px; }
.kv .v { color: #c9dadd; }
.uc-wrap { text-align: center; padding: 18px 10px 14px; border: 1px dashed #24343b; border-radius: 10px; margin: 14px 0; }
.uc-label { font-size: 11px; color: #7d949c; letter-spacing: 2px; margin-bottom: 6px; }
.user-code { font-size: 44px; font-weight: 800; letter-spacing: 6px; color: #2dd4bf; font-family: ui-monospace, Menlo, monospace; }
.vuc-link { display: inline-block; margin-top: 10px; font-size: 13px; word-break: break-all; }
.uc-meta { margin-top: 8px; font-size: 12px; color: #8aa0a8; }
.uc-meta strong { color: #ffd28a; font-variant-numeric: tabular-nums; }
.logbox {
  background: #0a1114; border: 1px solid #1a282e; border-radius: 8px; padding: 10px 12px;
  max-height: 280px; overflow-y: auto; font-family: ui-monospace, Menlo, Consolas, monospace;
  font-size: 12px; line-height: 1.8; margin-top: 12px;
}
.log-line { color: #9fb3ba; word-break: break-all; }
.log-line.ok { color: #5eead4; }
.log-line.err { color: #fda4a4; }
.log-line.pending { color: #ffd28a; }
.log-line.slow { color: #f0a35e; }
.log-line.info { color: #7dd3fc33; color: #8fb8c4; }
details { margin-top: 12px; font-size: 12px; color: #8aa0a8; }
details summary { cursor: pointer; user-select: none; }
details[open] summary { margin-bottom: 8px; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 16px; margin-bottom: 20px; }
a.card-link { text-decoration: none; color: inherit; display: block; }
a.card-link:hover .card.flat { border-color: #2dd4bf66; background: #101b20; }
.card.flat { margin-bottom: 0; height: 100%; }
.steps { margin: 10px 0 0 18px; }
.steps li { font-size: 12.5px; color: #9fb3ba; line-height: 1.9; }
.client { font-size: 12px; color: #7d949c; }
.enter { display: inline-block; margin-top: 14px; font-size: 13px; color: #2dd4bf; font-weight: 600; }
footer { margin-top: 30px; padding-top: 16px; border-top: 1px solid #1c2a30; font-size: 11.5px; color: #62777f; line-height: 1.8; }
.jump-note { color: #ffd28a; font-size: 12px; }
</style>
</head>
<body>
<div class="container">
<header>
  <div class="brand">OAuth <span class="accent">Playground</span><span class="tag">沙箱内部测试工具 · 仅本机可达</span></div>
  <nav>
    ${nav("/", "home", "首页")}
    ${nav("/device-flow", "device", "设备授权")}
    ${nav("/auth-code-flow", "auth", "授权码")}
    ${nav("/client-credentials", "cc", "客户端凭据")}
    ${nav("/password-flow", "pwd", "资源密码")}
    ${nav("/external-api", "ext", "第三方对接")}
  </nav>
</header>
${opts.body}
<footer>
  Dante Cloud 授权服务器 <span class="mono">http://localhost:3001</span> · 前端(consent/device 批准页) <span class="mono">http://localhost:3000</span> · 本工具 <span class="mono">:3004</span> 纯 node:http 零依赖 · 沙箱内部测试工具, 网关不转发, 仅本机可达
</footer>
</div>
${opts.script ? `<script>\n${opts.script}\n</script>` : ""}
</body>
</html>`
}

/* ==================================================================== */
/* 首页 /                                                                */
/* ==================================================================== */

export const homePage = layout({
  title: "首页",
  active: "home",
  body: `
<h1>OAuth Playground <span class="h1sub">Dante Cloud 授权服务器流程演练</span></h1>
<p class="sub">四条 OAuth 流程的端到端演练工具: 所有请求真实调用授权服务器 <code>http://localhost:3001</code> (授权服务器 CORS 全放开, 页面可浏览器直连), 关键步骤由本服务服务端代理 (client_secret 只存服务端, 不落页面)。批准/consent 操作在前端 <code>http://localhost:3000</code> 完成 (登录 admin/admin123)。</p>

<div class="grid">
  <a class="card-link" href="/device-flow">
    <div class="card flat">
      <h2>📺 设备授权测试 (RFC 8628)</h2>
      <p class="client">client <code>smart-tv</code> (PUBLIC · scope profile · 无 secret)</p>
      <ol class="steps">
        <li>设备 POST <code>/oauth/device/authorization</code> → device_code + user_code</li>
        <li>用户打开 verification_uri_complete → 前端 <code>/device</code> 登录后批准</li>
        <li>设备按 interval(5s) 轮询 POST <code>/oauth/token</code> (authorization_pending / slow_down)</li>
        <li>批准后拿到 access_token → GET <code>/oauth/userinfo</code> 展示真实用户名</li>
      </ol>
      <span class="enter">进入设备授权测试 →</span>
    </div>
  </a>
  <a class="card-link" href="/auth-code-flow">
    <div class="card flat">
      <h2>🔐 授权码测试 (Authorization Code + PKCE S256)</h2>
      <p class="client">client <code>dante-web</code> (CONFIDENTIAL · redirect_uri <code>http://localhost:3004/callback</code>)</p>
      <ol class="steps">
        <li>服务端生成 code_verifier + S256 code_challenge + state</li>
        <li>跳转 <code>/oauth/authorize</code> (经前端 :3000 代理) → consent 授权确认页</li>
        <li>用户批准 → 302 回调 <code>/callback?code=…&amp;state=…</code></li>
        <li>POST <code>/oauth/token</code> (Basic 认证 + code_verifier) 换 access/refresh</li>
        <li>GET <code>/oauth/userinfo</code> 展示用户名 · refresh_token 轮换测试</li>
      </ol>
      <span class="enter">进入授权码测试 →</span>
    </div>
  </a>
  <a class="card-link" href="/client-credentials">
    <div class="card flat">
      <h2>🤖 客户端凭据测试 (client_credentials)</h2>
      <p class="client">client <code>api-service</code> (CONFIDENTIAL · Basic 认证 · scope profile)</p>
      <ol class="steps">
        <li>api-service 携 Basic 凭据 POST <code>/oauth/token</code> → 纯客户端身份令牌</li>
        <li>POST <code>/oauth/introspect</code> (RFC 7662) 服务间验证令牌活性</li>
        <li>POST <code>/oauth/revoke</code> (RFC 7009) 吊销后再内省 → <code>active:false</code> 闭环</li>
        <li>用此令牌调 <code>/oauth/userinfo</code> → 观察无用户上下文的 401 invalid_token (教学点)</li>
      </ol>
      <span class="enter">进入客户端凭据测试 →</span>
    </div>
  </a>
  <a class="card-link" href="/password-flow">
    <div class="card flat">
      <h2>🔑 资源密码测试 (password)</h2>
      <p class="client">client <code>resource-portal</code> (CONFIDENTIAL · form 通道认证 · scope profile,email)</p>
      <ol class="steps">
        <li>表单提交用户名/密码 → POST <code>/oauth/token</code> (grant_type=password)</li>
        <li>GET <code>/oauth/userinfo</code> 展示用户多租户上下文 (租户/组织/角色)</li>
        <li>refresh_token 轮换演示 (旧 refresh 立即失效)</li>
        <li>错误凭据演示 → <code>invalid_grant</code> 精确文案</li>
      </ol>
      <span class="enter">进入资源密码测试 →</span>
    </div>
  </a>
  <a class="card-link" href="/external-api">
    <div class="card flat">
      <h2>🌐 第三方应用对接 (菜单 / 角色对外 API)</h2>
      <p class="client">client <code>api-service</code> 关联应用 <code>athena</code> (数据分析)</p>
      <ol class="steps">
        <li>client_credentials 获取服务身份令牌 (同客户端凭据页)</li>
        <li>GET <code>/api/external/menus</code> 拉取关联应用菜单树 (导航渲染/权限同步)</li>
        <li>GET <code>/api/external/roles</code> 拉取授权组织与角色清单 + 菜单授权数</li>
        <li>教学点: 未关联应用的客户端 → 404; 关联配置在 OAuth 客户端管理页</li>
      </ol>
      <span class="enter">进入第三方对接演示 →</span>
    </div>
  </a>
</div>

<section class="card">
  <h2>环境速览</h2>
  <div class="kv"><span class="k">授权服务器 (issuer)</span><span class="v mono">http://localhost:3001 · /oauth/authorize · /oauth/token · /oauth/device/authorization · /oauth/userinfo</span></div>
  <div class="kv"><span class="k">前端 (批准/consent 页)</span><span class="v mono">http://localhost:3000 · 登录 admin/admin123 (平台租户)</span></div>
  <div class="kv"><span class="k">本工具</span><span class="v mono">http://localhost:3004 · 纯 node:http, 无框架, node --watch 热重载</span></div>
  <div class="kv"><span class="k">发现文档</span><span class="v mono"><a href="http://localhost:3001/oauth/.well-known/openid-configuration" target="_blank" rel="noopener">GET /oauth/.well-known/openid-configuration</a></span></div>
  <div class="kv"><span class="k">预置账号</span><span class="v mono">admin/admin123 (也可用 wangwu/Ww@12345 等普通用户体验授权)</span></div>
</section>`,
})

/* ==================================================================== */
/* 设备授权测试页 /device-flow                                            */
/* ==================================================================== */

const deviceFlowScript = `
(function () {
  "use strict";
  var BACKEND = "http://localhost:3001";
  var FRONTEND = "http://localhost:3000";
  var st = { deviceCode: null, interval: 5, expiresAt: 0, pollTimer: null, cdTimer: null, done: false };

  function $(id) { return document.getElementById(id); }
  function ts() { return new Date().toTimeString().slice(0, 8); }
  function trunc(s, n) { return s.length > n ? s.slice(0, n) + "…" : s; }
  function log(kind, text) {
    var box = $("poll-log");
    var line = document.createElement("div");
    line.className = "log-line " + kind;
    line.textContent = "[" + ts() + "] " + text;
    box.appendChild(line);
    box.scrollTop = box.scrollHeight;
  }
  function copyText(text, btn) {
    if (!text) return;
    navigator.clipboard.writeText(text).then(function () {
      btn.textContent = "已复制 ✓";
      setTimeout(function () { btn.textContent = "复制"; }, 1400);
    });
  }
  function setButtons() {
    $("btn-poll").disabled = !st.deviceCode || st.done || !!st.pollTimer;
    $("btn-stop").disabled = !st.pollTimer;
  }
  function stopPoll(reason) {
    if (st.pollTimer) { clearInterval(st.pollTimer); st.pollTimer = null; }
    st.done = true;
    setButtons();
    if (reason) {
      log("err", reason);
      var box = $("final-error");
      box.hidden = false;
      box.textContent = reason;
    }
  }
  function armTimer() {
    if (st.pollTimer) clearInterval(st.pollTimer);
    // +500ms 缓冲: setInterval 的毫秒抖动会让 5.0s 间隔实测落到 4.9x s,
    // 触发授权服务器 last_polled_at 边界判定而无谓 slow_down (实测复现过)
    st.pollTimer = setInterval(pollOnce, st.interval * 1000 + 500);
  }
  function renderUserinfo(box, u) {
    box.textContent = "";
    var name = document.createElement("strong");
    name.textContent = u.preferred_username || u.sub || "?";
    name.style.color = "#2dd4bf";
    box.appendChild(name);
    var rest = document.createElement("span");
    var tenant = (u.tenant && u.tenant.name) ? u.tenant.name : "-";
    var org = (u.organization && u.organization.name) ? u.organization.name : "-";
    var roles = (u.roles || []).join(", ") || "-";
    rest.textContent = " · 真实姓名: " + (u.real_name || "-") + " · 租户: " + tenant + " · 组织: " + org + " · 角色: " + roles + " · scope: " + (u.scope || "-");
    box.appendChild(rest);
  }
  async function pollOnce() {
    if (st.done || !st.deviceCode) return;
    var body = new URLSearchParams();
    body.set("grant_type", "urn:ietf:params:oauth:grant-type:device_code");
    body.set("device_code", st.deviceCode);
    body.set("client_id", "smart-tv");
    try {
      var res = await fetch(BACKEND + "/oauth/token", {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: body.toString()
      });
      var data = await res.json();
      if (data && data.access_token) {
        log("ok", "✅ 成功 access_token=" + trunc(data.access_token, 20) + "… token_type=" + data.token_type + " expires_in=" + data.expires_in + "s scope=" + (data.scope || "-"));
        stopPoll(null);
        $("token-result").hidden = false;
        $("access-token").textContent = trunc(data.access_token, 56) + "…";
        $("refresh-token").textContent = data.refresh_token ? trunc(data.refresh_token, 24) + "…" : "(未返回)";
        $("btn-copy-at").onclick = function () { copyText(data.access_token, this); };
        $("btn-copy-rt").onclick = function () { copyText(data.refresh_token || "", this); };
        $("raw-token").textContent = JSON.stringify({
          access_token: trunc(data.access_token, 20) + "…",
          token_type: data.token_type,
          expires_in: data.expires_in,
          scope: data.scope,
          refresh_token: data.refresh_token ? trunc(data.refresh_token, 12) + "…" : null
        }, null, 2);
        var ures = await fetch(BACKEND + "/oauth/userinfo", { headers: { "Authorization": "Bearer " + data.access_token } });
        if (ures.ok) {
          var u = await ures.json();
          renderUserinfo($("userinfo"), u);
          log("ok", "GET /oauth/userinfo → " + (u.preferred_username || u.sub) + " (" + ((u.real_name) || "-") + ")");
        } else {
          $("userinfo").textContent = "userinfo 调用失败 HTTP " + ures.status;
          log("err", "userinfo 调用失败 HTTP " + ures.status);
        }
        return;
      }
      var err = (data && data.error) || ("HTTP " + res.status);
      var desc = (data && data.error_description) || "";
      if (err === "authorization_pending") {
        log("pending", "authorization_pending — 等待用户批准 (打开上方链接, 在前端登录后点「批准」)");
      } else if (err === "slow_down") {
        st.interval += 5;
        log("slow", "slow_down — 轮询过频, interval 自动调整为 " + st.interval + "s (RFC 8628)");
        armTimer();
      } else {
        stopPoll("轮询终止: " + err + (desc ? " (" + desc + ")" : ""));
      }
    } catch (e) {
      stopPoll("网络错误: " + e.message);
    }
  }
  $("btn-poll").addEventListener("click", function () {
    if (!st.deviceCode || st.pollTimer) return;
    st.done = false;
    $("final-error").hidden = true;
    log("info", "开始轮询 POST " + BACKEND + "/oauth/token · interval=" + st.interval + "s");
    setButtons();
    pollOnce();
    armTimer();
  });
  $("btn-stop").addEventListener("click", function () {
    stopPoll("手动停止轮询");
  });
  $("btn-start").addEventListener("click", async function () {
    var btn = this;
    btn.disabled = true;
    btn.textContent = "正在发起…";
    $("start-error").hidden = true;
    try {
      var res = await fetch("/api/device-start", { method: "POST" });
      var data = await res.json();
      if (!res.ok || !data.device_code) {
        throw new Error((data && (data.error_description || data.error)) || ("HTTP " + res.status));
      }
      st.deviceCode = data.device_code;
      st.interval = data.interval || 5;
      st.expiresAt = Date.now() + (data.expires_in || 600) * 1000;
      st.done = false;
      $("user-code").textContent = data.user_code;
      var vuc = data.verification_uri_complete || "";
      var href = vuc.indexOf("http") === 0 ? vuc : FRONTEND + vuc;
      $("vuc-link").href = href;
      $("vuc-link").textContent = href;
      $("interval-hint").textContent = "每 " + st.interval + " 秒轮询一次, slow_down 时自动 +5s";
      $("raw-start").textContent = JSON.stringify({
        device_code: trunc(data.device_code, 12) + "…",
        user_code: data.user_code,
        verification_uri: data.verification_uri,
        verification_uri_complete: data.verification_uri_complete,
        expires_in: data.expires_in,
        interval: data.interval
      }, null, 2);
      $("start-result").hidden = false;
      setButtons();
      if (st.cdTimer) clearInterval(st.cdTimer);
      st.cdTimer = setInterval(function () {
        var left = Math.max(0, Math.round((st.expiresAt - Date.now()) / 1000));
        $("countdown").textContent = String(left);
        if (left <= 0) {
          clearInterval(st.cdTimer);
          stopPoll("设备码已过期 (expires_in 倒计时归零), 轮询终止");
        }
      }, 1000);
      log("info", "已发起新设备授权 user_code=" + data.user_code + " · expires_in=" + data.expires_in + "s · interval=" + data.interval + "s");
    } catch (e) {
      var box = $("start-error");
      box.hidden = false;
      box.textContent = "发起设备授权失败: " + e.message + " — 请确认授权服务器 :3001 与本服务 :3004 均在运行";
    } finally {
      btn.disabled = false;
      btn.textContent = "模拟设备发起授权";
    }
  });
})();
`

export const deviceFlowPage = layout({
  title: "设备授权测试",
  active: "device",
  script: deviceFlowScript,
  body: `
<h1>设备授权流程测试 <span class="h1sub">RFC 8628 · client smart-tv (PUBLIC · scope profile)</span></h1>
<p class="sub">本页模拟一台输入受限设备 (智能电视): 向授权服务器申请设备码, 引导用户去前端批准, 再按 interval 轮询令牌端点。<strong>批准操作需要在 <a href="http://localhost:3000" target="_blank" rel="noopener">前端 :3000</a> 登录 (admin/admin123)</strong>, 直接点下方「打开前端批准页」链接即可。授权服务器返回的 verification_uri 是相对路径 (PUBLIC_APP_URL 为空), 页面已按前端 origin 自动补全。</p>

<section class="card">
  <h2>① 设备发起授权</h2>
  <p>POST <code>http://localhost:3001/oauth/device/authorization</code> · form: <code>client_id=smart-tv&amp;scope=profile</code> (PUBLIC 客户端无 secret, 由本服务 :3004 服务端代理发起)</p>
  <div class="btnrow"><button id="btn-start">模拟设备发起授权</button></div>
  <div id="start-error" class="errbox" hidden></div>
  <div id="start-result" hidden>
    <div class="uc-wrap">
      <div class="uc-label">USER CODE</div>
      <div class="user-code" id="user-code">----</div>
      <a id="vuc-link" class="vuc-link" target="_blank" rel="noopener"></a>
      <div class="uc-meta">有效期剩余 <strong id="countdown">--</strong> 秒 · <span id="interval-hint"></span></div>
    </div>
    <details><summary>设备授权端点原始响应 JSON (device_code 已截断)</summary><pre id="raw-start" class="mono"></pre></details>
  </div>
</section>

<section class="card">
  <h2>② 设备端轮询令牌端点</h2>
  <p>POST <code>http://localhost:3001/oauth/token</code> · form: <code>grant_type=urn:ietf:params:oauth:grant-type:device_code&amp;device_code=…&amp;client_id=smart-tv</code> — 浏览器直接跨源轮询 (授权服务器 CORS 全放开)。authorization_pending / slow_down 继续轮询, 成功或失败即终止并展示。</p>
  <div class="btnrow">
    <button id="btn-poll" disabled>开始轮询</button>
    <button id="btn-stop" class="secondary" disabled>停止轮询</button>
  </div>
  <div id="poll-log" class="logbox"><div class="log-line info">[提示] 先「模拟设备发起授权」, 到前端批准后回到本页「开始轮询」即可拿到令牌。</div></div>
  <div id="final-error" class="errbox" hidden></div>
  <div id="token-result" hidden>
    <h2 class="subhead">③ 拿到令牌</h2>
    <div class="kv"><span class="k">access_token</span><span class="v mono" id="access-token"></span><button id="btn-copy-at" class="copy">复制</button></div>
    <div class="kv"><span class="k">refresh_token</span><span class="v mono" id="refresh-token"></span><button id="btn-copy-rt" class="copy">复制</button></div>
    <div class="kv"><span class="k">GET /oauth/userinfo</span><span class="v" id="userinfo"></span></div>
    <details><summary>令牌端点响应 JSON (截断)</summary><pre id="raw-token" class="mono"></pre></details>
  </div>
</section>`,
})

/* ==================================================================== */
/* 授权码测试页 /auth-code-flow                                           */
/* ==================================================================== */

const authCodeFlowScript = `
(function () {
  "use strict";
  function $(id) { return document.getElementById(id); }
  function trunc(s, n) { return s.length > n ? s.slice(0, n) + "…" : s; }
  $("btn-start").addEventListener("click", async function () {
    var btn = this;
    btn.disabled = true;
    btn.textContent = "正在生成 PKCE…";
    $("start-error").hidden = true;
    try {
      var res = await fetch("/api/auth-start", { method: "POST" });
      var data = await res.json();
      if (!res.ok || !data.authorizeUrl) {
        throw new Error((data && (data.error_description || data.error)) || ("HTTP " + res.status));
      }
      localStorage.setItem("oauth-pg:" + data.state, data.verifier);
      $("p-state").textContent = data.state;
      $("p-verifier").textContent = data.verifier;
      $("p-challenge").textContent = data.codeChallenge;
      $("p-url").href = data.authorizeUrl;
      $("p-url").textContent = data.authorizeUrl;
      $("start-result").hidden = false;
      var jump = $("jump-note");
      setTimeout(function () { window.location.href = data.authorizeUrl; }, 1200);
    } catch (e) {
      var box = $("start-error");
      box.hidden = false;
      box.textContent = "发起授权失败: " + e.message + " — 请确认本服务 :3004 与前端 :3000 均在运行";
      btn.disabled = false;
      btn.textContent = "发起授权 (跳转 authorize)";
    }
  });
})();
`

export const authCodeFlowPage = layout({
  title: "授权码测试",
  active: "auth",
  script: authCodeFlowScript,
  body: `
<h1>授权码流程测试 <span class="h1sub">Authorization Code + PKCE (S256) · client dante-web (CONFIDENTIAL)</span></h1>
<p class="sub">本页模拟第三方 Web 应用: 由本服务服务端生成 PKCE <code>code_verifier</code> (56 位随机) + <code>code_challenge = BASE64URL(SHA256(verifier))</code> + <code>state</code>, 跳转授权服务器 authorize 端点; 用户在前端 consent 页批准后 302 回调 <code>/callback</code> 换取令牌。<strong>发起前请先在 <a href="http://localhost:3000" target="_blank" rel="noopener">前端 :3000</a> 登录 (admin/admin123)</strong> — consent 页需要登录态, 未登录会先跳登录页再回来。</p>

<section class="card">
  <h2>① 生成 PKCE 并发起授权</h2>
  <p>code_verifier 存储双保险: 本服务内存 Map (state → verifier) + 页面 localStorage, 回调页自动取用。authorize 请求<strong>经前端 :3000 的 /oauth 代理</strong>发起 —— 授权服务器 302 的 Location 是相对路径 <code>/consent?request_id=…</code> (PUBLIC_APP_URL 为空), 只有在 3000 origin 下解析才能落到前端 consent 页。</p>
  <div class="btnrow"><button id="btn-start">发起授权 (跳转 authorize)</button></div>
  <div id="start-error" class="errbox" hidden></div>
  <div id="start-result" hidden>
    <div class="kv"><span class="k">state</span><span class="v mono" id="p-state"></span></div>
    <div class="kv"><span class="k">code_verifier (56 位)</span><span class="v mono" id="p-verifier"></span></div>
    <div class="kv"><span class="k">code_challenge (S256)</span><span class="v mono" id="p-challenge"></span></div>
    <div class="kv"><span class="k">authorize URL</span><span class="v mono"><a id="p-url" target="_blank" rel="noopener"></a></span></div>
    <p class="jump-note" id="jump-note">⚡ 1.2 秒后自动跳转到授权确认页… (若未跳转, 手动点击上方 authorize URL)</p>
  </div>
</section>

<section class="card">
  <h2>后续步骤 (自动进行)</h2>
  <p>② 用户在 consent 页「批准授权」→ 302 <code>http://localhost:3004/callback?code=…&amp;state=…</code> · 「拒绝授权」则回调 <code>?error=access_denied</code> (回调页有错误分支展示)<br>
  ③ 回调页自动用 code + code_verifier + Basic 认证 (dante-web:dante-web-secret) POST <code>/oauth/token</code> 换 access/refresh token<br>
  ④ 回调页调 <code>/oauth/userinfo</code> 展示用户名, 并提供「刷新令牌」按钮验证 refresh_token grant (服务端轮换, 旧 refresh 立即吊销)</p>
</section>`,
})

/* ==================================================================== */
/* 回调页 /callback                                                      */
/* ==================================================================== */

const callbackScript = `
(function () {
  "use strict";
  var BACKEND = "http://localhost:3001";
  var REDIRECT_URI = window.location.origin + "/callback";
  var BASIC = "Basic " + btoa("dante-web:dante-web-secret");
  var params = new URLSearchParams(window.location.search);
  var code = params.get("code");
  var stateParam = params.get("state") || "";
  var error = params.get("error");
  var errorDesc = params.get("error_description") || "";
  var tokens = null;

  function $(id) { return document.getElementById(id); }
  function trunc(s, n) { return s.length > n ? s.slice(0, n) + "…" : s; }
  function ts() { return new Date().toTimeString().slice(0, 8); }
  function copyText(text, btn) {
    if (!text) return;
    navigator.clipboard.writeText(text).then(function () {
      btn.textContent = "已复制 ✓";
      setTimeout(function () { btn.textContent = "复制"; }, 1400);
    });
  }
  function rlog(kind, text) {
    var box = $("refresh-log");
    var line = document.createElement("div");
    line.className = "log-line " + kind;
    line.textContent = "[" + ts() + "] " + text;
    box.appendChild(line);
    box.scrollTop = box.scrollHeight;
  }
  function renderUserinfo(el, u) {
    el.textContent = "";
    var name = document.createElement("strong");
    name.textContent = u.preferred_username || u.sub || "?";
    name.style.color = "#2dd4bf";
    el.appendChild(name);
    var rest = document.createElement("span");
    var tenant = (u.tenant && u.tenant.name) ? u.tenant.name : "-";
    var org = (u.organization && u.organization.name) ? u.organization.name : "-";
    var roles = (u.roles || []).join(", ") || "-";
    rest.textContent = " · 真实姓名: " + (u.real_name || "-") + " · 租户: " + tenant + " · 组织: " + org + " · 角色: " + roles + " · scope: " + (u.scope || "-");
    el.appendChild(rest);
  }

  $("p-code").textContent = code ? trunc(code, 24) + "…" : "-";
  $("p-state").textContent = stateParam || "-";
  $("p-error").textContent = error || "-";
  $("p-error-desc").textContent = errorDesc || "-";

  if (error) {
    var ebox0 = $("error-box");
    ebox0.hidden = false;
    ebox0.textContent = "❌ 授权失败: " + error + (errorDesc ? " — " + errorDesc : "") + " (state=" + stateParam + ")";
  } else if (!code) {
    var ebox1 = $("error-box");
    ebox1.hidden = false;
    ebox1.textContent = "回调缺少 code 与 error 参数 — 请从「授权码测试」页重新发起完整流程。";
  } else {
    $("exchange-card").hidden = false;
  }

  async function findVerifier() {
    var local = localStorage.getItem("oauth-pg:" + stateParam);
    if (local) return { verifier: local, source: "localStorage" };
    var res = await fetch("/api/verifier?state=" + encodeURIComponent(stateParam));
    if (res.ok) {
      var d = await res.json();
      return { verifier: d.verifier, source: "服务端内存 Map (localStorage 未命中)" };
    }
    return null;
  }

  function renderTokens(data) {
    $("token-result").hidden = false;
    $("p-at").textContent = trunc(data.access_token, 56) + "…";
    $("p-rt").textContent = data.refresh_token ? trunc(data.refresh_token, 24) + "…" : "(未返回)";
    $("p-meta").textContent = (data.token_type || "Bearer") + " / " + data.expires_in + "s / " + (data.scope || "-");
    $("btn-copy-at").onclick = function () { copyText(data.access_token, this); };
    $("btn-copy-rt").onclick = function () { copyText(data.refresh_token || "", this); };
    $("raw-token").textContent = JSON.stringify({
      access_token: trunc(data.access_token, 20) + "…",
      token_type: data.token_type,
      expires_in: data.expires_in,
      scope: data.scope,
      refresh_token: data.refresh_token ? trunc(data.refresh_token, 12) + "…" : null
    }, null, 2);
  }

  async function loadUserinfo(accessToken) {
    try {
      var res = await fetch(BACKEND + "/oauth/userinfo", { headers: { "Authorization": "Bearer " + accessToken } });
      if (!res.ok) {
        $("p-userinfo").textContent = "HTTP " + res.status;
        return;
      }
      var u = await res.json();
      renderUserinfo($("p-userinfo"), u);
    } catch (e) {
      $("p-userinfo").textContent = "userinfo 调用失败: " + e.message;
    }
  }

  async function exchange() {
    var statusEl = $("exchange-status");
    $("error-box").hidden = true;
    statusEl.textContent = "正在查找 code_verifier…";
    try {
      var v = await findVerifier();
      if (!v) {
        statusEl.textContent = "";
        var box = $("error-box");
        box.hidden = false;
        box.textContent = "找不到 code_verifier: state=" + stateParam + " 未命中 localStorage 与服务端 Map (服务可能重启过)。请回到「授权码测试」页重新发起。";
        return;
      }
      statusEl.textContent = "code_verifier 来自 " + v.source + " · 正在 POST /oauth/token…";
      var body = new URLSearchParams();
      body.set("grant_type", "authorization_code");
      body.set("code", code);
      body.set("redirect_uri", REDIRECT_URI);
      body.set("code_verifier", v.verifier);
      var res = await fetch(BACKEND + "/oauth/token", {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded", "Authorization": BASIC },
        body: body.toString()
      });
      var data = await res.json();
      statusEl.textContent = "";
      if (!res.ok || !data.access_token) {
        var ebox = $("error-box");
        ebox.hidden = false;
        ebox.textContent = "❌ 交换令牌失败: " + (data.error || ("HTTP " + res.status)) + (data.error_description ? " — " + data.error_description : "");
        return;
      }
      tokens = data;
      renderTokens(data);
      loadUserinfo(data.access_token);
    } catch (e) {
      statusEl.textContent = "";
      var eb = $("error-box");
      eb.hidden = false;
      eb.textContent = "交换令牌网络错误: " + e.message;
    }
  }

  $("btn-exchange").addEventListener("click", exchange);
  $("btn-refresh").addEventListener("click", async function () {
    if (!tokens || !tokens.refresh_token) return;
    var btn = this;
    btn.disabled = true;
    try {
      var body = new URLSearchParams();
      body.set("grant_type", "refresh_token");
      body.set("refresh_token", tokens.refresh_token);
      // 注意: 授权服务器 refresh 端点在省略 scope 时会按客户端注册 scope 解析,
      // 原令牌 scope 收窄 (profile ⊂ profile+email) 时会报 invalid_scope —
      // 显式带上原令牌 scope (RFC 6749 允许收窄) 即可稳定刷新。
      if (tokens.scope) body.set("scope", tokens.scope);
      var res = await fetch(BACKEND + "/oauth/token", {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded", "Authorization": BASIC },
        body: body.toString()
      });
      var data = await res.json();
      if (res.ok && data.access_token) {
        rlog("ok", "✅ 刷新成功: 新 access_token=" + trunc(data.access_token, 20) + "… (旧 refresh_token 已被服务端轮换吊销)");
        tokens = data;
        renderTokens(data);
        loadUserinfo(data.access_token);
      } else {
        rlog("err", "刷新失败: " + (data.error || ("HTTP " + res.status)) + (data.error_description ? " — " + data.error_description : ""));
      }
    } catch (e) {
      rlog("err", "刷新网络错误: " + e.message);
    } finally {
      btn.disabled = false;
    }
  });

  if (code && !error) exchange();
})();
`

export const callbackPage = layout({
  title: "授权码回调",
  active: "auth",
  script: callbackScript,
  body: `
<h1>授权码回调 <span class="h1sub">/callback · redirect_uri=http://localhost:3004/callback</span></h1>
<p class="sub">接收授权服务器 302 回调 (<code>?code&amp;state</code> 或 <code>?error&amp;state</code>), 展示授权码并自动向令牌端点交换 (Basic 认证 dante-web + PKCE code_verifier)。授权码 60 秒有效且一次性。</p>

<section class="card">
  <h2>① 回调参数</h2>
  <div class="kv"><span class="k">code</span><span class="v mono" id="p-code">-</span></div>
  <div class="kv"><span class="k">state</span><span class="v mono" id="p-state">-</span></div>
  <div class="kv"><span class="k">error</span><span class="v mono" id="p-error">-</span></div>
  <div class="kv"><span class="k">error_description</span><span class="v" id="p-error-desc">-</span></div>
</section>

<div id="error-box" class="errbox" hidden></div>

<section class="card" id="exchange-card" hidden>
  <h2>② 交换令牌 (authorization_code + PKCE)</h2>
  <p>POST <code>http://localhost:3001/oauth/token</code> · Authorization: Basic <code>dante-web:dante-web-secret</code> · form: <code>grant_type=authorization_code&amp;code&amp;redirect_uri&amp;code_verifier</code> — 浏览器直连授权服务器。</p>
  <div class="btnrow">
    <button id="btn-exchange" class="secondary">重新交换令牌 (演示: 授权码一次性, 二次消费应报 invalid_grant)</button>
    <span id="exchange-status" class="status"></span>
  </div>
  <div id="token-result" hidden>
    <div class="kv"><span class="k">access_token</span><span class="v mono" id="p-at"></span><button id="btn-copy-at" class="copy">复制</button></div>
    <div class="kv"><span class="k">refresh_token</span><span class="v mono" id="p-rt"></span><button id="btn-copy-rt" class="copy">复制</button></div>
    <div class="kv"><span class="k">token_type / expires_in / scope</span><span class="v mono" id="p-meta"></span></div>
    <div class="kv"><span class="k">GET /oauth/userinfo</span><span class="v" id="p-userinfo"></span></div>
    <details><summary>令牌端点响应 JSON (截断)</summary><pre id="raw-token" class="mono"></pre></details>
    <h2 class="subhead">③ 刷新令牌 (refresh_token grant · 服务端轮换)</h2>
    <p>POST <code>/oauth/token</code> · form: <code>grant_type=refresh_token&amp;refresh_token=…</code> + Basic 认证。dante-web 启用了 refresh_token 模式, 服务端会轮换: 旧 refresh_token 立即吊销, 返回新的 access + refresh。</p>
    <div class="btnrow"><button id="btn-refresh">刷新令牌</button></div>
    <div id="refresh-log" class="logbox"></div>
  </div>
</section>`,
})

/* ==================================================================== */
/* 客户端凭据测试页 /client-credentials (Task 20)                          */
/* ==================================================================== */

const ccFlowScript = `
(function () {
  "use strict";
  var st = { token: null, expiresAt: 0, revoked: false };

  function $(id) { return document.getElementById(id); }
  function ts() { return new Date().toTimeString().slice(0, 8); }
  function trunc(s, n) { return s.length > n ? s.slice(0, n) + "…" : s; }
  function log(kind, text) {
    var box = $("cc-log");
    var line = document.createElement("div");
    line.className = "log-line " + kind;
    line.textContent = "[" + ts() + "] " + text;
    box.appendChild(line);
    box.scrollTop = box.scrollHeight;
  }
  function copyText(text, btn) {
    if (!text) return;
    navigator.clipboard.writeText(text).then(function () {
      btn.textContent = "已复制 ✓";
      setTimeout(function () { btn.textContent = "复制"; }, 1400);
    });
  }
  function setButtons() {
    var has = !!st.token;
    $("btn-introspect").disabled = !has;
    $("btn-revoke").disabled = !has || st.revoked;
    $("btn-userinfo").disabled = !has;
    $("btn-copy-at").disabled = !has;
  }
  async function post(path, body) {
    var res = await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body || {})
    });
    var text = await res.text();
    var json = null;
    try { json = JSON.parse(text); } catch (e) { /* 非 JSON 透传原文 */ }
    return { status: res.status, json: json, text: text };
  }
  async function ccToken() {
    $("btn-token").disabled = true;
    $("token-result").hidden = true;
    $("errbox").hidden = true;
    log("info", "POST /api/cc-token → (服务端代理, api-service Basic 认证) POST /oauth/token grant_type=client_credentials");
    var r = await post("/api/cc-token");
    if (r.status === 200 && r.json && r.json.access_token) {
      st.token = r.json.access_token;
      st.revoked = false;
      st.expiresAt = Date.now() + (r.json.expires_in || 7200) * 1000;
      $("p-at").textContent = trunc(st.token, 72);
      $("p-meta").textContent = "token_type=" + (r.json.token_type || "-") + " · expires_in=" + (r.json.expires_in || "-") + "s · scope=" + (r.json.scope || "-");
      var hasRefresh = !!r.json.refresh_token;
      $("p-refresh-note").textContent = hasRefresh
        ? "⚠ 响应含 refresh_token (观察项)"
        : "✓ 响应不含 refresh_token — 符合 RFC 6749 §4.4.3: 客户端凭据模式不应签发刷新令牌";
      $("raw-token").textContent = JSON.stringify(r.json, null, 2);
      $("token-result").hidden = false;
      log("ok", "200 OK · access_token 已签发 (" + trunc(st.token, 24) + "…) · 无用户上下文(纯客户端身份)");
    } else {
      $("errbox").hidden = false;
      $("errbox").textContent = "HTTP " + r.status + " · " + (r.text || "").slice(0, 400);
      log("err", "HTTP " + r.status + " " + (r.text || "").slice(0, 160));
      st.token = null;
    }
    setButtons();
    $("btn-token").disabled = false;
  }
  async function doIntrospect(note) {
    if (!st.token) return null;
    log("info", "POST /api/introspect → (服务端代理, api-service Basic 认证) POST /oauth/introspect token=…");
    var r = await post("/api/introspect", { token: st.token });
    var box = $("introspect-result");
    if (r.json) {
      var active = r.json.active === true;
      var kv = "active=" + r.json.active + " · client_id=" + (r.json.client_id || "-") + " · scope=" + (r.json.scope || "-") + " · username=" + (r.json.username || "(空·无用户)") + " · exp=" + (r.json.exp || "-");
      box.textContent = kv;
      box.style.color = active ? "#5eead4" : "#fda4a4";
      $("raw-introspect").textContent = JSON.stringify(r.json, null, 2);
      log(active ? "ok" : "err", "200 OK · " + kv);
    } else {
      log("err", "HTTP " + r.status + " " + (r.text || "").slice(0, 160));
    }
    if (note) log("info", note);
    return r.json;
  }
  async function doRevoke() {
    if (!st.token) return;
    $("btn-revoke").disabled = true;
    log("info", "POST /api/revoke → (服务端代理, api-service Basic 认证) POST /oauth/revoke token=… (RFC 7009, 无论结果始终 200)");
    var r = await post("/api/revoke", { token: st.token });
    log("ok", "HTTP " + r.status + " · 吊销请求已受理 (RFC 7009 始终 200)");
    st.revoked = true;
    await doIntrospect("★ 吊销后内省: active 应为 false — 吊销闭环完成");
    setButtons();
  }
  async function tryUserinfo() {
    if (!st.token) return;
    log("info", "POST /api/userinfo → (服务端代理, Bearer) GET /oauth/userinfo — 期望 401 invalid_token (无用户上下文; 若令牌已吊销则先撞 401 已吊销)");
    var r = await post("/api/userinfo", { token: st.token });
    var box = $("userinfo-result");
    if (r.status === 403 || (r.json && r.json.error)) {
      box.textContent = "HTTP " + r.status + " · " + (r.json ? r.json.error + ": " + (r.json.error_description || r.json.message || "") : r.text.slice(0, 200));
      box.style.color = "#ffd28a";
      log("pending", "HTTP " + r.status + " · 预期行为: client_credentials 令牌无 user_id, userinfo 拒绝 (教学点: 服务间调用应改用 /oauth/introspect)");
    } else {
      box.textContent = "HTTP " + r.status + " · " + (r.text || "").slice(0, 200);
      box.style.color = "#5eead4";
      log("ok", "HTTP " + r.status + " · 意外成功 (令牌竟含用户上下文?)");
    }
  }
  $("btn-token").addEventListener("click", ccToken);
  $("btn-introspect").addEventListener("click", function () { doIntrospect(); });
  $("btn-revoke").addEventListener("click", doRevoke);
  $("btn-userinfo").addEventListener("click", tryUserinfo);
  $("btn-copy-at").addEventListener("click", function () { copyText(st.token, this); });
  setButtons();
})();
`

export const clientCredentialsPage = layout({
  title: "客户端凭据",
  active: "cc",
  script: ccFlowScript,
  body: `
<h1>客户端凭据测试 <span class="h1sub">client_credentials · RFC 6749 §4.4 · 机器对机器</span></h1>
<p class="sub">演示<b>服务身份令牌</b>: 客户端自己就是资源所有者 (无用户参与)。api-service 代表后端服务直接向令牌端点认证并取令牌 —— 典型的定时任务/微服务间调用场景。所有 upstream 请求由本服务服务端代理发出 (client_secret 只存服务端内存, 浏览器不可见, 与真实部署一致)。</p>

<section class="card">
  <h2>演示客户端</h2>
  <div class="kv"><span class="k">client_id</span><span class="v mono">api-service</span></div>
  <div class="kv"><span class="k">类型 / 认证通道</span><span class="v mono">CONFIDENTIAL · HTTP Basic (RFC 6749 §2.3.1): Basic base64(api-service:api-service-secret)</span></div>
  <div class="kv"><span class="k">已注册授权模式</span><span class="v mono">client_credentials (仅此一种)</span></div>
  <div class="kv"><span class="k">已注册 scope</span><span class="v mono">profile · 令牌有效期 7200s</span></div>
</section>

<section class="card">
  <h2>① 获取客户端凭据令牌</h2>
  <p>等价 curl: <code>curl -u api-service:api-service-secret -d "grant_type=client_credentials&scope=profile" http://localhost:3001/oauth/token</code></p>
  <div class="btnrow">
    <button id="btn-token">获取访问令牌</button>
    <span class="status">→ POST /api/cc-token (本服务代理 → /oauth/token)</span>
  </div>
  <div id="errbox" class="errbox" hidden></div>
  <div id="token-result" hidden>
    <div class="kv"><span class="k">access_token</span><span class="v mono" id="p-at">-</span><button id="btn-copy-at" class="copy">复制</button></div>
    <div class="kv"><span class="k">token_type / expires_in / scope</span><span class="v mono" id="p-meta">-</span></div>
    <div class="kv"><span class="k">refresh_token 观察</span><span class="v" id="p-refresh-note">-</span></div>
    <details><summary>令牌端点响应 JSON</summary><pre id="raw-token" class="mono"></pre></details>
  </div>
</section>

<section class="card" id="introspect-card">
  <h2>② 服务间验证 · 令牌内省 (RFC 7662)</h2>
  <p>api-service 作为受保护资源, 用自己的 Basic 凭据向授权服务器验证令牌活性。等价 curl: <code>curl -u api-service:api-service-secret -d "token=…" http://localhost:3001/oauth/introspect</code></p>
  <div class="btnrow"><button id="btn-introspect" class="secondary">内省当前令牌</button></div>
  <div class="kv"><span class="k">内省结果</span><span class="v mono" id="introspect-result">-</span></div>
  <details><summary>内省响应 JSON</summary><pre id="raw-introspect" class="mono"></pre></details>
</section>

<section class="card">
  <h2>③ 吊销并复验 (RFC 7009 闭环)</h2>
  <p>吊销当前令牌后自动再内省, <code>active:false</code> 即闭环成立。等价 curl: <code>curl -u api-service:api-service-secret -d "token=…" http://localhost:3001/oauth/revoke</code></p>
  <div class="btnrow"><button id="btn-revoke" class="secondary">吊销当前令牌 + 自动复验</button></div>
</section>

<section class="card" id="userinfo-card">
  <h2>④ 教学点 · 此令牌调 userinfo 会怎样?</h2>
  <p>userinfo 需要<b>用户上下文</b> (user_id), 而 client_credentials 令牌没有用户。观察 401 invalid_token「该令牌没有关联用户」的精确拒绝 —— 服务间调用应改用 ② 的 introspect 端点。</p>
  <div class="btnrow"><button id="btn-userinfo" class="secondary">用此令牌调 /oauth/userinfo</button></div>
  <div class="kv"><span class="k">userinfo 响应</span><span class="v mono" id="userinfo-result">-</span></div>
</section>

<section class="card">
  <h2>流程日志</h2>
  <div id="cc-log" class="logbox"></div>
</section>`,
})

/* ==================================================================== */
/* 资源密码测试页 /password-flow (Task 20)                                 */
/* ==================================================================== */

const passwordFlowScript = `
(function () {
  "use strict";
  var st = { at: null, rt: null, oldRt: null, expiresAt: 0, rtRevoked: false };

  function $(id) { return document.getElementById(id); }
  function ts() { return new Date().toTimeString().slice(0, 8); }
  function trunc(s, n) { return s.length > n ? s.slice(0, n) + "…" : s; }
  function log(kind, text) {
    var box = $("pwd-log");
    var line = document.createElement("div");
    line.className = "log-line " + kind;
    line.textContent = "[" + ts() + "] " + text;
    box.appendChild(line);
    box.scrollTop = box.scrollHeight;
  }
  function copyText(text, btn) {
    if (!text) return;
    navigator.clipboard.writeText(text).then(function () {
      btn.textContent = "已复制 ✓";
      setTimeout(function () { btn.textContent = "复制"; }, 1400);
    });
  }
  function setButtons() {
    var has = !!st.at;
    $("btn-userinfo").disabled = !has;
    $("btn-refresh").disabled = !has || !st.rt;
    $("btn-reuse-rt").disabled = !st.oldRt || st.rtRevoked;
    $("btn-copy-at2").disabled = !has;
    $("btn-copy-rt2").disabled = !has;
  }
  function readScope() {
    var scopes = [];
    if ($("sc-profile").checked) scopes.push("profile");
    if ($("sc-email").checked) scopes.push("email");
    return scopes.join(" ");
  }
  async function post(path, body) {
    var res = await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body || {})
    });
    var text = await res.text();
    var json = null;
    try { json = JSON.parse(text); } catch (e) { /* 非 JSON */ }
    return { status: res.status, json: json, text: text };
  }
  async function getToken(badPassword) {
    $("btn-get").disabled = true;
    $("token-result").hidden = true;
    $("errbox").hidden = true;
    var username = $("in-username").value.trim();
    var password = $("in-password").value;
    if (badPassword) password = password + "-wrong";
    var scope = readScope();
    log("info", "POST /api/password-token → (服务端代理, resource-portal form 认证) POST /oauth/token grant_type=password scope=" + (scope || "(默认)"));
    var r = await post("/api/password-token", { username: username, password: password, scope: scope });
    if (r.status === 200 && r.json && r.json.access_token) {
      st.at = r.json.access_token;
      st.rt = r.json.refresh_token || null;
      st.oldRt = null;
      st.rtRevoked = false;
      st.expiresAt = Date.now() + (r.json.expires_in || 7200) * 1000;
      $("p-at2").textContent = trunc(st.at, 60);
      $("p-rt2").textContent = st.rt ? trunc(st.rt, 60) : "(未签发)";
      $("p-meta2").textContent = "token_type=" + (r.json.token_type || "-") + " · expires_in=" + (r.json.expires_in || "-") + "s · scope=" + (r.json.scope || "-");
      $("raw-token2").textContent = JSON.stringify(r.json, null, 2);
      $("token-result").hidden = false;
      log("ok", "200 OK · 令牌已签发 · 上下文取自该用户第一个组织成员关系 (后端语义)");
      if (badPassword) log("err", "!! 错误密码竟然成功? — 请报告这个 bug");
    } else {
      $("errbox").hidden = false;
      $("errbox").textContent = "HTTP " + r.status + " · " + (r.json ? (r.json.error + ": " + (r.json.error_description || r.json.message || "")) : (r.text || "").slice(0, 300));
      log(badPassword ? "pending" : "err", "HTTP " + r.status + " · " + (r.json ? r.json.error + " — " + (r.json.error_description || r.json.message || "") : (r.text || "").slice(0, 140)));
      if (badPassword) log("ok", "✓ 错误凭据被正确拒绝 (invalid_grant)");
      st.at = null; st.rt = null;
    }
    setButtons();
    $("btn-get").disabled = false;
  }
  async function callUserinfo() {
    if (!st.at) return;
    log("info", "POST /api/userinfo → (服务端代理, Bearer) GET /oauth/userinfo");
    var r = await post("/api/userinfo", { token: st.at });
    if (r.status === 200 && r.json) {
      var u = r.json;
      var tenant = (u.tenant && u.tenant.name) ? u.tenant.name : "-";
      var org = (u.organization && u.organization.name) ? u.organization.name : "-";
      var roles = (u.roles || []).join(", ") || "-";
      $("ui-main").textContent = u.preferred_username || u.sub || "?";
      $("ui-main").style.color = "#2dd4bf";
      $("ui-detail").textContent = " · 真实姓名: " + (u.real_name || "-") + " · 租户: " + tenant + " · 组织: " + org + " · 角色: " + roles + " · scope: " + (u.scope || "-");
      log("ok", "200 OK · 用户: " + (u.preferred_username || "-") + " · 租户: " + tenant + " · 组织: " + org);
    } else {
      $("ui-main").textContent = "HTTP " + r.status;
      $("ui-main").style.color = "#fda4a4";
      $("ui-detail").textContent = (r.text || "").slice(0, 220);
      log("err", "HTTP " + r.status + " " + (r.text || "").slice(0, 140));
    }
  }
  async function doRefresh() {
    if (!st.rt) return;
    $("btn-refresh").disabled = true;
    var oldRt = st.rt;
    log("info", "POST /api/refresh-token → (服务端代理, resource-portal form 认证) POST /oauth/token grant_type=refresh_token");
    var r = await post("/api/refresh-token", { refreshToken: oldRt });
    if (r.status === 200 && r.json && r.json.access_token) {
      var rotated = r.json.refresh_token && r.json.refresh_token !== oldRt;
      st.at = r.json.access_token;
      if (r.json.refresh_token) st.rt = r.json.refresh_token;
      if (rotated) st.oldRt = oldRt; // 保存轮换前的旧 RT, 供「复用旧令牌」验证
      $("p-at2").textContent = trunc(st.at, 60);
      $("p-rt2").textContent = st.rt ? trunc(st.rt, 60) : "(未签发)";
      $("raw-token2").textContent = JSON.stringify(r.json, null, 2);
      $("rot-note").textContent = rotated
        ? "✓ refresh_token 已轮换: 服务端签发了新 refresh_token, 旧的已吊销 (可点下方按钮复用旧令牌验证)"
        : "refresh_token 未轮换 (沿用原值)";
      $("rot-note").style.color = rotated ? "#5eead4" : "#8aa0a8";
      log("ok", "200 OK · access_token 已刷新" + (rotated ? " · refresh_token 已轮换" : " · refresh_token 沿用"));
    } else {
      log("err", "HTTP " + r.status + " · " + (r.text || "").slice(0, 160));
    }
    setButtons();
  }
  async function reuseOldRt() {
    if (!st.oldRt || st.rtRevoked) return;
    $("btn-reuse-rt").disabled = true;
    var stale = st.oldRt;
    log("info", "演示: 用已轮换的旧 refresh_token 再次刷新 (应 400 invalid_grant)");
    var r = await post("/api/refresh-token", { refreshToken: stale });
    if (r.status !== 200) {
      st.rtRevoked = true;
      log("ok", "✓ HTTP " + r.status + " · " + (r.json ? r.json.error + " — " + (r.json.error_description || r.json.message || "") : (r.text || "").slice(0, 140)));
      log("ok", "旧 refresh_token 已被服务端吊销, 复用被正确拒绝 (安全闭环)");
    } else {
      log("err", "!! 旧 refresh_token 仍可用? — 请报告这个 bug");
    }
    setButtons();
  }
  $("btn-get").addEventListener("click", function () { getToken(false); });
  $("btn-bad").addEventListener("click", function () { getToken(true); });
  $("btn-userinfo").addEventListener("click", callUserinfo);
  $("btn-refresh").addEventListener("click", doRefresh);
  $("btn-reuse-rt").addEventListener("click", reuseOldRt);
  $("btn-copy-at2").addEventListener("click", function () { copyText(st.at, this); });
  $("btn-copy-rt2").addEventListener("click", function () { copyText(st.rt, this); });
  setButtons();
})();
`

export const passwordFlowPage = layout({
  title: "资源密码",
  active: "pwd",
  script: passwordFlowScript,
  body: `
<h1>资源密码测试 <span class="h1sub">password grant · RFC 6749 §4.3 · 仅限高信任第一方应用</span></h1>
<p class="sub">演示<b>资源密码模式</b>: 应用直接采集用户名/密码换取令牌。仅适用于操作系统级高度信任的第一方应用 (原生 App/遗留系统), 第三方应用严禁使用 (用户凭据暴露给应用方)。resource-portal 是后端渲染的传统 Web 应用 —— client_secret 与用户密码均由本服务服务端代理转发, 不落浏览器。</p>

<section class="card">
  <h2>演示客户端</h2>
  <div class="kv"><span class="k">client_id</span><span class="v mono">resource-portal</span></div>
  <div class="kv"><span class="k">类型 / 认证通道</span><span class="v mono">CONFIDENTIAL · form body 通道: client_id + client_secret (与客户端凭据页的 Basic 通道形成对照)</span></div>
  <div class="kv"><span class="k">已注册授权模式</span><span class="v mono">authorization_code, password, refresh_token</span></div>
  <div class="kv"><span class="k">已注册 scope</span><span class="v mono">profile, email · 令牌有效期 7200s</span></div>
</section>

<section class="card">
  <h2>① 用户凭据换令牌</h2>
  <p>等价 curl: <code>curl -d "grant_type=password&username=admin&password=…&scope=profile email&client_id=resource-portal&client_secret=…" http://localhost:3001/oauth/token</code></p>
  <div class="kv"><span class="k">用户名</span><span class="v"><input id="in-username" value="admin" autocomplete="off" spellcheck="false" style="background:#122026;border:1px solid #24343b;color:#c9dadd;border-radius:6px;padding:6px 10px;font-size:13px;width:220px;"></span></div>
  <div class="kv"><span class="k">密码</span><span class="v"><input id="in-password" type="password" value="admin123" autocomplete="off" style="background:#122026;border:1px solid #24343b;color:#c9dadd;border-radius:6px;padding:6px 10px;font-size:13px;width:220px;"></span></div>
  <div class="kv"><span class="k">scope</span><span class="v"><label style="margin-right:14px;cursor:pointer"><input type="checkbox" id="sc-profile" checked> profile</label><label style="cursor:pointer"><input type="checkbox" id="sc-email" checked> email</label></span></div>
  <div class="btnrow">
    <button id="btn-get">获取访问令牌</button>
    <button id="btn-bad" class="secondary">错误凭据演示 (密码尾部拼 -wrong)</button>
  </div>
  <p class="client">提示: 也可换 <code>wangwu/Ww@12345</code> (双组织) 或 <code>chenjiu/Cj@12345</code> (星辰制造·非平台租户), 观察 userinfo 返回的不同租户/组织上下文。</p>
  <div id="errbox" class="errbox" hidden></div>
  <div id="token-result" hidden>
    <div class="kv"><span class="k">access_token</span><span class="v mono" id="p-at2">-</span><button id="btn-copy-at2" class="copy">复制</button></div>
    <div class="kv"><span class="k">refresh_token</span><span class="v mono" id="p-rt2">-</span><button id="btn-copy-rt2" class="copy">复制</button></div>
    <div class="kv"><span class="k">token_type / expires_in / scope</span><span class="v mono" id="p-meta2">-</span></div>
    <details><summary>令牌端点响应 JSON</summary><pre id="raw-token2" class="mono"></pre></details>
  </div>
</section>

<section class="card" id="ui-card">
  <h2>② 调用 /oauth/userinfo (Bearer)</h2>
  <div class="btnrow"><button id="btn-userinfo" class="secondary">查询当前令牌用户信息</button></div>
  <p><strong id="ui-main">-</strong><span id="ui-detail">-</span></p>
</section>

<section class="card">
  <h2>③ 刷新令牌与轮换验证</h2>
  <p>resource-portal 启用 refresh_token 模式: 刷新时服务端轮换 refresh_token (旧值立即吊销)。</p>
  <div class="btnrow">
    <button id="btn-refresh" class="secondary">刷新令牌 (refresh_token grant)</button>
    <button id="btn-reuse-rt" class="secondary">复用旧 refresh_token (应 invalid_grant)</button>
  </div>
  <div class="kv"><span class="k">轮换观察</span><span class="v" id="rot-note">先刷新一次再观察</span></div>
</section>

<section class="card">
  <h2>流程日志</h2>
  <div id="pwd-log" class="logbox"></div>
</section>`,
})

/* ==================================================================== */
/* 第三方应用对接演示页 /external-api (Task 21-e)                          */
/* ==================================================================== */

const externalApiScript = `
(function () {
  "use strict";
  var st = { at: null };

  function $(id) { return document.getElementById(id); }
  function ts() { return new Date().toTimeString().slice(0, 8); }
  function trunc(s, n) { return s.length > n ? s.slice(0, n) + "…" : s; }
  function log(kind, text) {
    var box = $("ext-log");
    var line = document.createElement("div");
    line.className = "log-line " + kind;
    line.textContent = "[" + ts() + "] " + text;
    box.appendChild(line);
    box.scrollTop = box.scrollHeight;
  }
  function copyText(text, btn) {
    if (!text) return;
    navigator.clipboard.writeText(text).then(function () {
      btn.textContent = "已复制 ✓";
      setTimeout(function () { btn.textContent = "复制"; }, 1400);
    });
  }
  function setButtons() {
    var has = !!st.at;
    $("btn-menus").disabled = !has;
    $("btn-roles").disabled = !has;
    $("btn-copy-at3").disabled = !has;
    $("btn-noauth").disabled = false;
  }
  async function post(path, body) {
    var res = await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body || {})
    });
    var text = await res.text();
    var json = null;
    try { json = JSON.parse(text); } catch (e) { /* 非 JSON */ }
    return { status: res.status, json: json, text: text };
  }

  /* ① client_credentials 令牌 (与客户端凭据页同一演示客户端 api-service) */
  async function getToken() {
    $("btn-get3").disabled = true;
    $("token-result3").hidden = true;
    $("errbox3").hidden = true;
    log("info", "POST /api/cc-token → (服务端代理, Basic 认证) POST /oauth/token grant_type=client_credentials");
    var r = await post("/api/cc-token", {});
    if (r.status === 200 && r.json && r.json.access_token) {
      st.at = r.json.access_token;
      $("p-at3").textContent = trunc(st.at, 60);
      $("p-meta3").textContent = "token_type=" + (r.json.token_type || "-") + " · expires_in=" + (r.json.expires_in || "-") + "s · scope=" + (r.json.scope || "-");
      $("raw-token3").textContent = JSON.stringify(r.json, null, 2);
      $("token-result3").hidden = false;
      log("ok", "200 OK · 服务身份令牌已签发 (sub=api-service, 无用户上下文)");
    } else {
      $("errbox3").hidden = false;
      $("errbox3").textContent = "HTTP " + r.status + " · " + (r.text || "").slice(0, 300);
      log("err", "HTTP " + r.status + " · " + (r.text || "").slice(0, 160));
      st.at = null;
    }
    setButtons();
    $("btn-get3").disabled = false;
  }

  /* 菜单树渲染: 递归缩进列表, DIR/MENU/BUTTON 三种类型徽章 */
  function menuLine(m, depth) {
    var pad = "";
    for (var i = 0; i < depth; i++) pad += "    ";
    var typeTag = m.type === "DIR" ? "[DIR]" : (m.type === "BUTTON" ? "[BTN]" : "[MENU]");
    var bits = [pad + typeTag + " " + (m.title || "?")];
    if (m.path) bits.push("path=" + m.path);
    if (m.permission) bits.push("perm=" + m.permission);
    bits.push("sort=" + (m.sortOrder != null ? m.sortOrder : "-"));
    if (m.visible === false) bits.push("隐藏");
    return bits.join(" · ");
  }
  function walkMenu(m, depth, out) {
    out.push(menuLine(m, depth));
    (m.children || []).forEach(function (c) { walkMenu(c, depth + 1, out); });
  }

  /* ② GET /api/external/menus */
  async function fetchMenus() {
    if (!st.at) return;
    log("info", "POST /api/external-menus → (服务端代理, Bearer) GET /api/external/menus");
    var r = await post("/api/external-menus", { token: st.at });
    if (r.status === 200 && r.json) {
      var app = r.json.application || {};
      var client = r.json.client || {};
      $("menus-app").textContent = app.code + " · " + (app.name || "-");
      $("menus-client").textContent = "client_id=" + (client.clientId || "-") + " · tokenType=" + (client.tokenType || "-") + " · username=" + (client.username || "(null · 机器身份)");
      var out = [];
      (r.json.menus || []).forEach(function (m) { walkMenu(m, 0, out); });
      $("menus-tree").textContent = out.length ? out.join("\\n") : "(该应用暂无菜单)";
      $("raw-menus").textContent = JSON.stringify(r.json, null, 2);
      $("menus-result").hidden = false;
      var n = out.length;
      log("ok", "200 OK · 应用「" + (app.name || app.code) + "」菜单树 " + n + " 项 (含子节点)");
    } else {
      $("menus-err").hidden = false;
      $("menus-err").textContent = "HTTP " + r.status + " · " + (r.json ? (r.json.message || r.json.error || r.text.slice(0, 200)) : (r.text || "").slice(0, 300));
      log("err", "HTTP " + r.status + " · " + ((r.json && (r.json.message || r.json.error)) || (r.text || "").slice(0, 160)));
    }
  }

  /* ③ GET /api/external/roles: 组织分组 + 角色行 */
  async function fetchRoles() {
    if (!st.at) return;
    log("info", "POST /api/external-roles → (服务端代理, Bearer) GET /api/external/roles");
    var r = await post("/api/external-roles", { token: st.at });
    if (r.status === 200 && r.json) {
      var app = r.json.application || {};
      $("roles-app").textContent = app.code + " · " + (app.name || "-");
      var orgs = r.json.organizations || [];
      var html = "";
      if (!orgs.length) {
        html = "<p class='client'>该应用尚未授权给任何组织 (管理端「授权管理」页可把应用授权给租户/组织)</p>";
      }
      orgs.forEach(function (o) {
        html += "<div style='margin-bottom:12px'><strong style='color:#2dd4bf'>" + (o.name || "?") + "</strong>";
        html += " <span class='client'>租户: " + (o.tenantName || "-") + "</span>";
        var roles = o.roles || [];
        if (!roles.length) {
          html += "<p class='client' style='margin:4px 0 0 4px'>· 该组织暂无角色</p>";
        } else {
          html += "<ul class='steps' style='margin-top:4px'>";
          roles.forEach(function (ro) {
            html += "<li>" + (ro.name || "?") + (ro.code ? " <span class='mono'>(" + ro.code + ")</span>" : "");
            html += (ro.builtIn ? " · <span style='color:#ffd28a'>内置</span>" : "");
            html += " · 本应用菜单授权 " + (ro.menuCount != null ? ro.menuCount : "0") + " 项</li>";
          });
          html += "</ul>";
        }
        html += "</div>";
      });
      $("roles-list").innerHTML = html;
      $("raw-roles").textContent = JSON.stringify(r.json, null, 2);
      $("roles-result").hidden = false;
      log("ok", "200 OK · " + orgs.length + " 个授权组织 · 共 " + orgs.reduce(function (acc, o) { return acc + (o.roles || []).length; }, 0) + " 个角色");
    } else {
      $("roles-err").hidden = false;
      $("roles-err").textContent = "HTTP " + r.status + " · " + (r.json ? (r.json.message || r.json.error || r.text.slice(0, 200)) : (r.text || "").slice(0, 300));
      log("err", "HTTP " + r.status + " · " + ((r.json && (r.json.message || r.json.error)) || (r.text || "").slice(0, 160)));
    }
  }

  /* ④ 教学点: 不带令牌调对外 API → 授权服务器 401 (经本服务代理为 400 缺 token) */
  async function callWithoutToken() {
    log("info", "POST /api/external-menus (不带 token) → 观察 400 invalid_request (缺少 token)");
    var r = await post("/api/external-menus", {});
    log("err", "HTTP " + r.status + " · " + ((r.json && r.json.error_description) || (r.text || "").slice(0, 160)));
    $("noauth-result").hidden = false;
    $("noauth-result").textContent = "HTTP " + r.status + " · " + ((r.json && (r.json.error + ": " + r.json.error_description)) || (r.text || "").slice(0, 200));
    log("ok", "✓ 对外 API 必须携带 Bearer 令牌 — 无凭证一律拒绝 (上游授权服务器对缺失 Authorization 头返回 401)");
  }

  $("btn-get3").addEventListener("click", getToken);
  $("btn-menus").addEventListener("click", fetchMenus);
  $("btn-roles").addEventListener("click", fetchRoles);
  $("btn-noauth").addEventListener("click", callWithoutToken);
  $("btn-copy-at3").onclick = function () { copyText(st.at, this); };
  setButtons();
})();`

export const externalApiPage = layout({
  title: "第三方对接",
  active: "ext",
  script: externalApiScript,
  body: `
<h1>第三方应用对接 <span class="h1sub">菜单 / 角色对外 API · client 身份拉取平台权限模型</span></h1>
<p class="sub">演示<b>第三方应用如何使用平台的菜单与角色数据</b>: OAuth 客户端在管理端关联一个平台应用后, 即可以服务身份 (client_credentials) 拉取该应用的菜单树与角色授权 —— 典型场景: 子应用渲染自己的导航、同步权限模型做细粒度鉴权。请求链路与真实部署一致: secret 只存服务端, 页面经本服务代理转发。</p>

<section class="card">
  <h2>演示客户端与关联关系</h2>
  <div class="kv"><span class="k">client_id</span><span class="v mono">api-service (CONFIDENTIAL · Basic 认证 · 仅 client_credentials)</span></div>
  <div class="kv"><span class="k">关联应用</span><span class="v mono">athena · 数据分析 (code=athena) — 种子预置的演示关联</span></div>
  <div class="kv"><span class="k">关联配置入口</span><span class="v">管理端 → 系统管理 → OAuth 客户端 → 编辑客户端 → 「关联应用」下拉 (Task 21 新增)</span></div>
  <div class="kv"><span class="k">对外端点</span><span class="v mono">GET /api/external/menus · GET /api/external/roles (Bearer access token)</span></div>
  <p class="client">注意: 关联是「客户端 → 应用」的声明, 菜单/角色数据本身仍按平台的租户/组织授权模型过滤 (external/roles 只返回授权了该应用的组织)。</p>
</section>

<section class="card">
  <h2>① 获取服务身份令牌 (client_credentials)</h2>
  <p>等价 curl: <code>curl -u api-service:api-service-secret -d "grant_type=client_credentials&scope=profile" http://localhost:3001/oauth/token</code></p>
  <div class="btnrow">
    <button id="btn-get3">获取访问令牌</button>
    <span class="status">→ POST /api/cc-token (本服务代理 → /oauth/token)</span>
  </div>
  <div id="errbox3" class="errbox" hidden></div>
  <div id="token-result3" hidden>
    <div class="kv"><span class="k">access_token</span><span class="v mono" id="p-at3">-</span><button id="btn-copy-at3" class="copy">复制</button></div>
    <div class="kv"><span class="k">token_type / expires_in / scope</span><span class="v mono" id="p-meta3">-</span></div>
    <details><summary>令牌端点响应 JSON</summary><pre id="raw-token3" class="mono"></pre></details>
  </div>
</section>

<section class="card" id="menus-card">
  <h2>② 拉取关联应用菜单树</h2>
  <p>等价 curl: <code>curl -H "Authorization: Bearer &lt;token&gt;" http://localhost:3001/api/external/menus</code> — 第三方应用可用其渲染导航或同步按钮级权限码。</p>
  <div class="btnrow"><button id="btn-menus" class="secondary">GET /api/external/menus</button></div>
  <div id="menus-err" class="errbox" hidden></div>
  <div id="menus-result" hidden>
    <div class="kv"><span class="k">应用</span><span class="v mono" id="menus-app">-</span></div>
    <div class="kv"><span class="k">令牌身份</span><span class="v mono" id="menus-client">-</span></div>
    <details open><summary>菜单树 (DIR/MENU/BUTTON 缩进层级)</summary><pre id="menus-tree" class="mono"></pre></details>
    <details><summary>响应 JSON</summary><pre id="raw-menus" class="mono"></pre></details>
  </div>
</section>

<section class="card" id="roles-card">
  <h2>③ 拉取授权组织与角色清单</h2>
  <p>等价 curl: <code>curl -H "Authorization: Bearer &lt;token&gt;" http://localhost:3001/api/external/roles</code> — 返回授权了该应用的组织及其角色, 每个角色附本应用的菜单授权数。</p>
  <div class="btnrow"><button id="btn-roles" class="secondary">GET /api/external/roles</button></div>
  <div id="roles-err" class="errbox" hidden></div>
  <div id="roles-result" hidden>
    <div class="kv"><span class="k">应用</span><span class="v mono" id="roles-app">-</span></div>
    <div id="roles-list"></div>
    <details><summary>响应 JSON</summary><pre id="raw-roles" class="mono"></pre></details>
  </div>
</section>

<section class="card">
  <h2>④ 教学点 · 鉴权与错误路径</h2>
  <p>对外 API 与 userinfo 同源鉴权: 无令牌 401 / 伪造 401 / 已吊销 401 / <b>客户端未关联应用 → 404「该客户端未关联应用」</b>。关联关系在管理端 OAuth 客户端编辑页随时可改 (清除关联后本页 ②③ 立即 404)。</p>
  <div class="btnrow"><button id="btn-noauth" class="secondary">不带令牌调 /api/external/menus</button></div>
  <div class="kv"><span class="k">观察结果</span><span class="v mono" id="noauth-result" hidden></span></div>
</section>

<section class="card">
  <h2>流程日志</h2>
  <div id="ext-log" class="logbox"></div>
</section>`,
})
