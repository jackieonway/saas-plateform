import assert from 'node:assert/strict'
import test from 'node:test'
import { MessageChannel } from 'node:worker_threads'
import { createIframeLifecycle } from '../src/components/micro/iframe.ts'
import { normalizedRoutePath } from '../src/lib/route-permissions.ts'
import { componentFixture, elements, button, success } from './helpers/component-fixture.mjs'

function contentFixture() {
  return componentFixture('src/components/micro/MicroAppContainer.tsx', [
    'MicroMountContent', 'MountAreaSkeleton', 'MountErrorCard', 'TrustedRuntimeFaultCard',
  ], {
    './runtime': { trustedMicroRuntimeDomain: { fault: null } },
    './sandbox': {}, './loader': {}, './iframe': {}, './entry': {}, './types': {},
    '../../../vendor/micro-app-sdk/generated/protocol.mjs': {},
    '../../../vendor/micro-app-sdk/releases/1.0.0/index.mjs': {},
  })
}

const contentProps = extra => ({ entryName: '订单管理', status: 'mounted', loading: false, fault: null,
  hasRuntimeToken: true, errorMessage: null, containerRef: { current: null }, onRetry() {}, ...extra })

test('mounted application renders only the full-width empty business mount point', () => {
  const f = contentFixture()
  const tree = f.render('MicroMountContent', contentProps())
  const mount = elements(tree, item => item.props?.['aria-label'] === '订单管理 挂载区')[0]
  assert.ok(mount)
  assert.equal(mount.props['aria-busy'], false)
  assert.equal(mount.props.children, undefined, 'React must not own the runtime mount children')
  assert.match(tree.props.className, /w-full/)
  assert.match(mount.props.className, /w-full/)
  assert.doesNotMatch(tree.props.className, /border|rounded|space-y|p-\d/)
  assert.equal(elements(tree, item => item.type === 'Button' || item.type === 'Tooltip' || item.type === 'Popover').length, 0)
  assert.doesNotMatch(JSON.stringify(tree), /令牌已注入|props\.token|micro-status-bar|沙箱诊断|运行时同步/)
})

test('application loading keeps its accessible overlay without adding a permanent toolbar', () => {
  const f = contentFixture()
  const tree = f.render('MicroMountContent', contentProps({ status: 'loading', loading: true }))
  assert.equal(elements(tree, item => item.props?.['aria-busy'] === true).length, 1)
  const overlay = elements(tree, item => item.type?.name === 'MountAreaSkeleton')[0]
  assert.ok(overlay)
  const skeleton = f.render('MountAreaSkeleton', overlay.props)
  assert.equal(skeleton.props.role, 'status')
  assert.equal(skeleton.props['aria-label'], '子应用加载中')
})

test('encoded and case-variant micro entry routes redirect to the registered spelling with query and hash intact', () => {
  const app = { id: 'orders', code: 'orders', name: 'Orders', runtimeMode: 'IFRAME', entryType: 'HTML', entryUrl: '/orders/' }
  for (const pathname of ['/micro/%6frders/detail', '/MICRO/orders/detail', '/micro/ORDERS/detail']) {
    const state = { token: 'session', profile: { applications: [app] }, contextEpoch: 1, currentApplicationId: 'previous-app' }
    const f = componentFixture('src/components/micro/MicroAppContainer.tsx', ['MicroAppContainer'], {
      'react-router-dom': { useParams: () => ({ appCode: pathname.includes('ORDERS') ? 'ORDERS' : 'orders' }),
        useLocation: () => ({ pathname, search: '?status=pending', hash: '#latest' }), useNavigate: () => () => {}, Navigate: 'Navigate' },
      '@/stores/auth': { useAuthStore: selector => selector(state) },
      '@/lib/route-permissions': { normalizedRoutePath },
      './runtime': { trustedMicroRuntimeDomain: { fault: null } },
      './sandbox': {}, './loader': {}, './iframe': {}, './entry': {}, './types': {},
      '../../../vendor/micro-app-sdk/generated/protocol.mjs': {},
      '../../../vendor/micro-app-sdk/releases/1.0.0/index.mjs': {},
    })
    const tree = f.render('MicroAppContainer')
    assert.equal(tree.type, 'Navigate')
    assert.equal(tree.props.to, '/micro/orders/detail?status=pending#latest')
    assert.equal(tree.props.replace, true)
  }
})

test('failed mounting retains the actual retry action and trusted faults reload the host', () => {
  const f = contentFixture()
  let retries = 0, reloads = 0
  const errorTree = f.render('MicroMountContent', contentProps({ status: 'error', errorMessage: '授权获取失败', onRetry() { retries++ } }))
  const error = elements(errorTree, item => item.type?.name === 'MountErrorCard')[0]
  assert.ok(error)
  const card = f.render('MountErrorCard', error.props)
  button(card, '重试').props.onClick()
  assert.equal(retries, 1)
  assert.match(JSON.stringify(card), /授权获取失败/)
  const faultTree = f.render('MicroMountContent', contentProps({ fault: { stage: 'mount' }, status: 'error' }))
  const fault = elements(faultTree, item => item.type?.name === 'TrustedRuntimeFaultCard')[0]
  const oldWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { location: { reload() { reloads++ } } } })
  try {
    const faultCard = f.render('TrustedRuntimeFaultCard', fault.props)
    button(faultCard, '重新加载宿主').props.onClick()
    assert.equal(reloads, 1)
  } finally {
    if (oldWindow) Object.defineProperty(globalThis, 'window', oldWindow)
    else delete globalThis.window
  }
})

function layout(pathname) {
  const profile = { user: { id: 'u', username: 'User' }, tenant: { id: 't' }, organization: { id: 'o' }, applications: [] }
  const state = { token: 'session', profile, contextEpoch: 1, currentApplicationId: null }
  const react = { lazy: () => () => null, useLayoutEffect() {}, useEffect() {},
    useRef: value => ({ current: value }), useMemo: callback => callback(), useCallback: callback => callback,
    useState: value => [typeof value === 'function' ? value() : value, () => {}] }
  const f = componentFixture('src/components/layout/AppLayout.tsx', ['AppLayout'], {
    react,
    'react-router-dom': { useLocation: () => ({ pathname, search: '', key: pathname }), Outlet: 'Outlet', Navigate: 'Navigate' },
    '@/stores/auth': { useAuthStore: selector => selector(state) },
    '@/hooks/use-platform-branding': { usePlatformBranding: () => ({ platformName: '平台' }) },
    '@/hooks/use-auth-data': { useProfileQuery: () => success(profile), useMenusQuery: () => success([]) },
    '@/hooks/use-application-navigation': { useApplicationNavigation: () => ({ selectApplication() {}, access: () => ({ status: 'allowed' }) }) },
    '@/hooks/use-document-title': { useDocumentTitle() {}, pageTitleFor: () => '页面' },
  })
  return f.render('AppLayout')
}

test('only actual micro routes remove the host padding and width cap while preserving navigation', () => {
  for (const path of ['/micro/orders', '/micro/orders/detail/1']) {
    const tree = layout(path)
    const main = elements(tree, item => item.type === 'main')[0]
    assert.match(main.props.className, /\bp-0\b/)
    const content = elements(tree, item => item.type === 'motion.div')[0]
    assert.equal(content.props.className, 'min-h-full w-full')
    assert.equal(elements(tree, item => item.type === 'TopBar').length, 1)
    assert.equal(elements(tree, item => item.type === 'AppRail').length, 1)
    assert.equal(elements(tree, item => item.type === 'RoutePermissionGuard').length, 1)
  }
  for (const path of ['/system/users', '/dashboard', '/micro', '/micro-other/orders']) {
    const tree = layout(path)
    assert.match(elements(tree, item => item.type === 'main')[0].props.className, /p-4 md:p-6/)
    assert.match(elements(tree, item => item.type === 'motion.div')[0].props.className, /max-w-\[1600px\]/)
  }
})

test('the actual iframe mount fills the viewport below the host top bar and preserves isolation', async () => {
  const frames = [], originals = new Map(), channels = []
  const target = { isConnected: true, nodes: [], contains(frame) { return this.nodes.includes(frame) }, replaceChildren(...nodes) { this.nodes = nodes } }
  class Frame {}
  Frame.prototype.sandbox = {}
  class Channel {
    constructor() { const real = new MessageChannel(); this.port1 = real.port1; this.port2 = real.port2; channels.push(this) }
  }
  const document = { documentElement: {}, createElement(tag) {
    assert.equal(tag, 'iframe')
    const frame = { style: {}, attributes: {}, setAttribute(key, value) { this.attributes[key] = value },
      remove() { target.nodes = [] }, contentWindow: { postMessage() {} } }
    frames.push(frame)
    return frame
  } }
  for (const [name, value] of Object.entries({ document, window: { location: { href: 'https://host.test/', origin: 'https://host.test' } },
    HTMLIFrameElement: Frame, MessageChannel: Channel, MutationObserver: class { observe() {} disconnect() {} } })) {
    originals.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
    Object.defineProperty(globalThis, name, { configurable: true, value })
  }
  const lifecycle = createIframeLifecycle({ appCode: 'orders', entryUrl: '/orders/' })
  const props = { contractVersion: '2.0', contextVersion: 'v1', contextEpoch: 1, appCode: 'orders',
    basePath: '/micro/orders', token: 'application-token', signal: new AbortController().signal,
    expiresAt: new Date(Date.now() + 60_000).toISOString(), tokenExpiresAt: new Date(Date.now() + 60_000).toISOString() }
  const mounting = lifecycle.mount(target, props)
  mounting.catch(() => {})
  try {
    for (let turn = 0; turn < 10; turn++) await Promise.resolve()
    assert.equal(frames.length, 1)
    assert.match(frames[0].style.cssText, /width:100%/)
    assert.match(frames[0].style.cssText, /height:max\(480px, calc\(100dvh - 3\.5rem\)\)/)
    assert.equal(frames[0].attributes.sandbox, 'allow-scripts allow-forms')
  } finally {
    lifecycle.unmount()
    await mounting.catch(() => {})
    for (const channel of channels) { channel.port1.close(); channel.port2.close() }
    for (const [name, descriptor] of originals) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor)
      else delete globalThis[name]
    }
  }
})
