/**
 * micro-vue 子应用 — SaaS Platform 微前端演示 (Vue 3 全局构建, 无构建步骤)
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
 * 视图: props.routePath === '/micro/vue-workshop/security' → 「权限与沙箱」, 否则「总览」。
 *
 * 实现说明:
 * - Vue 库来自 /vendor/vue.global.prod.js (先于本文件加载)。该全局构建以 top-level
 *   `var Vue = ...` 暴露; 沙箱 with(proxy)+has()→true 执行模式下该 var 经 with 对象
 *   写入代理 (实证), 故优先取 window.Vue, 兜底裸标识符 —— 两种执行模型都能工作。
 * - 全部组件使用 render 函数 (h 闭包引用), 不使用 template 字符串:
 *   Vue 全局构建的运行时模板编译器会用 Function(code)() 在「真实全局作用域」执行
 *   生成代码并引用裸标识符 Vue —— 在防污染沙箱中真实 window 没有 Vue 会抛
 *   ReferenceError(沙箱模拟实证)。render 函数则完全在沙箱闭包内解析, 双模式可用。
 * - 样式全部手写、选择器一律 .mf-vue- 前缀, 以 <style> 渲染进 mount 容器内,
 *   随 app.unmount() 一并清理, 不向 document.head 泄漏。
 * - fetch('/api/external/menus' | '/api/external/roles') 为同源主应用请求
 *   (Vite 代理 → Rust 后端), Authorization 只使用 props.token 中的应用专用令牌。
 *   本地控制器联动 props.signal；上下文更换/卸载会中止旧请求并拒绝迟到响应。
 * - mount 时向 window 写入 __VUE_APP_MARKER (沙箱清理探针), 卸载时
 *   刻意不清除 —— 该痕迹应由主应用沙箱负责回收, 供其验证「真实 window 无残留」。
 */
;(function () {
  'use strict'

  /* ===== 解析 Vue 全局库 (兼容沙箱的两种 script 执行方式) ===== */
  var VueLib = null
  if (typeof window !== 'undefined' && window.Vue && typeof window.Vue.createApp === 'function') {
    VueLib = window.Vue
  } else if (typeof Vue !== 'undefined' && Vue && typeof Vue.createApp === 'function') {
    VueLib = Vue
  }

  /* ===== 模块级挂载状态 ===== */
  var currentProps = null
  var app = null
  var mountedContainer = null
  var rootVm = null
  var contextController = null
  var hostSignal = null
  var hostAbort = null
  var requestEpoch = 0
  /* 响应式 props 视图: lifecycle.update 增量写入 → 组件自动重渲染(不重挂) */
  var propsState = null

  function ensurePropsState() {
    if (!propsState && VueLib && typeof VueLib.reactive === 'function') {
      propsState = VueLib.reactive({
        user: { id: '', username: '—', displayName: '—' },
        appCode: '—',
        basePath: '—',
        routePath: '',
        theme: '—',
        token: '',
        contractVersion: '',
        contextVersion: '',
        contextEpoch: 0,
        expiresAt: '',
        tokenExpiresAt: '',
        tenant: {},
        organization: {},
        roles: [],
        permissions: [],
        dataGrants: [],
        menus: [],
        contextAvailable: false,
        menusCount: null,
        updates: 0,
        lastUpdateAt: '',
      })
    }
    return propsState
  }

  function syncPropsState(props) {
    var s = ensurePropsState()
    if (!s || !props) return
    var u = props.user || {}
    s.user = {
      id: u.id || '',
      username: u.username || '—',
      displayName: u.displayName || '—',
    }
    s.appCode = props.appCode || '—'
    s.basePath = props.basePath || '—'
    s.routePath = props.routePath || ''
    s.theme = props.theme || '—'
    s.token = typeof props.token === 'string' ? props.token : ''
    s.contractVersion = props.contractVersion
    s.contextVersion = props.contextVersion
    s.contextEpoch = props.contextEpoch
    s.expiresAt = props.expiresAt
    s.tokenExpiresAt = props.tokenExpiresAt
    s.tenant = props.tenant || {}
    s.organization = props.organization || {}
    s.roles = Array.isArray(props.roles) ? props.roles : []
    s.permissions = Array.isArray(props.permissions) ? props.permissions : []
    s.dataGrants = Array.isArray(props.dataGrants) ? props.dataGrants : []
    s.menus = Array.isArray(props.menus) ? props.menus : []
    s.contextAvailable = contextUsable()
    s.menusCount = Array.isArray(props.menus) ? props.menus.length : null
  }

  function emptyResult() {
    return { loading: false, fired: false, status: '', ok: false, message: '', data: null }
  }

  function clearResults() {
    if (rootVm) {
      rootVm.menus = emptyResult()
      rootVm.roles = emptyResult()
    }
  }

  function contextUsable() {
    var p = currentProps
    return !!(p && p.contractVersion === '2.0' && p.token && contextController &&
      !contextController.signal.aborted && Date.parse(p.expiresAt) > Date.now() &&
      Date.parse(p.tokenExpiresAt) > Date.now())
  }

  function stopContext() {
    requestEpoch += 1
    if (contextController) contextController.abort()
    contextController = null
    if (hostSignal && hostAbort) hostSignal.removeEventListener('abort', hostAbort)
    hostSignal = null
    hostAbort = null
    if (propsState) propsState.contextAvailable = false
    clearResults()
  }

  function applyContext(props) {
    if (!props || props.contractVersion !== '2.0' || !props.signal || !props.tenant || !props.organization || !props.user) {
      throw new Error('微应用需要 MicroProps 2.0 完整上下文和 AbortSignal')
    }
    var old = currentProps
    var changed = !old || ['token', 'contextVersion', 'contextEpoch', 'expiresAt', 'tokenExpiresAt', 'signal', 'appCode'].some(function (key) {
      return old[key] !== props[key]
    }) || old.tenant.id !== props.tenant.id || old.organization.id !== props.organization.id || old.user.id !== props.user.id
    if (changed) {
      stopContext()
      currentProps = props
      contextController = new AbortController()
      hostSignal = props.signal
      hostAbort = function () {
        requestEpoch += 1
        if (contextController) contextController.abort()
        if (propsState) propsState.contextAvailable = false
        clearResults()
      }
      hostSignal.addEventListener('abort', hostAbort, { once: true })
      if (hostSignal.aborted) hostAbort()
    } else {
      currentProps = props
    }
    syncPropsState(props)
  }

  function requestExternal(endpoint) {
    if (!contextUsable()) return Promise.resolve({ epoch: requestEpoch, ok: false, status: '—', body: { message: '应用上下文已失效，等待宿主更新' } })
    var epoch = requestEpoch
    var controller = contextController
    var token = currentProps.token
    function isCurrent() {
      return epoch === requestEpoch && controller === contextController && contextUsable()
    }
    return fetch(endpoint, { signal: controller.signal, headers: { Authorization: 'Bearer ' + token } })
      .then(function (res) {
        return res.json().catch(function () { return {} }).then(function (body) {
          return isCurrent() ? { epoch: epoch, ok: res.ok, status: String(res.status), body: body } : null
        })
      })
      .catch(function (err) {
        if (!isCurrent() || (err && err.name === 'AbortError')) return null
        return { epoch: epoch, ok: false, status: '—', body: { message: '网络错误: ' + (err && err.message ? err.message : String(err)) } }
      })
  }
  function normalizePath(p) {
    var s = String(p || '')
    if (s.length > 1) s = s.replace(/\/+$/, '')
    return s
  }

  function readMarker() {
    try {
      return String(window.__VUE_APP_MARKER || '')
    } catch (e) {
      return ''
    }
  }

  function pad2(n) {
    return (n < 10 ? '0' : '') + n
  }

  function formatTime(d) {
    return pad2(d.getHours()) + ':' + pad2(d.getMinutes()) + ':' + pad2(d.getSeconds())
  }

  function typeClass(t) {
    return String(t || '').toLowerCase()
  }

  /* ===== 手写样式 (全部 .mf-vue- 前缀, 白底/灰边框 #e5e7eb/圆角 8px) ===== */
  var CSS = [
    '.mf-vue-root{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC","Hiragino Sans GB","Microsoft YaHei",sans-serif;font-size:13px;line-height:1.65;color:#1f2937;background:#ffffff;border:1px solid #e5e7eb;border-radius:8px;padding:16px;box-sizing:border-box;}',
    '.mf-vue-root *,.mf-vue-root *::before,.mf-vue-root *::after{box-sizing:border-box;}',
    '.mf-vue-header{display:flex;flex-wrap:wrap;align-items:center;gap:8px;padding-bottom:12px;border-bottom:1px solid #e5e7eb;margin-bottom:14px;}',
    '.mf-vue-badge{display:inline-flex;align-items:center;padding:1px 9px;border:1px solid #e5e7eb;border-radius:999px;font-size:11px;line-height:18px;background:#f9fafb;color:#4b5563;white-space:nowrap;font-weight:500;}',
    '.mf-vue-badge-vue{background:#42b883;border-color:#42b883;color:#ffffff;}',
    '.mf-vue-badge-muted{background:#f3f4f6;border-color:#e5e7eb;color:#9ca3af;}',
    '.mf-vue-badge-upd{background:#ecfdf5;border-color:#a7f3d0;color:#047857;}',
    '.mf-vue-routechip{margin-left:auto;font-size:12px;color:#6b7280;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:100%;}',
    '.mf-vue-routechip code{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,"Liberation Mono",monospace;font-size:11px;background:#f3f4f6;border-radius:4px;padding:1px 6px;color:#374151;}',
    '.mf-vue-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:12px;}',
    '.mf-vue-card{border:1px solid #e5e7eb;border-radius:8px;padding:14px;background:#ffffff;min-width:0;}',
    '.mf-vue-card-head{display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:8px;margin-bottom:10px;}',
    '.mf-vue-card-title{font-weight:600;font-size:13px;color:#111827;}',
    '.mf-vue-kv-list{display:grid;gap:6px;}',
    '.mf-vue-kv{display:flex;align-items:baseline;gap:10px;min-width:0;}',
    '.mf-vue-kv>span:first-child{flex:0 0 150px;color:#6b7280;font-size:12px;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;}',
    '.mf-vue-kv>b{font-weight:500;color:#111827;word-break:break-all;min-width:0;}',
    '.mf-vue-muted{color:#9ca3af;font-weight:400;}',
    '.mf-vue-root i.mf-vue-muted{font-style:normal;color:#9ca3af;margin-left:6px;font-size:12px;font-weight:400;}',
    '.mf-vue-token{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:12px;background:#f3f4f6;border:1px solid #e5e7eb;border-radius:4px;padding:1px 6px;color:#374151;}',
    '.mf-vue-btn{appearance:none;-webkit-appearance:none;border:1px solid #d1d5db;background:#ffffff;color:#374151;border-radius:6px;padding:5px 12px;font-size:12px;line-height:20px;cursor:pointer;transition:border-color .15s,color .15s,background-color .15s;font-family:inherit;white-space:nowrap;}',
    '.mf-vue-btn:hover:not(:disabled){border-color:#42b883;color:#2f9e6e;}',
    '.mf-vue-btn:active:not(:disabled){background:#f0fdf4;}',
    '.mf-vue-btn:disabled{opacity:.55;cursor:not-allowed;}',
    '.mf-vue-btn-primary{background:#42b883;border-color:#42b883;color:#ffffff;}',
    '.mf-vue-btn-primary:hover:not(:disabled){background:#38a373;border-color:#38a373;color:#ffffff;}',
    '.mf-vue-status{font-size:12px;color:#6b7280;margin-bottom:8px;word-break:break-all;}',
    '.mf-vue-ok{color:#059669;font-weight:600;}',
    '.mf-vue-err{color:#dc2626;font-weight:600;}',
    '.mf-vue-error{margin:0 0 8px;padding:7px 10px;border:1px solid #fecaca;background:#fef2f2;color:#b91c1c;border-radius:6px;font-size:12px;word-break:break-all;}',
    '.mf-vue-menu-list{list-style:none;margin:0;padding:0;max-height:300px;overflow-y:auto;}',
    '.mf-vue-menu-list .mf-vue-menu-list{margin-top:2px;border-left:1px dashed #e5e7eb;max-height:none;overflow:visible;}',
    '.mf-vue-menu-node{padding:1px 0;}',
    '.mf-vue-menu-line{display:flex;flex-wrap:wrap;align-items:center;gap:6px;padding:2px 0;}',
    '.mf-vue-type{font-size:10px;line-height:16px;padding:0 6px;border-radius:999px;}',
    '.mf-vue-type-dir{background:#ecfdf5;border-color:#a7f3d0;color:#047857;}',
    '.mf-vue-type-menu{background:#f3f4f6;border-color:#d1d5db;color:#374151;}',
    '.mf-vue-type-button{background:#fffbeb;border-color:#fde68a;color:#b45309;}',
    '.mf-vue-menu-title{font-weight:500;color:#111827;}',
    '.mf-vue-menu-path{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;color:#6b7280;background:#f9fafb;padding:0 5px;border-radius:4px;}',
    '.mf-vue-menu-perm{font-size:11px;color:#9ca3af;}',
    '.mf-vue-org{padding:8px 0;border-bottom:1px dashed #e5e7eb;}',
    '.mf-vue-org:last-child{border-bottom:none;padding-bottom:0;}',
    '.mf-vue-org-name{font-weight:600;font-size:12px;color:#111827;margin-bottom:4px;}',
    '.mf-vue-role-list{list-style:none;margin:0;padding:0 0 0 4px;display:grid;gap:3px;}',
    '.mf-vue-role-list li{display:flex;flex-wrap:wrap;align-items:center;gap:6px;font-size:12px;}',
    '.mf-vue-role-list b{font-weight:500;color:#111827;}',
    '.mf-vue-note{margin:8px 0 0;color:#6b7280;font-size:12px;}',
    '.mf-vue-log{list-style:none;margin:10px 0 0;padding:0;display:grid;gap:4px;max-height:140px;overflow-y:auto;}',
    '.mf-vue-log li{font-size:11px;color:#6b7280;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;background:#f9fafb;border-radius:4px;padding:3px 8px;}',
    '.mf-vue-tick{font-variant-numeric:tabular-nums;}',
  ].join('\n')

  /* ===== 递归菜单树组件 (render 函数, h 闭包引用, 递归直接引用组件对象) ===== */
  var MenuTree = {
    name: 'MenuTree',
    props: {
      nodes: { type: Array, default: function () { return [] } },
      depth: { type: Number, default: 0 },
    },
    render: function () {
      var h = VueLib.h
      var depth = this.depth || 0
      var nodes = this.nodes || []
      return h('ul', { class: 'mf-vue-menu-list' }, nodes.map(function (n) {
        var line = h('div', { class: 'mf-vue-menu-line', style: { marginLeft: depth * 16 + 'px' } }, [
          h('span', { class: 'mf-vue-badge mf-vue-type mf-vue-type-' + typeClass(n.type) }, String(n.type || '')),
          h('span', { class: 'mf-vue-menu-title' }, String(n.title || '')),
          n.path ? h('code', { class: 'mf-vue-menu-path' }, String(n.path)) : null,
          n.permission ? h('span', { class: 'mf-vue-menu-perm' }, String(n.permission)) : null,
        ])
        var children = [line]
        if (n.children && n.children.length) {
          children.push(h(MenuTree, { nodes: n.children, depth: depth + 1 }))
        }
        return h('li', { class: 'mf-vue-menu-node', key: n.id }, children)
      }))
    },
  }

  /* ===== 根组件 (props 走响应式 propsState, update 增量写入不重挂; render 函数, 无运行时模板编译) ===== */
  function makeRootComponent() {
    return {
      data: function () {
        ensurePropsState()
        return {
          version: (VueLib && VueLib.version) || '3.x',
          vueMarker: readMarker(),
          seconds: 0,
          p: propsState,
          menus: { loading: false, fired: false, status: '', ok: false, message: '', data: null },
          roles: { loading: false, fired: false, status: '', ok: false, message: '', data: null },
          notifyLog: [],
        }
      },
      computed: {
        routePath: function () {
          return this.p.routePath || '(空)'
        },
        view: function () {
          return normalizePath(this.p.routePath) === '/micro/vue-workshop/security' ? 'security' : 'overview'
        },
        tokenPreview: function () {
          return this.p.token ? this.p.token.slice(0, 8) + '…' : ''
        },
      },
      mounted: function () {
        var vm = this
        // 沙箱自检计时器: 每秒 +1; 用 window.setInterval 便于主应用沙箱跟踪/回收
        this._timer = window.setInterval(function () {
          vm.seconds += 1
          if (!contextUsable()) {
            propsState.contextAvailable = false
            clearResults()
          }
        }, 1000)
      },
      beforeUnmount: function () {
        if (this._timer) {
          window.clearInterval(this._timer)
          this._timer = null
        }
      },
      methods: {
        typeClass: function (t) {
          return typeClass(t)
        },
        clientLabel: function (c) {
          if (!c) return ''
          var parts = [c.clientId || '?']
          parts.push(c.tokenType === 'client_credentials' ? '客户端凭据令牌' : '用户令牌')
          if (c.username) parts.push('用户: ' + c.username)
          return parts.join(' · ')
        },
        requestExternal: function (endpoint) {
          return requestExternal(endpoint)
        },
        loadMenus: function () {
          var vm = this
          vm.menus = { loading: true, fired: true, status: '', ok: false, message: '', data: null }
          return vm.requestExternal('/api/external/menus').then(function (r) {
            if (!r || r.epoch !== requestEpoch || !mountedContainer) return
            vm.menus = {
              loading: false,
              fired: true,
              status: r.status,
              ok: r.ok,
              message: (r.body && r.body.message) || '',
              data: r.ok ? r.body : null,
            }
          })
        },
        loadRoles: function () {
          var vm = this
          vm.roles = { loading: true, fired: true, status: '', ok: false, message: '', data: null }
          return vm.requestExternal('/api/external/roles').then(function (r) {
            if (!r || r.epoch !== requestEpoch || !mountedContainer) return
            vm.roles = {
              loading: false,
              fired: true,
              status: r.status,
              ok: r.ok,
              message: (r.body && r.body.message) || '',
              data: r.ok ? r.body : null,
            }
          })
        },
        sendNotify: function () {
          var fn = currentProps && typeof currentProps.onNotify === 'function' ? currentProps.onNotify : null
          var stamp = formatTime(new Date())
          if (!fn) {
            this.notifyLog.unshift('[' + stamp + '] 发送失败: 主应用未提供 onNotify 回调')
          } else {
            fn('来自 Vue 子应用的消息', 'success')
            this.notifyLog.unshift('[' + stamp + '] 已发送: 来自 Vue 子应用的消息 (success)')
          }
          if (this.notifyLog.length > 5) this.notifyLog.length = 5
        },
        goOverview: function () {
          var fn = currentProps && typeof currentProps.onNavigate === 'function' ? currentProps.onNavigate : null
          if (fn) fn('/micro/vue-workshop')
        },
      /* ---------- 总览视图 ---------- */
      renderOverview: function () {
        var h = VueLib.h
        var vm = this
        var cards = []

        /* ① props 注入面板 */
        var kv = function (k, valueNode) {
          return h('div', { class: 'mf-vue-kv' }, [h('span', null, k), h('b', null, valueNode)])
        }
        var tokenValue = this.p.token
          ? h('code', { class: 'mf-vue-token' }, this.tokenPreview)
          : h('span', { class: 'mf-vue-badge mf-vue-badge-muted' }, '未携带令牌')
        cards.push(
          h('section', { class: 'mf-vue-card' }, [
            h('div', { class: 'mf-vue-card-title' }, '① 主应用注入的 props'),
            h('div', { class: 'mf-vue-kv-list' }, [
              kv('user.username', this.p.user.username),
              kv('user.displayName', this.p.user.displayName),
              kv('appCode', this.p.appCode),
              kv('basePath', this.p.basePath),
              kv('routePath', this.routePath),
              kv('theme', [this.p.theme, h('i', { class: 'mf-vue-muted' }, '（本子应用不做暗色适配）')]),
              kv('token', tokenValue),
              kv('contractVersion', this.p.contractVersion),
              kv('contextVersion', this.p.contextVersion),
              kv('contextEpoch', String(this.p.contextEpoch)),
              kv('tenant', this.p.tenant.name + ' · ' + this.p.tenant.code),
              kv('organization', this.p.organization.name + ' · ' + this.p.organization.path),
              kv('expiresAt', this.p.expiresAt),
              kv('tokenExpiresAt', this.p.tokenExpiresAt),
              kv('context', this.p.contextAvailable ? '有效' : '已失效，等待宿主更新'),
              this.p.menusCount !== null ? kv('menus', '注入 ' + this.p.menusCount + ' 项') : null,
            ]),
          ])
        )

        cards.push(h('section', { class: 'mf-vue-card' }, [
          h('div', { class: 'mf-vue-card-title' }, '当前用户的角色与操作权限'),
          h('p', { class: 'mf-vue-note' }, this.p.contextAvailable ? this.p.roles.map(function (role) { return role.name + ' (' + role.code + ')' }).join('、') || '没有应用角色' : '上下文已失效'),
          h('p', { class: 'mf-vue-note' }, this.p.contextAvailable ? this.p.permissions.join('、') || '没有操作权限' : ''),
          h('div', { class: 'mf-vue-card-title' }, '独立数据授权'),
          h('ul', { class: 'mf-vue-role-list' }, this.p.contextAvailable ? this.p.dataGrants.map(function (grant) {
            return h('li', { key: grant.resource + ':' + grant.action }, [
              h('b', null, grant.resource + ':' + grant.action + ' · ' + grant.scope),
              h('span', { class: 'mf-vue-muted' }, '用户 ' + grant.subjectUserId + ' · 租户 ' + grant.tenantId + ' · 组织 ' + (grant.organizationIds || []).join(', ')),
            ])
          }) : []),
          h('p', { class: 'mf-vue-note' }, '没有显式数据授权时默认拒绝。业务后端必须同时校验操作权限和资源对应的数据范围。'),
          this.p.contextAvailable ? h(MenuTree, { nodes: this.p.menus, depth: 0 }) : null,
        ]))

        /* ② 拉取本应用菜单 */
        var m = this.menus
        var menusChildren = [
          h('div', { class: 'mf-vue-card-head' }, [
            h('div', { class: 'mf-vue-card-title' }, '② 拉取本应用菜单'),
            h(
              'button',
              { class: 'mf-vue-btn', type: 'button', disabled: m.loading || !this.p.contextAvailable, onClick: function () { vm.loadMenus() } },
              m.loading ? '请求中…' : 'GET /api/external/menus'
            ),
          ]),
        ]
        if (m.fired) {
          var mStatus = ['HTTP ', h('b', { class: m.ok ? 'mf-vue-ok' : 'mf-vue-err' }, m.status)]
          if (m.ok && m.data && m.data.application) mStatus.push(' · 应用「' + m.data.application.name + '」')
          if (m.ok && m.data && this.clientLabel(m.data.client)) mStatus.push(' · ' + this.clientLabel(m.data.client))
          menusChildren.push(h('div', { class: 'mf-vue-status' }, mStatus))
        }
        if (m.fired && !m.loading && !m.ok) {
          var mErr = [m.message || '请求失败']
          if (!this.p.token) mErr.push(h('i', { class: 'mf-vue-muted' }, '（主应用未注入应用 token）'))
          menusChildren.push(h('p', { class: 'mf-vue-error' }, mErr))
        }
        if (m.ok && m.data && m.data.menus && m.data.menus.length) {
          menusChildren.push(h(MenuTree, { nodes: m.data.menus, depth: 0 }))
        } else if (m.ok) {
          menusChildren.push(h('p', { class: 'mf-vue-note mf-vue-muted' }, '该应用没有启用中的菜单。'))
        }
        cards.push(h('section', { class: 'mf-vue-card' }, menusChildren))

        /* ③ 拉取本应用角色 */
        var r = this.roles
        var rolesChildren = [
          h('div', { class: 'mf-vue-card-head' }, [
            h('div', { class: 'mf-vue-card-title' }, '③ 拉取本应用角色'),
            h(
              'button',
              { class: 'mf-vue-btn', type: 'button', disabled: r.loading || !this.p.contextAvailable, onClick: function () { vm.loadRoles() } },
              r.loading ? '请求中…' : 'GET /api/external/roles'
            ),
          ]),
        ]
        if (r.fired) {
          var rStatus = ['HTTP ', h('b', { class: r.ok ? 'mf-vue-ok' : 'mf-vue-err' }, r.status)]
          if (r.ok && r.data && r.data.application) rStatus.push(' · 应用「' + r.data.application.name + '」')
          rolesChildren.push(h('div', { class: 'mf-vue-status' }, rStatus))
        }
        if (r.fired && !r.loading && !r.ok) {
          var rErr = [r.message || '请求失败']
          if (!this.p.token) rErr.push(h('i', { class: 'mf-vue-muted' }, '（主应用未注入应用 token）'))
          rolesChildren.push(h('p', { class: 'mf-vue-error' }, rErr))
        }
        if (r.ok && r.data) {
          var orgs = r.data.organizations || []
          var orgNodes = orgs.map(function (org) {
            var roleLis = (org.roles || []).map(function (role) {
              return h('li', { key: role.id }, [
                h('b', null, role.name),
                h('span', { class: 'mf-vue-muted' }, role.code),
              ])
            })
            if (!roleLis.length) roleLis.push(h('li', { class: 'mf-vue-muted' }, '（无启用中的角色）'))
            return h('div', { class: 'mf-vue-org', key: org.id }, [
              h('div', { class: 'mf-vue-org-name' }, [
                org.name + ' ',
                h('span', { class: 'mf-vue-muted' }, '· ' + this.p.tenant.name),
              ]),
              h('ul', { class: 'mf-vue-role-list' }, roleLis),
            ])
          }, this)
          if (!orgNodes.length) {
            orgNodes.push(h('p', { class: 'mf-vue-note mf-vue-muted' }, '当前用户在当前组织没有应用角色。'))
          }
          rolesChildren.push(h('div', null, orgNodes))
        }
        cards.push(h('section', { class: 'mf-vue-card' }, rolesChildren))

        /* ④ 与主应用通信 */
        cards.push(
          h('section', { class: 'mf-vue-card' }, [
            h('div', { class: 'mf-vue-card-title' }, '④ 与主应用通信'),
            h('p', { class: 'mf-vue-note' }, '点击按钮调用 props.onNotify(message, variant)，由主应用以 Toast 呈现：'),
            h('p', { class: 'mf-vue-note' }, [
              h('button', { class: 'mf-vue-btn mf-vue-btn-primary', type: 'button', onClick: function () { vm.sendNotify() } }, '向主应用发通知'),
            ]),
            this.notifyLog.length
              ? h('ul', { class: 'mf-vue-log' }, this.notifyLog.map(function (item, i) {
                  return h('li', { key: i }, item)
                }))
              : null,
          ])
        )

        return h('div', { class: 'mf-vue-grid' }, cards)
      },
      /* ---------- 权限与沙箱视图 ---------- */
      renderSecurity: function () {
        var h = VueLib.h
        var vm = this
        return h('div', { class: 'mf-vue-grid' }, [
          h('section', { class: 'mf-vue-card' }, [
            h('div', { class: 'mf-vue-card-title' }, '① 沙箱自检'),
            h('div', { class: 'mf-vue-kv-list' }, [
              h('div', { class: 'mf-vue-kv' }, [
                h('span', null, '沙箱计时器'),
                h('b', { class: 'mf-vue-tick' }, this.seconds + ' 秒（每秒 +1）'),
              ]),
              h('div', { class: 'mf-vue-kv' }, [
                h('span', null, 'window.__VUE_APP_MARKER 已写入'),
                h('b', null, h('code', { class: 'mf-vue-token' }, this.vueMarker || '(读取失败)')),
              ]),
            ]),
            h('p', { class: 'mf-vue-note' }, 'mount 时本应用向 window 写入 __VUE_APP_MARKER 并读回展示；计时器自挂载起每秒 +1。卸载后主应用可验证真实 window 上无此痕迹、计时器停止。'),
          ]),
          h('section', { class: 'mf-vue-card' }, [
            h('div', { class: 'mf-vue-card-title' }, '② 导航演示 · 增量更新'),
            h('p', { class: 'mf-vue-note' }, '调用 props.onNavigate(\'/micro/vue-workshop\') 交由主应用路由切回总览：本应用已实现 lifecycle.update 契约，路由变化只增量推送新 props，不重挂载——注意计时器持续走秒、按钮点击等内部状态全保留。'),
            h('p', { class: 'mf-vue-note' }, [
              h('button', { class: 'mf-vue-btn', type: 'button', onClick: function () { vm.goOverview() } }, '切换到总览'),
            ]),
          ]),
        ])
      },
      },
      render: function () {
        var h = VueLib.h
        var header = h('header', { class: 'mf-vue-header' }, [
          h('span', { class: 'mf-vue-badge mf-vue-badge-vue' }, 'Vue ' + this.version),
          h('span', { class: 'mf-vue-badge' }, '微前端子应用 · HTML 入口 + 沙箱'),
          this.p.updates > 0
            ? h(
                'span',
                { class: 'mf-vue-badge mf-vue-badge-upd', title: '最近一次增量更新 ' + this.p.lastUpdateAt },
                '增量更新 ×' + this.p.updates + (this.p.lastUpdateAt ? ' · ' + this.p.lastUpdateAt : '')
              )
            : null,
          h('span', { class: 'mf-vue-routechip' }, ['routePath: ', h('code', null, this.routePath)]),
        ])
        var body = this.view === 'security' ? this.renderSecurity() : this.renderOverview()
        return h('div', { class: 'mf-vue-root' }, [h('style', null, CSS), header, body])
      },
    }
  }

  /* ===== 生命周期契约 ===== */
  window.__MICRO_APP__ = {
    name: 'micro-vue',
    mount: function (container, props) {
      return new Promise(function (resolve, reject) {
        try {
          if (!container) {
            reject(new Error('mount 需要一个 container HTMLElement'))
            return
          }
          if (!VueLib) {
            reject(new Error('Vue 全局库未加载 (缺少 /vendor/vue.global.prod.js)'))
            return
          }
          stopContext()
          currentProps = null
          // 沙箱清理探针: 刻意写在 window 上(沙箱内即代理自有表), 卸载时不由本应用删除
          window.__VUE_APP_MARKER = 'written-by-vue-subapp'
          if (app) {
            try {
              app.unmount()
            } catch (e) {
              /* 忽略上次卸载残留 */
            }
            app = null
          }
          rootVm = null
          applyContext(props)
          if (propsState) {
            propsState.updates = 0
            propsState.lastUpdateAt = ''
          }
          app = VueLib.createApp(makeRootComponent())
          rootVm = app.mount(container)
          mountedContainer = container
          resolve()
        } catch (err) {
          stopContext()
          currentProps = null
          if (app) { try { app.unmount() } catch (e) {} }
          app = null
          rootVm = null
          if (container) container.innerHTML = ''
          mountedContainer = null
          reject(err)
        }
      })
    },
    unmount: function () {
      return new Promise(function (resolve) {
        stopContext()
        if (rootVm && rootVm._timer) {
          window.clearInterval(rootVm._timer)
          rootVm._timer = null
        }
        try {
          if (app) {
            app.unmount() // beforeUnmount 钩子中清理 setInterval
            app = null
          }
        } catch (e) {
          /* 卸载容错: 无论如何都 resolve, 不阻塞主应用 */
        }
        app = null
        rootVm = null
        if (mountedContainer) mountedContainer.innerHTML = ''
        mountedContainer = null
        currentProps = null
        resolve()
      })
    },
    /* 增量更新契约: 路由/主题/菜单/令牌变化时由容器调用, 不重挂。
       写入响应式 propsState → 组件自动重渲染; seconds/notifyLog 等组件内部状态保留。 */
    update: function (props) {
      return new Promise(function (resolve, reject) {
        try {
          if (!mountedContainer) { resolve(); return }
          applyContext(props)
          if (propsState) {
            propsState.updates += 1
            var d = new Date()
            propsState.lastUpdateAt = pad2(d.getHours()) + ':' + pad2(d.getMinutes()) + ':' + pad2(d.getSeconds())
          }
        } catch (e) {
          stopContext()
          currentProps = null
          reject(e)
          return
        }
        resolve()
      })
    },
  }
})()
