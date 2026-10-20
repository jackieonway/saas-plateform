import './style.css'
import { apiRequest, explainError } from './api.js'
import { createDevicePoller } from './device-poller.js'

// Capture once and remove the authorization response before any network work.
// The verifier and all tokens stay inside the Rust session, never browser state.
const isCallback = location.pathname === '/callback' || location.pathname === '/callback.html'
let callback
if (isCallback) {
  const params = new URLSearchParams(location.search)
  callback = { state: params.get('state') || '', ...(params.has('code') ? { code: params.get('code') } : {}), ...(params.has('error') ? { error: params.get('error') } : {}) }
  history.replaceState(null, '', '/callback')
}

const flows = {
  authorization_code: {
    number: '01', title: '授权码 + PKCE', english: 'AUTHORIZATION CODE', standard: 'RFC 7636 · S256',
    subtitle: '浏览器发起授权，服务端完成机密交换。',
    steps: ['生成挑战', '平台批准', '验证回调', '服务端换取令牌'],
    detail: '发起会话绑定 state 与 S256 挑战。平台批准后回到本页，API 一次性消费授权状态并完成交换。',
    identity: '用户身份',
  },
  device: {
    number: '02', title: '设备授权', english: 'DEVICE AUTHORIZATION', standard: 'RFC 8628',
    subtitle: '在另一台设备批准，观察真实轮询状态。',
    steps: ['申请用户码', '用户登录批准', '按间隔轮询', '服务端保存令牌'],
    detail: '将用户码输入平台验证页。页面遵守平台 interval，遇到 slow_down 后延长间隔，过期或离开页面即停止轮询。',
    identity: '用户身份',
  },
  client_credentials: {
    number: '03', title: '客户端凭据', english: 'CLIENT CREDENTIALS', standard: 'RFC 6749 · §4.4',
    subtitle: '以服务身份访问，观察用户上下文边界。',
    steps: ['读取私有配置', '客户端认证', '签发服务令牌', '内省与吊销'],
    detail: '由 API 使用受控客户端凭据请求平台。此令牌表示服务身份，调用 userinfo 时应观察到缺少用户上下文的拒绝。',
    identity: '服务身份',
  },
  password: {
    number: '04', title: '资源密码', english: 'LEGACY PASSWORD', standard: '历史兼容 · 默认禁用',
    subtitle: '仅用于验证既有系统的兼容行为。',
    steps: ['私有开关启用', '提交测试凭据', '平台校验', '刷新与失效验证'],
    detail: 'password grant 仅保留历史兼容演练，不作为新业务登录方案。只有服务端明确启用且配置客户端后，才可提交测试账号。',
    identity: '历史用户身份',
  },
}

document.querySelector('#app').innerHTML = `
  <aside class="sidebar">
    <a class="brand" href="/" aria-label="OAuth 协议实验室首页"><span class="brand-mark" aria-hidden="true"><i></i><i></i><i></i><i></i></span><span>OAuth<span class="brand-sub">协议实验室</span></span></a>
    <div class="rail-label">PLAYGROUND <span> / 01—04</span></div>
    <nav class="flow-nav" aria-label="OAuth 授权流程">
      ${Object.entries(flows).map(([key, flow]) => `<button type="button" data-flow="${key}" data-testid="flow-${key}"><span class="nav-number">${flow.number}</span><span>${flow.title}${key === 'password' ? '<small>历史兼容</small>' : ''}</span><span class="nav-arrow" aria-hidden="true">↗</span></button>`).join('')}
    </nav>
    <a class="resource-nav" href="#resources"><span aria-hidden="true">⌘</span> 资源与令牌检查 <span aria-hidden="true">↓</span></a>
    <div class="sidebar-note"><span class="lock-icon" aria-hidden="true">▣</span><strong>机密留在服务端</strong><p>页面只接收流程状态与公开摘要。<br>会话由同源 Cookie 绑定。</p><span class="rail-build">VITE STATIC / RUST API</span></div>
  </aside>
  <main>
    <header class="topbar"><span class="breadcrumb">开发工具 <span>/</span> OAuth Playground</span><span class="connection" id="connection"><i></i><span>连接流程 API</span></span></header>
    <section class="hero">
      <div><p class="eyebrow"><span></span> OAUTH 2.0 / LIVE PROTOCOL EXERCISE</p><h1>每一步授权，<br><em>都清楚可见。</em></h1><p class="hero-description">在真实平台中演练授权、检查身份边界与令牌生命周期。<br>选择一个流程，从协议开始。</p></div>
      <div class="hero-diagram" aria-hidden="true"><span class="diagram-label">TRUST BOUNDARY</span><div class="diagram-row"><div class="diagram-node">浏览器<small>PUBLIC</small></div><span class="diagram-edge">↔</span><div class="diagram-node api">流程 API<small>PRIVATE</small></div></div><div class="diagram-tail"><span>同源会话</span><span>↕</span></div><div class="platform-node">平台授权服务器 <span>OAUTH 2.0</span></div></div>
    </section>
    <div id="notice" class="notice" role="status" aria-live="polite" hidden></div>
    <section class="workspace" aria-label="授权流程演练">
      <article class="flow-card">
        <div class="section-heading"><span class="section-label">授权流程 <span id="flow-number">01</span></span><span id="flow-standard" class="technical-tag"></span></div>
        <div class="flow-title-row"><div><p class="flow-english" id="flow-english"></p><h2 id="flow-title"></h2></div><span class="capability" id="flow-capability">正在读取配置</span></div>
        <p id="flow-subtitle" class="flow-subtitle"></p>
        <ol id="flow-steps" class="flow-steps"></ol>
        <p id="flow-detail" class="flow-detail"></p>
        <div class="flow-panel" data-panel="authorization_code"><button class="primary" id="start-auth" data-testid="start-auth" type="button" disabled>发起平台授权 <span aria-hidden="true">↗</span></button><p class="helper">将跳转至平台登录与授权确认页，完成后自动返回。</p></div>
        <div class="flow-panel" data-panel="device" hidden>
          <div class="button-row"><button class="primary" id="start-device" data-testid="start-device" type="button" disabled>申请设备授权 <span aria-hidden="true">→</span></button><button class="secondary" id="stop-device" data-testid="stop-device" type="button" disabled>停止轮询</button></div>
          <div class="device-card" id="device-card" hidden><span class="eyebrow">输入用户码 / USER CODE</span><strong id="device-user-code" data-testid="device-user-code"></strong><a class="device-link" id="device-authorize-link" data-testid="device-authorize-link" target="_blank" rel="noopener noreferrer">前往平台批准授权 <span aria-hidden="true">↗</span></a><p id="device-timing"></p><p class="helper" id="device-status"></p></div>
          <p class="helper">批准页将在新标签打开。此页仅持有公开用户码。</p>
        </div>
        <div class="flow-panel" data-panel="client_credentials" hidden><button class="primary" id="issue-client_credentials" data-testid="issue-client_credentials" type="button" disabled>获取服务身份授权 <span aria-hidden="true">→</span></button><p class="helper">客户端认证由 Rust API 处理，页面只展示签发摘要。</p></div>
        <div class="flow-panel" data-panel="password" hidden>
          <div class="legacy-note"><strong>历史兼容，默认关闭</strong><p>新业务请使用授权码 + PKCE。启用此模式需要维护者设置服务端私有开关，界面无法开启。</p></div>
          <form id="password-form" autocomplete="off"><div class="credential-fields"><label>测试用户名<input id="username" name="username" data-testid="username" autocomplete="off" spellcheck="false" required disabled placeholder="输入测试账号"></label><label>测试密码<input id="password" name="password" data-testid="password" type="password" autocomplete="off" required disabled placeholder="输入测试密码"></label></div><button class="primary" id="issue-password" data-testid="issue-password" type="submit" disabled>验证历史兼容流程 <span aria-hidden="true">→</span></button></form>
        </div>
      </article>
      <aside class="summary-card" aria-label="令牌公开摘要">
        <div class="section-heading"><span class="section-label">会话观察</span><span class="session-dot" aria-hidden="true"></span></div>
        <div class="summary-icon" aria-hidden="true"><span>↳</span></div><h3 id="summary-title">等待授权</h3><p id="summary-description">令牌保存在流程 API。完成授权后，此处显示公开摘要。</p>
        <dl class="token-summary" id="token-summary" data-testid="token-summary"><div><dt>当前身份</dt><dd id="summary-identity">用户身份</dd></div><div><dt>令牌类型</dt><dd id="summary-type">—</dd></div><div><dt>有效时长</dt><dd id="summary-expiry">—</dd></div><div><dt>授权范围</dt><dd id="summary-scope">—</dd></div><div><dt>可刷新</dt><dd id="summary-refresh">—</dd></div></dl>
        <p class="summary-note"><span aria-hidden="true">↺</span> 刷新页面后，仍可选择原流程并通过内省检查服务端会话。</p>
      </aside>
    </section>
    <section id="resources" class="resources-card" aria-labelledby="resources-title">
      <div class="resource-heading"><div><span class="section-label">下一步 / INSPECT & VERIFY</span><h2 id="resources-title">检查授权，调用资源。</h2></div><label class="flow-select-label" for="resource-flow">使用流程<select id="resource-flow" data-testid="resource-flow">${Object.entries(flows).map(([key, flow]) => `<option value="${key}">${flow.title}</option>`).join('')}</select></label></div>
      <p class="resource-note">请求携带流程名称，由 API 选取会话中的令牌。普通 OAuth 令牌调用应用绑定的菜单 / 角色 API 时，平台应明确拒绝。</p>
      <div class="resource-actions">
        <button type="button" data-resource="userinfo" data-testid="resource-userinfo"><span class="action-symbol">◎</span><strong>用户信息</strong><small>userinfo</small><span class="action-arrow">↗</span></button>
        <button type="button" data-resource="introspect" data-testid="resource-introspect"><span class="action-symbol">⌕</span><strong>令牌内省</strong><small>introspect</small><span class="action-arrow">↗</span></button>
        <button type="button" data-resource="refresh-token" data-testid="resource-refresh-token"><span class="action-symbol">↻</span><strong>刷新授权</strong><small>refresh_token</small><span class="action-arrow">↗</span></button>
        <button type="button" data-resource="revoke" data-testid="resource-revoke"><span class="action-symbol">⊘</span><strong>吊销令牌</strong><small>revoke</small><span class="action-arrow">↗</span></button>
        <button type="button" data-resource="external-menus" data-testid="resource-external-menus"><span class="action-symbol">⊞</span><strong>应用菜单</strong><small>external / menus</small><span class="action-arrow">↗</span></button>
        <button type="button" data-resource="external-roles" data-testid="resource-external-roles"><span class="action-symbol">♧</span><strong>应用角色</strong><small>external / roles</small><span class="action-arrow">↗</span></button>
      </div>
      <div class="response-section"><div class="response-heading"><span class="section-label">协议响应 <span id="response-name">/ 尚无请求</span></span><span class="response-status" id="response-status">IDLE</span></div><pre id="response-json" data-testid="response-json" tabindex="0">// 发起授权，或选择一个操作查看真实响应。</pre></div>
    </section>
    <section class="log-section" aria-label="流程事件"><div class="log-heading"><span class="section-label">流程事件 / SESSION EVENTS</span><button class="text-button" id="clear-log" type="button">清空记录</button></div><ol id="events" class="events" aria-live="polite"><li class="empty-event">流程就绪后，操作事件将在这里出现。</li></ol></section>
    <footer><span>OAuth Playground <span class="footer-divider">/</span> 协议演练工具</span><span id="public-origin">读取公开配置…</span></footer>
  </main>`

const $ = selector => document.querySelector(selector)
const summaries = new Map()
let config
let activeFlow = Object.hasOwn(flows, location.hash.slice(1)) ? location.hash.slice(1) : 'authorization_code'
let busy = false
let deviceRunning = false
let countdown
let deviceStartGeneration = 0
let deviceStartController

function log(message, kind = 'info') {
  const events = $('#events')
  events.querySelector('.empty-event')?.remove()
  const line = document.createElement('li')
  line.className = kind
  const time = document.createElement('time')
  time.textContent = new Date().toLocaleTimeString('zh-CN', { hour12: false })
  const text = document.createElement('span')
  text.textContent = message
  line.append(time, text)
  events.prepend(line)
  while (events.children.length > 40) events.lastElementChild.remove()
}

function notice(message, kind = 'info') {
  $('#notice').textContent = message
  $('#notice').className = `notice ${kind}`
  $('#notice').hidden = false
}

function capability(flow) { return config?.flows?.[flow]?.enabled === true }

function setBusy(value) {
  busy = value
  updateButtons()
}

function updateButtons() {
  $('#start-auth').disabled = busy || !capability('authorization_code')
  $('#start-device').disabled = busy || deviceRunning || !capability('device')
  $('#stop-device').disabled = !deviceRunning
  $('#issue-client_credentials').disabled = busy || !capability('client_credentials')
  $('#issue-password').disabled = busy || !capability('password')
  $('#username').disabled = busy || !capability('password')
  $('#password').disabled = busy || !capability('password')
  document.querySelectorAll('[data-resource]').forEach(button => { button.disabled = busy || !capability($('#resource-flow').value) })
  $('#resource-flow').disabled = busy || !config
}

function renderSummary() {
  const summary = summaries.get(activeFlow)
  const granted = summary?.granted === true
  $('.summary-card').classList.toggle('granted', granted)
  $('#summary-title').textContent = granted ? '授权已就绪' : summary?.revoked ? '令牌已吊销' : '等待授权'
  $('#summary-description').textContent = granted ? '服务端已接收令牌。页面只展示公开元数据，可继续检查资源。' : summary?.revoked ? '服务端已移除此流程的令牌。需要重新完成授权后，再访问资源。' : '令牌保存在流程 API。完成授权后，此处显示公开摘要。'
  $('#summary-identity').textContent = flows[activeFlow].identity
  $('#summary-type').textContent = granted ? summary.tokenType || '—' : '—'
  $('#summary-expiry').textContent = granted && Number.isFinite(summary.expiresIn) ? `${summary.expiresIn} 秒` : '—'
  $('#summary-scope').textContent = granted ? summary.scope || '未返回' : config?.scope || '—'
  $('#summary-refresh').textContent = granted ? summary.hasRefreshToken ? '是 · 服务端轮换' : '否' : '—'
}

function selectFlow(flow, updateHash = true) {
  if (!Object.hasOwn(flows, flow)) return
  activeFlow = flow
  const item = flows[flow]
  document.querySelectorAll('[data-flow]').forEach(button => {
    const selected = button.dataset.flow === flow
    button.classList.toggle('active', selected)
    button.setAttribute('aria-pressed', String(selected))
  })
  document.querySelectorAll('[data-panel]').forEach(panel => { panel.hidden = panel.dataset.panel !== flow })
  $('#flow-number').textContent = item.number
  $('#flow-title').textContent = item.title
  $('#flow-english').textContent = item.english
  $('#flow-standard').textContent = item.standard
  $('#flow-subtitle').textContent = item.subtitle
  $('#flow-detail').textContent = item.detail
  $('#flow-steps').replaceChildren(...item.steps.map((step, index) => {
    const li = document.createElement('li')
    const span = document.createElement('span')
    span.textContent = String(index + 1).padStart(2, '0')
    li.append(span, document.createTextNode(step))
    return li
  }))
  const enabled = capability(flow)
  $('#flow-capability').textContent = !config ? '等待配置' : enabled ? '可演练' : config.flows?.[flow]?.reason === 'legacy_password_disabled' ? '默认禁用' : '未配置'
  $('#flow-capability').classList.toggle('enabled', enabled)
  $('#resource-flow').value = flow
  if (updateHash && !isCallback) history.replaceState(null, '', `/#${flow}`)
  renderSummary()
  updateButtons()
}

function showResult(name, reply) {
  $('#response-name').textContent = `/ ${name}`
  $('#response-status').textContent = `HTTP ${reply.status}`
  $('#response-status').className = `response-status ${reply.ok ? 'ok' : 'error'}`
  $('#response-json').textContent = JSON.stringify(reply.data, null, 2)
  log(`${name} · HTTP ${reply.status}`, reply.ok ? 'ok' : 'error')
  if (!reply.ok) notice(explainError(reply.data?.error), 'error')
}

function acceptSummary(data, flow) {
  if (data?.granted !== true) return false
  summaries.set(flow, data)
  renderSummary()
  notice(`${flows[flow].title}已完成，令牌由服务端会话保存。`, 'ok')
  return true
}

async function perform(path, body = {}, { onSuccess } = {}) {
  if (busy) return
  setBusy(true)
  try {
    const reply = await apiRequest(`/api/${path}`, body)
    showResult(path, reply)
    if (reply.ok) onSuccess?.(reply.data)
    return reply
  } catch (error) {
    notice(explainError(error.name === 'TimeoutError' ? 'request_timeout' : 'network_error'), 'error')
    log(`${path} · 请求未完成`, 'error')
  } finally {
    setBusy(false)
  }
}

function stopDevice(message) {
  deviceStartGeneration++
  deviceStartController?.abort()
  deviceStartController = undefined
  poller.cancel()
  clearInterval(countdown)
  deviceRunning = false
  if (message) { $('#device-status').textContent = message; log(message) }
  updateButtons()
}

const poller = createDevicePoller({
  request: signal => apiRequest('/api/device-token', {}, signal),
  onEvent(event) {
    if (event.kind === 'waiting' || event.kind === 'pending' || event.kind === 'slow_down') {
      const update = () => { $('#device-timing').textContent = `剩余 ${Math.max(0, Math.ceil((event.expiresAt - Date.now()) / 1000))} 秒 · 每 ${event.interval} 秒轮询` }
      clearInterval(countdown)
      update()
      countdown = setInterval(update, 1000)
      $('#device-status').textContent = event.kind === 'waiting' ? '等待平台批准，首次轮询将在指定间隔后开始。' : explainError(event.kind === 'slow_down' ? 'slow_down' : 'authorization_pending')
      if (event.kind !== 'waiting') log(`设备授权 · ${event.kind === 'slow_down' ? '降低轮询频率' : '等待批准'}`)
      return
    }
    stopDevice()
    if (event.kind === 'granted') {
      showResult('device-token', { ok: true, status: 200, data: event.data })
      acceptSummary(event.data, 'device')
      $('#device-status').textContent = '授权已批准，轮询结束。'
    } else {
      const error = event.kind === 'expired' ? 'expired_token' : event.error
      $('#device-status').textContent = explainError(error)
      notice(explainError(error), 'error')
      log(`设备授权 · ${event.kind === 'expired' ? '已过期' : '已停止'}`, 'error')
    }
  },
})

function trustedPlatformLink(raw) {
  try {
    const url = new URL(raw)
    const platform = new URL(config.platformOrigin)
    return url.origin === platform.origin && ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password ? url.href : undefined
  } catch { return undefined }
}

$('#start-auth').addEventListener('click', () => perform('auth-start', {}, { onSuccess(data) {
  const authorizeUrl = trustedPlatformLink(data.authorizeUrl)
  if (authorizeUrl) location.assign(authorizeUrl)
  else notice('授权跳转地址不属于已配置的平台，已停止跳转。', 'error')
} }))

$('#start-device').addEventListener('click', async () => {
  if (busy || deviceRunning) return
  stopDevice()
  const generation = deviceStartGeneration
  deviceStartController = new AbortController()
  deviceRunning = true
  $('#device-card').hidden = true
  updateButtons()
  try {
    const reply = await apiRequest('/api/device-start', {}, deviceStartController.signal)
    if (generation !== deviceStartGeneration) return
    showResult('device-start', reply)
    const data = reply.data
    const verificationUrl = trustedPlatformLink(data.verification_uri_complete || data.verification_uri)
    if (!reply.ok || !data.user_code || !verificationUrl) {
      stopDevice()
      if (reply.ok) notice('平台验证地址或用户码无效，设备轮询未启动。', 'error')
      return
    }
    $('#device-user-code').textContent = data.user_code
    $('#device-authorize-link').href = verificationUrl
    $('#device-card').hidden = false
    poller.start(data)
  } catch (error) {
    if (generation === deviceStartGeneration) {
      stopDevice()
      notice(explainError(error.name === 'TimeoutError' ? 'request_timeout' : 'network_error'), 'error')
    }
  }
})
$('#stop-device').addEventListener('click', () => stopDevice('设备轮询已停止，可重新申请用户码。'))

$('#issue-client_credentials').addEventListener('click', () => perform('cc-token', {}, { onSuccess: data => acceptSummary(data, 'client_credentials') }))
$('#password-form').addEventListener('submit', event => {
  event.preventDefault()
  if (!capability('password') || busy) return
  const body = { username: $('#username').value, password: $('#password').value }
  // Remove the password from the form immediately after capturing this request.
  $('#password').value = ''
  void perform('password-token', body, { onSuccess: data => acceptSummary(data, 'password') })
})

document.querySelectorAll('[data-flow]').forEach(button => button.addEventListener('click', () => selectFlow(button.dataset.flow)))
$('#resource-flow').addEventListener('change', () => { updateButtons() })
document.querySelectorAll('[data-resource]').forEach(button => button.addEventListener('click', () => {
  const flow = $('#resource-flow').value
  const operation = button.dataset.resource
  void perform(operation, { flow }, { onSuccess(data) {
    if (operation === 'refresh-token') acceptSummary(data, flow)
    else if (operation === 'revoke') {
      summaries.set(flow, { revoked: true })
      renderSummary()
      notice('吊销请求已完成，服务端已移除本流程令牌。再次内省将明确拒绝缺失令牌。', 'ok')
    } else notice('平台已返回真实资源响应。', 'ok')
  } })
}))
$('#clear-log').addEventListener('click', () => { $('#events').replaceChildren() })
window.addEventListener('pagehide', () => stopDevice())

selectFlow(isCallback ? 'authorization_code' : activeFlow, false)
if (isCallback) notice('已接收平台回调，正在验证发起会话与一次性授权状态。')

async function init() {
  try {
    const reply = await apiRequest('/api/config')
    if (!reply.ok) throw new Error('config_unavailable')
    config = reply.data
    $('#connection').classList.add('connected')
    $('#connection span').textContent = '流程 API 已连接'
    $('#public-origin').textContent = config.publicOrigin || location.origin
    selectFlow(activeFlow, false)
    log('公开配置已读取，流程能力由服务端决定。')
    if (isCallback) {
      const result = await perform('callback', callback, { onSuccess: data => acceptSummary(data, 'authorization_code') })
      if (result?.ok && result.data?.granted !== true) notice('回调未返回授权摘要，请重新发起流程。', 'error')
      callback = undefined
    }
  } catch {
    $('#connection').classList.add('unavailable')
    $('#connection span').textContent = '流程 API 不可达'
    notice('无法读取流程配置。请启动 Rust API 并检查同源代理，随后刷新页面。', 'error')
    log('配置请求失败，授权操作保持禁用。', 'error')
    callback = undefined
  }
}

void init()
