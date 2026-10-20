/**
 * micro-jquery 子应用 — SaaS Platform 微前端「遗留系统」演示 (jQuery, 无构建步骤)
 * 模拟十年前的内部工单处理系统, 证明遗留栈也能接入新权限链路。
 *
 * 生命周期契约 (与主应用共享, 必须精确一致):
 *   window.__MICRO_APP__ = {
 *     name: string,
 *     mount(container: HTMLElement, props: MicroProps): Promise<void>,
 *     update(props: MicroProps): Promise<void>, unmount(): Promise<void>
 *   }
 *   MicroProps 2.0 包含应用 token、租户、组织、角色、操作权限、独立 dataGrants、
 *   contextVersion/contextEpoch、两个到期时间和宿主 AbortSignal；详见接入文档。
 *
 * 实现说明:
 * - jQuery 来自 /vendor/jquery.min.js (先于本文件加载)。jQuery UMD 会显式把
 *   jQuery/$ 挂到 window 上, 沙箱内直接取 window.jQuery 即可 (兜底 window.$)。
 * - 事件采用「遗留风格」的 $(document) 命名空间委托 (.mfjq), unmount 时
 *   $(document).off('.mfjq') 精确解绑, 不污染主应用的事件表。
 * - 样式全部手写、选择器一律 .mf-jq- 前缀, <style> 注入 mount 容器内, 卸载随容器清空。
 * - mount 时向 window 写入 __JQ_APP_MARKER (沙箱清理探针), 卸载时刻意不由本应用删除,
 *   该痕迹应由主应用沙箱负责回收, 供其验证「真实 window 无残留」。
 */
;(function () {
  'use strict'

  /* ===== 解析 jQuery 全局库 ===== */
  var $ = null
  if (typeof window !== 'undefined') {
    if (typeof window.jQuery === 'function') $ = window.jQuery
    else if (typeof window.$ === 'function') $ = window.$
  }

  /* ===== 模块级挂载状态 ===== */
  var currentProps = null
  var authorizationKey = null
  var contextController = null
  var contextClient = null
  var hostSignal = null
  var hostAbort = null
  var requestEpoch = 0
  var mountContainer = null
  var tickTimer = null
  var tickSeconds = 0
  var updateCount = 0
  var state = { sortKey: 'id', sortDir: 'asc', keyword: '', status: '' }

  function find(selector) {
    return mountContainer ? mountContainer.querySelector(selector) : null
  }

  function contextUsable() {
    var p = currentProps
    return !!(p && p.contractVersion === '2.0' && p.token && contextController &&
      !contextController.signal.aborted && Date.parse(p.expiresAt) > Date.now() &&
      Date.parse(p.tokenExpiresAt) > Date.now())
  }

  function clearResults() {
    var list = find('#mf-jq-menu-list')
    if (list) list.innerHTML = ''
    setStatus('#mf-jq-menu-status', '上下文已更新，旧请求和结果已清理', null)
  }

  function stopContext() {
    requestEpoch += 1
    if (contextClient) contextClient.dispose()
    contextClient = null
    if (contextController) contextController.abort()
    contextController = null
    if (hostSignal && hostAbort) hostSignal.removeEventListener('abort', hostAbort)
    hostSignal = null
    hostAbort = null
    clearResults()
  }

  function applyContext(props) {
    if (!props || props.contractVersion !== '2.0' || !props.signal || !props.tenant || !props.organization || !props.user) {
      throw new Error('微应用需要 MicroProps 2.0 完整上下文和 AbortSignal')
    }
    var nextKey = window.MicroSdk && window.MicroSdk.authorizationContextSignature(props)
    var changed = !contextUsable() || nextKey === null || nextKey !== authorizationKey || hostSignal !== props.signal
    if (changed) {
      stopContext()
      currentProps = props
      contextController = new AbortController()
      if (!window.MicroSdk || window.MicroSdk.sdkVersion !== '1.0.0') throw new Error('Registered micro SDK 1.0.0 required')
      contextClient = window.MicroSdk.createContextClient(Object.assign({}, props, { signal: contextController.signal }))
      hostSignal = props.signal
      hostAbort = function () {
        requestEpoch += 1
        if (contextController) contextController.abort()
        clearResults()
        renderContext()
      }
      hostSignal.addEventListener('abort', hostAbort, { once: true })
      if (hostSignal.aborted) hostAbort()
    } else {
      currentProps = props
      contextClient.update(Object.assign({}, props, { signal: contextController.signal }))
    }
    authorizationKey = nextKey
  }

  /* ===== mock 工单数据 (2018 年存档风格) ===== */
  var TICKETS = [
    { id: 'GD-2018-0031', title: '生产环境数据库连接超时', status: '处理中', priority: '紧急', updated: '2018-11-02 09:41' },
    { id: 'GD-2018-0030', title: '客户报表导出乱码（GBK 编码问题）', status: '待处理', priority: '高', updated: '2018-11-01 17:25' },
    { id: 'GD-2018-0029', title: '门禁系统每日凌晨自动掉线', status: '已解决', priority: '普通', updated: '2018-10-30 11:03' },
    { id: 'GD-2018-0028', title: '老机房 UPS 电池告警', status: '处理中', priority: '紧急', updated: '2018-10-29 20:17' },
    { id: 'GD-2018-0027', title: 'ERP 接口偶发 502', status: '待处理', priority: '高', updated: '2018-10-28 14:52' },
    { id: 'GD-2018-0026', title: '财务室打印机驱动异常', status: '已关闭', priority: '低', updated: '2018-10-26 10:30' },
    { id: 'GD-2018-0025', title: 'OA 系统证书到期提醒', status: '已解决', priority: '普通', updated: '2018-10-25 09:12' },
    { id: 'GD-2018-0024', title: '仓库扫码枪批量失灵', status: '待处理', priority: '高', updated: '2018-10-24 16:44' },
    { id: 'GD-2018-0023', title: '邮件网关垃圾邮件误杀', status: '已关闭', priority: '普通', updated: '2018-10-22 13:08' },
    { id: 'GD-2018-0022', title: 'VPN 账号锁定重置', status: '已解决', priority: '低', updated: '2018-10-19 15:57' },
  ]

  var PRIORITY_RANK = { 紧急: 0, 高: 1, 普通: 2, 低: 3 }
  var STATUS_CLASS = { 待处理: 'wait', 处理中: 'doing', 已解决: 'done', 已关闭: 'closed' }
  var PRI_CLASS = { 紧急: 'urgent', 高: 'high', 普通: 'normal', 低: 'low' }

  /* ===== 手写样式 (全部 .mf-jq- 前缀, 带一点「旧系统」味道但整体干净) ===== */
  var CSS = [
    '.mf-jq-root{font-family:Verdana,"Microsoft YaHei","PingFang SC",sans-serif;font-size:12px;color:#333333;background:#ffffff;border:1px solid #d6d6d6;border-radius:6px;padding:12px;box-sizing:border-box;}',
    '.mf-jq-root *,.mf-jq-root *::before,.mf-jq-root *::after{box-sizing:border-box;}',
    '.mf-jq-header{display:flex;flex-wrap:wrap;gap:6px;align-items:center;padding-bottom:10px;border-bottom:1px solid #e6e6e6;margin-bottom:10px;}',
    '.mf-jq-badge{display:inline-block;border:1px solid #d6d6d6;background:#f7f7f7;color:#555555;border-radius:3px;font-size:11px;line-height:17px;padding:0 7px;white-space:nowrap;}',
    '.mf-jq-badge-jq{background:#0769ad;border-color:#0769ad;color:#ffffff;font-weight:700;}',
    '.mf-jq-badge-upd{background:#ecfdf5;border-color:#a7f3d0;color:#047857;}',
    '.mf-jq-route{margin-left:auto;font-size:11px;color:#777777;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}',
    '.mf-jq-route code{font-family:"Courier New",monospace;background:#f0f0f0;padding:0 4px;}',
    '.mf-jq-tokenbox{font-size:11px;color:#777777;white-space:nowrap;}',
    '.mf-jq-token{font-family:"Courier New",monospace;font-size:11px;background:#f0f0f0;border:1px solid #e0e0e0;border-radius:3px;padding:0 4px;color:#555555;}',
    '.mf-jq-title-row{margin:2px 0 10px;}',
    '.mf-jq-title{font-family:Georgia,"Times New Roman","SimSun",serif;font-size:16px;font-weight:700;color:#222222;margin:0;display:inline;}',
    '.mf-jq-sub{font-size:11px;color:#999999;margin-left:8px;}',
    '.mf-jq-toolbar{display:flex;flex-wrap:wrap;gap:8px;align-items:center;margin-bottom:8px;}',
    '.mf-jq-input{border:1px solid #bbbbbb;padding:4px 6px;font-size:12px;border-radius:3px;width:230px;max-width:60%;font-family:inherit;}',
    '.mf-jq-input:focus{outline:none;border-color:#0769ad;}',
    '.mf-jq-select{border:1px solid #bbbbbb;padding:4px 4px;font-size:12px;border-radius:3px;background:#ffffff;font-family:inherit;}',
    '.mf-jq-count{color:#888888;font-size:11px;margin-left:auto;white-space:nowrap;}',
    '.mf-jq-tablewrap{max-height:340px;overflow-y:auto;border:1px solid #dddddd;}',
    '.mf-jq-table{width:100%;border-collapse:collapse;font-size:12px;background:#ffffff;}',
    '.mf-jq-table th{background:#f2f2f2;border:1px solid #dddddd;padding:4px 8px;text-align:left;font-weight:700;white-space:nowrap;cursor:pointer;-webkit-user-select:none;user-select:none;}',
    '.mf-jq-table th.mf-jq-sort:hover{background:#eaeaea;}',
    '.mf-jq-table th.mf-jq-sorted{background:#e8f2ee;}',
    '.mf-jq-table td{border:1px solid #dddddd;padding:4px 8px;vertical-align:top;}',
    '.mf-jq-table tbody tr:nth-child(even){background:#fafafa;}',
    '.mf-jq-table tbody tr:hover{background:#f0f7f4;}',
    '.mf-jq-table code{font-family:"Courier New",monospace;color:#555555;}',
    '.mf-jq-time{color:#666666;white-space:nowrap;font-family:"Courier New",monospace;font-size:11px;}',
    '.mf-jq-empty{text-align:center;color:#999999;padding:12px !important;}',
    '.mf-jq-st,.mf-jq-pri{display:inline-block;font-size:11px;line-height:16px;padding:0 6px;border-radius:2px;border:1px solid transparent;white-space:nowrap;}',
    '.mf-jq-st-wait{background:#fef7e0;border-color:#f3d9a4;color:#8a6d1a;}',
    '.mf-jq-st-doing{background:#e2f5ef;border-color:#a8ddc9;color:#1a7a5e;}',
    '.mf-jq-st-done{background:#e9f6e2;border-color:#b5dca0;color:#3d7a1f;}',
    '.mf-jq-st-closed{background:#f1f1f1;border-color:#d9d9d9;color:#888888;}',
    '.mf-jq-pri-urgent{background:#fbe5e2;border-color:#f0b3ac;color:#aa3333;}',
    '.mf-jq-pri-high{background:#fdf0dd;border-color:#e8c48f;color:#925e10;}',
    '.mf-jq-pri-normal{background:#f4f4f4;border-color:#d9d9d9;color:#555555;}',
    '.mf-jq-pri-low{background:#fafafa;border-color:#e3e3e3;color:#999999;}',
    '.mf-jq-cards{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:10px;margin-top:12px;}',
    '.mf-jq-card{border:1px solid #dddddd;border-radius:5px;padding:10px;background:#fdfdfd;min-width:0;}',
    '.mf-jq-card-title{font-family:Georgia,"Times New Roman",serif;font-weight:700;font-size:13px;color:#333333;margin-bottom:6px;border-bottom:1px dotted #cccccc;padding-bottom:4px;}',
    '.mf-jq-kv{list-style:none;margin:0;padding:0;}',
    '.mf-jq-kv li{padding:2px 0;color:#555555;}',
    '.mf-jq-kv b{color:#222222;font-variant-numeric:tabular-nums;}',
    '.mf-jq-kv code{font-family:"Courier New",monospace;background:#f0f0f0;padding:0 4px;font-size:11px;}',
    '.mf-jq-btn{border:1px solid #999999;background:#f5f5f5;background:linear-gradient(#ffffff,#eaeaea);color:#333333;border-radius:3px;padding:4px 12px;font-size:12px;line-height:18px;cursor:pointer;font-family:inherit;}',
    '.mf-jq-btn:hover{background:#eef6f2;border-color:#3a7a63;color:#1a7a5e;}',
    '.mf-jq-btn-info{border-color:#0769ad;color:#0769ad;background:#ffffff;}',
    '.mf-jq-btn-info:hover{background:#eef4fa;border-color:#0769ad;color:#054c7c;}',
    '.mf-jq-note{margin:6px 0 0;color:#888888;font-size:11px;}',
    '.mf-jq-menu-status{display:block;margin:6px 0 4px;font-size:11px;color:#777777;word-break:break-all;}',
    '.mf-jq-menu-status[data-ok="1"]{color:#2c7a52;}',
    '.mf-jq-menu-status[data-ok="0"]{color:#bb3333;}',
    '.mf-jq-menu-list{list-style:none;margin:0;padding:0;max-height:220px;overflow-y:auto;border:1px solid #eeeeee;background:#ffffff;}',
    '.mf-jq-menu-list li{padding:2px 8px;border-bottom:1px dotted #eeeeee;color:#444444;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-size:11px;}',
    '.mf-jq-menu-type{display:inline-block;font-size:10px;color:#888888;border:1px solid #dddddd;background:#f7f7f7;padding:0 3px;margin-right:4px;border-radius:2px;min-width:44px;text-align:center;}',
    '.mf-jq-menu-list code{font-family:"Courier New",monospace;color:#888888;font-size:10px;}',
  ].join('\n')

  /* ===== 辅助函数 ===== */
  function esc(v) {
    return String(v === null || v === undefined ? '' : v).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]
    })
  }

  function filteredTickets() {
    var kw = String(state.keyword || '').trim().toLowerCase()
    var rows = TICKETS.filter(function (t) {
      if (state.status && t.status !== state.status) return false
      if (kw && t.title.toLowerCase().indexOf(kw) === -1 && t.id.toLowerCase().indexOf(kw) === -1) return false
      return true
    })
    var dir = state.sortDir === 'desc' ? -1 : 1
    var key = state.sortKey
    rows = rows.slice().sort(function (a, b) {
      var va = a[key]
      var vb = b[key]
      if (key === 'priority') {
        va = PRIORITY_RANK[va] !== undefined ? PRIORITY_RANK[va] : 9
        vb = PRIORITY_RANK[vb] !== undefined ? PRIORITY_RANK[vb] : 9
      }
      if (va < vb) return -1 * dir
      if (va > vb) return 1 * dir
      return 0
    })
    return rows
  }

  function rowHtml(t) {
    var st = STATUS_CLASS[t.status] || 'closed'
    var pri = PRI_CLASS[t.priority] || 'normal'
    return (
      '<tr>' +
      '<td><code>' + esc(t.id) + '</code></td>' +
      '<td>' + esc(t.title) + '</td>' +
      '<td><span class="mf-jq-st mf-jq-st-' + st + '">' + esc(t.status) + '</span></td>' +
      '<td><span class="mf-jq-pri mf-jq-pri-' + pri + '">' + esc(t.priority) + '</span></td>' +
      '<td class="mf-jq-time">' + esc(t.updated) + '</td>' +
      '</tr>'
    )
  }

  function renderRows() {
    var rows = filteredTickets()
    var tbody = document.getElementById('mf-jq-tbody')
    if (!tbody) return
    tbody.innerHTML = rows.length
      ? rows.map(rowHtml).join('')
      : '<tr><td class="mf-jq-empty" colspan="5">没有匹配的工单。</td></tr>'
    var count = document.getElementById('mf-jq-count')
    if (count) count.textContent = rows.length + ' / ' + TICKETS.length + ' 条'
    // 更新表头排序指示
    var ths = document.querySelectorAll('#mf-jq-table th.mf-jq-sort')
    for (var i = 0; i < ths.length; i++) {
      var th = ths[i]
      var key = th.getAttribute('data-key')
      var arrow = th.querySelector('.mf-jq-arrow')
      if (key === state.sortKey) {
        th.className = 'mf-jq-sort mf-jq-sorted'
        if (arrow) arrow.textContent = state.sortDir === 'asc' ? ' ▲' : ' ▼'
      } else {
        th.className = 'mf-jq-sort'
        if (arrow) arrow.textContent = ' ↕'
      }
    }
  }

  function renderMenuFlat(nodes, depth) {
    var out = ''
    var arr = nodes || []
    for (var i = 0; i < arr.length; i++) {
      var n = arr[i]
      var indent = ''
      for (var d = 0; d < depth; d++) indent += '&nbsp;&nbsp;&nbsp;&nbsp;'
      out +=
        '<li>' + indent +
        '<span class="mf-jq-menu-type">' + esc(n.type) + '</span>' +
        esc(n.title) +
        (n.path ? ' <code>' + esc(n.path) + '</code>' : '') +
        '</li>'
      if (n.children && n.children.length) out += renderMenuFlat(n.children, depth + 1)
    }
    return out
  }

  function setStatus(selector, text, ok) {
    var el = find(selector)
    if (!el) return
    el.textContent = text
    if (ok === true) el.setAttribute('data-ok', '1')
    else if (ok === false) el.setAttribute('data-ok', '0')
    else el.removeAttribute('data-ok')
  }

  /* ===== 外部权限 API (遗留栈接新链路) ===== */
  function fetchMenus() {
    if (!contextUsable()) {
      setStatus('#mf-jq-menu-status', '应用上下文已失效，等待宿主更新', false)
      return Promise.resolve()
    }
    var epoch = requestEpoch
    var controller = contextController
    function isCurrent() {
      return epoch === requestEpoch && controller === contextController && contextUsable() && !!mountContainer
    }
    var token = (currentProps && currentProps.token) || ''
    var list = find('#mf-jq-menu-list')
    if (list) list.innerHTML = ''
    setStatus('#mf-jq-menu-status', '请求中…', null)
    return contextClient.requestJson('/api/external/menus')
      .then(function (body) { return { ok: true, status: 200, body: body } })
      .then(function (r) {
        if (!isCurrent()) return
        if (r.ok && r.body && r.body.menus) {
          var appName = r.body.application ? r.body.application.name : ''
          var clientId = r.body.client ? r.body.client.clientId : ''
          setStatus('#mf-jq-menu-status', 'HTTP ' + r.status + (appName ? ' · 应用「' + appName + '」' : '') + (clientId ? ' · ' + clientId : ''), true)
          if (list) list.innerHTML = renderMenuFlat(r.body.menus, 0)
        } else {
          var msg = r.body && r.body.message ? r.body.message : 'HTTP ' + r.status
          if (!token) msg += '（主应用未注入 token，空 Bearer 调用被拒绝属预期）'
          setStatus('#mf-jq-menu-status', 'HTTP ' + r.status + ' · ' + msg, false)
        }
      })
      .catch(function (err) {
        if (!isCurrent() || (err && err.name === 'AbortError')) return
        setStatus('#mf-jq-menu-status', '网络错误: ' + (err && err.message ? err.message : err), false)
      })
  }

  function renderContext() {
    var panel = find('#mf-jq-context')
    if (!panel) return
    var p = currentProps || {}
    var available = contextUsable()
    panel.innerHTML = '<div class="mf-jq-card-title">当前租户与组织</div>' +
      '<ul class="mf-jq-kv"><li>租户: <b>' + esc(p.tenant && p.tenant.name) + '</b> · ' + esc(p.tenant && p.tenant.code) + '</li>' +
      '<li>组织: <b>' + esc(p.organization && p.organization.name) + '</b> · ' + esc(p.organization && p.organization.path) + '</li>' +
      '<li>契约: ' + esc(p.contractVersion) + ' · 上下文序号 ' + esc(p.contextEpoch) + '</li>' +
      '<li>主题: ' + esc(p.theme) + '</li>' +
      '<li>上下文版本: <code>' + esc(p.contextVersion) + '</code></li>' +
      '<li>上下文到期: ' + esc(p.expiresAt) + '</li><li>令牌到期: ' + esc(p.tokenExpiresAt) + '</li></ul>' +
      '<div class="mf-jq-card-title">角色与操作权限</div>' +
      '<p class="mf-jq-note">' + (available ? esc((p.roles || []).map(function (r) { return r.name + ' (' + r.code + ')' }).join('、') || '没有应用角色') : '上下文已失效，等待宿主更新') + '</p>' +
      '<p class="mf-jq-note">' + (available ? esc((p.permissions || []).join('、') || '没有操作权限') : '') + '</p>' +
      '<div class="mf-jq-card-title">独立数据授权</div>' +
      '<ul class="mf-jq-kv">' + (available ? (p.dataGrants || []).map(function (g) {
        return '<li><b>' + esc(g.resource + ':' + g.action + ' · ' + g.scope) + '</b><br>用户 ' + esc(g.subjectUserId) + ' · 租户 ' + esc(g.tenantId) + ' · 组织 ' + esc((g.organizationIds || []).join(', ')) + '</li>'
      }).join('') : '') + '</ul>' +
      '<p class="mf-jq-note">没有显式数据授权时默认拒绝。业务后端必须同时校验操作权限和资源对应的数据范围。</p>' +
      '<ul class="mf-jq-menu-list">' + (available ? renderMenuFlat(p.menus, 0) : '') + '</ul>'
    var button = find('#mf-jq-btn-menus')
    if (button) button.disabled = !available
  }

  /* ===== 渲染整体骨架 ===== */
  function renderShell() {
    var p = currentProps || {}
    var u = p.user || {}
    var jqVersion = $.fn.jquery || '?'
    var route = p.routePath || ''
    var token = typeof p.token === 'string' ? p.token : ''
    var tokenChip = token
      ? '<code class="mf-jq-token">' + '应用令牌已注入' + '</code>'
      : '<span class="mf-jq-badge">未携带令牌</span>'

    var statusOptions = ['', '待处理', '处理中', '已解决', '已关闭']
      .map(function (s) {
        return '<option value="' + esc(s) + '">' + (s ? esc(s) : '全部状态') + '</option>'
      })
      .join('')

    var cols = [
      ['id', '编号'],
      ['title', '标题'],
      ['status', '状态'],
      ['priority', '优先级'],
      ['updated', '更新时间'],
    ]
      .map(function (c) {
        return '<th class="mf-jq-sort" data-key="' + c[0] + '">' + c[1] + '<span class="mf-jq-arrow"> ↕</span></th>'
      })
      .join('')

    mountContainer.innerHTML =
      '<style>\n' + CSS + '\n</style>\n' +
      '<div class="mf-jq-root">' +
      '<div class="mf-jq-header">' +
      '<span class="mf-jq-badge mf-jq-badge-jq">jQuery ' + esc(jqVersion) + '</span>' +
      '<span class="mf-jq-badge">遗留系统 · 微前端接入</span>' +
      '<span class="mf-jq-badge mf-jq-badge-upd mf-jq-upd-badge" style="display:none"></span>' +
      (route ? '<span class="mf-jq-route">routePath: <code>' + esc(route) + '</code></span>' : '') +
      '</div>' +
      '<div class="mf-jq-title-row">' +
      '<h1 class="mf-jq-title">工单处理系统</h1>' +
      '<span class="mf-jq-sub">运维工单 · 2018 存档演示数据 · 接入用户 ' + esc(u.username || '—') + '（' + esc(u.displayName || '—') + '）</span>' +
      '</div>' +
      '<div class="mf-jq-toolbar">' +
      '<input id="mf-jq-search" class="mf-jq-input" type="text" placeholder="搜索工单编号或标题…" value="' + esc(state.keyword) + '">' +
      '<select id="mf-jq-status-filter" class="mf-jq-select">' + statusOptions + '</select>' +
      '<span class="mf-jq-count" id="mf-jq-count"></span>' +
      '<span class="mf-jq-tokenbox">令牌: ' + tokenChip + '</span>' +
      '</div>' +
      '<div class="mf-jq-tablewrap">' +
      '<table id="mf-jq-table" class="mf-jq-table">' +
      '<thead><tr>' + cols + '</tr></thead>' +
      '<tbody id="mf-jq-tbody"></tbody>' +
      '</table>' +
      '</div>' +
      '<div class="mf-jq-cards">' +
      '<div id="mf-jq-context" class="mf-jq-card"></div>' +
      '<div class="mf-jq-card">' +
      '<div class="mf-jq-card-title">外部权限 API（新链路）</div>' +
      '<p class="mf-jq-note">遗留栈同样可携带主应用注入的令牌调用对外菜单接口：</p>' +
      '<p class="mf-jq-note"><button id="mf-jq-btn-menus" class="mf-jq-btn" type="button">拉取本应用菜单</button></p>' +
      '<span id="mf-jq-menu-status" class="mf-jq-menu-status"></span>' +
      '<ul id="mf-jq-menu-list" class="mf-jq-menu-list"></ul>' +
      '</div>' +
      '<div class="mf-jq-card">' +
      '<div class="mf-jq-card-title">沙箱自检</div>' +
      '<ul class="mf-jq-kv">' +
      '<li>沙箱计时器: <b id="mf-jq-tick">0 秒</b>（每秒 +1）</li>' +
      '<li>window.__JQ_APP_MARKER 已写入: <code>' + esc(readMarker()) + '</code></li>' +
      '</ul>' +
      '<p class="mf-jq-note">卸载时本应用解绑 $(document) 事件、清空容器并停止计时器；window 痕迹由主应用沙箱负责回收验证。</p>' +
      '</div>' +
      '<div class="mf-jq-card">' +
      '<div class="mf-jq-card-title">与主应用通信</div>' +
      '<p class="mf-jq-note">调用 props.onNotify(message, variant)，由主应用以 Toast 呈现：</p>' +
      '<p class="mf-jq-note"><button id="mf-jq-btn-notify" class="mf-jq-btn mf-jq-btn-info" type="button">向主应用发通知</button></p>' +
      '<span id="mf-jq-comm-note" class="mf-jq-menu-status"></span>' +
      '</div>' +
      '</div>' +
      '</div>'
  }

  function readMarker() {
    try {
      return String(window.__JQ_APP_MARKER || '')
    } catch (e) {
      return ''
    }
  }

  /* ===== 事件绑定 (遗留风格: $(document) 命名空间委托, 卸载时精确 off) ===== */
  function bindEvents() {
    /* 调试探针: 属性经挂载容器可见(沙箱内 window 对外不可见) */
    try { mountContainer.setAttribute('data-jq-bind', 'start:' + $().jquery) } catch (e) {}
    $(document)
      .off('.mfjq')
      .on('click.mfjq', '.mf-jq-sort', function () {
        try { mountContainer.setAttribute('data-jq-fired', String(Date.now())) } catch (e) {}
        var key = $(this).data('key')
        if (state.sortKey === key) {
          state.sortDir = state.sortDir === 'asc' ? 'desc' : 'asc'
        } else {
          state.sortKey = key
          state.sortDir = 'asc'
        }
        renderRows()
      })
      .on('keyup.mfjq change.mfjq', '#mf-jq-search', function () {
        state.keyword = $(this).val() || ''
        renderRows()
      })
      .on('change.mfjq', '#mf-jq-status-filter', function () {
        state.status = $(this).val() || ''
        renderRows()
      })
      .on('click.mfjq', '#mf-jq-btn-menus', fetchMenus)
      .on('click.mfjq', '#mf-jq-btn-notify', function () {
        var fn = currentProps && typeof currentProps.onNotify === 'function' ? currentProps.onNotify : null
        if (fn) {
          fn('来自 jQuery 遗留系统的消息', 'info')
          setStatus('#mf-jq-comm-note', '已发送: 来自 jQuery 遗留系统的消息 (info)', true)
        } else {
          setStatus('#mf-jq-comm-note', '发送失败: 主应用未提供 onNotify 回调', false)
        }
      })
    try { mountContainer.setAttribute('data-jq-bind', 'bound-ok') } catch (e) {}
  }

  /* ===== 生命周期契约 ===== */
  window.__MICRO_APP__ = {
    name: 'micro-jquery',
    sdkVersion: window.MicroSdk && window.MicroSdk.sdkVersion,
    mount: function (container, props) {
      return new Promise(function (resolve, reject) {
        try {
          if (!container) {
            reject(new Error('mount 需要一个 container HTMLElement'))
            return
          }
          if (!$) {
            reject(new Error('jQuery 全局库未加载 (缺少 /vendor/jquery.min.js)'))
            return
          }
          stopContext()
          currentProps = null
          if ($) $(document).off('.mfjq')
          if (tickTimer) { window.clearInterval(tickTimer); tickTimer = null }
          if (mountContainer) mountContainer.innerHTML = ''
          applyContext(props)
          if (container.setAttribute) container.setAttribute('data-sdk-version', contextClient.sdkVersion)
          // 沙箱清理探针: 刻意写在真实 window 上, 卸载时不由本应用删除
          window.__JQ_APP_MARKER = 'written-by-jq-subapp'
          mountContainer = container
          state = { sortKey: 'id', sortDir: 'asc', keyword: '', status: '' }
          tickSeconds = 0
          updateCount = 0
          container.innerHTML = '' // 清空重建
          renderShell()
          renderContext()
          renderRows()
          bindEvents()
          // 沙箱自检计时器: 用 window.setInterval 便于主应用沙箱跟踪/回收
          tickTimer = window.setInterval(function () {
            tickSeconds += 1
            var el = find('#mf-jq-tick')
            if (el) el.textContent = tickSeconds + ' 秒'
            if (!contextUsable()) {
              clearResults()
              renderContext()
            }
          }, 1000)
          resolve()
        } catch (err) {
          stopContext()
          currentProps = null
          if ($) $(document).off('.mfjq')
          if (tickTimer) { window.clearInterval(tickTimer); tickTimer = null }
          if (mountContainer) mountContainer.innerHTML = ''
          mountContainer = null
          reject(err)
        }
      })
    },
    unmount: function () {
      return new Promise(function (resolve) {
        stopContext()
        try {
          if ($) $(document).off('.mfjq') // 解绑全部命名空间事件
          if (tickTimer) {
            window.clearInterval(tickTimer) // 停止计时器
            tickTimer = null
          }
          if (mountContainer) {
            mountContainer.innerHTML = '' // 清空容器
            mountContainer = null
          }
        } catch (e) {
          /* 卸载容错: 无论如何都 resolve, 不阻塞主应用 */
        }
        currentProps = null
        resolve()
      })
    },
    /* 增量更新契约: 路由/主题/菜单/令牌变化时由容器调用, 不重挂。
       遗留系统无响应式框架 —— 手术式只刷新头部路由/令牌 chip 与更新徽章,
       表格/筛选/排序/滚动状态全部保留(即「未重挂」的行为学证据)。 */
    update: function (props) {
      return new Promise(function (resolve, reject) {
        try {
          if (!mountContainer) {
            resolve()
            return
          }
          applyContext(props)
          renderContext()
          var user = currentProps.user
          var sub = find('.mf-jq-sub')
          if (sub) sub.textContent = '运维工单 · 2018 存档演示数据 · 接入用户 ' + user.username + '（' + user.displayName + '）'
          updateCount += 1
          var badge = mountContainer.querySelector('.mf-jq-upd-badge')
          if (badge) {
            badge.textContent = '增量更新 ×' + updateCount
            badge.style.display = ''
            badge.title = '最近一次增量更新 ' + new Date().toLocaleTimeString()
          }
          var route = currentProps.routePath || ''
          var routeEl = mountContainer.querySelector('.mf-jq-route')
          if (routeEl) {
            routeEl.innerHTML = route ? 'routePath: <code>' + esc(route) + '</code>' : ''
          } else if (route) {
            // 极端情形: 头部路由 chip 不存在(旧 DOM) → 追加
            var header = mountContainer.querySelector('.mf-jq-header')
            if (header) {
              var span = document.createElement('span')
              span.className = 'mf-jq-route'
              span.innerHTML = 'routePath: <code>' + esc(route) + '</code>'
              header.appendChild(span)
            }
          }
          var token = typeof currentProps.token === 'string' ? currentProps.token : ''
          var tokenBox = mountContainer.querySelector('.mf-jq-tokenbox')
          if (tokenBox) {
            var chip = token
              ? '<code class="mf-jq-token">' + '应用令牌已注入' + '</code>'
              : '<span class="mf-jq-badge">未携带令牌</span>'
            tokenBox.innerHTML = '令牌: ' + chip
          }
        } catch (e) {
          stopContext()
          currentProps = null
          renderContext()
          reject(e)
          return
        }
        resolve()
      })
    },
  }
})()
