import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { createContext, Script } from 'node:vm';
import ts from '../node_modules/typescript/lib/typescript.js';

const source = readFileSync(new URL('../src/components/micro/sandbox.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
}).outputText;

// Browser-shaped targets make event registration identity/capture, timer queues and
// MutationObserver microtask delivery explicit; no app framework or network is needed.
class Target {
  registrations = [];
  addEventListener(type, callback, options = false) {
    const capture = typeof options === 'boolean' ? options : !!options?.capture;
    const signal = typeof options === 'object' ? options?.signal : undefined;
    if (!callback || signal?.aborted || this.registrations.some((record) =>
      record.type === type && record.callback === callback && record.capture === capture)) return;
    const record = { type, callback, capture, once: !!options?.once, signal };
    if (signal) {
      record.abort = () => this.removeEventListener(type, callback, capture);
      signal.addEventListener('abort', record.abort, { once: true });
    }
    this.registrations.push(record);
  }
  removeEventListener(type, callback, options = false) {
    const capture = typeof options === 'boolean' ? options : !!options?.capture;
    this.registrations = this.registrations.filter((record) => {
      const match = record.type === type && record.callback === callback && record.capture === capture;
      if (match && record.signal) record.signal.removeEventListener('abort', record.abort);
      return !match;
    });
  }
  dispatch(type) {
    for (const record of [...this.registrations]) {
      if (record.type !== type || !this.registrations.includes(record)) continue;
      if (record.once) this.removeEventListener(type, record.callback, record.capture);
      if (typeof record.callback === 'function') record.callback.call(this, { type, target: this });
      else record.callback.handleEvent({ type, target: this });
    }
  }
}

function runtime() {
  const observers = new Set();
  class Element {
    nodeType = 1;
    parentNode = null;
    children = [];
    attributes = new Set();
    constructor(tagName, root = false) { this.tagName = tagName.toUpperCase(); this.root = root; }
    get isConnected() { return this.root || !!this.parentNode?.isConnected; }
    hasAttribute(name) { return this.attributes.has(name); }
    querySelectorAll(selector) {
      const tags = selector.toUpperCase().split(',');
      return this.children.flatMap((child) => [
        ...(tags.includes(child.tagName) ? [child] : []), ...child.querySelectorAll(selector),
      ]);
    }
    appendChild(child) {
      if (child.parentNode) child.parentNode.removeChild(child);
      child.parentNode = this;
      this.children.push(child);
      for (const observer of observers) {
        if (observer.targets.some(({ target, options }) => {
          for (let current = this; current; current = current.parentNode) {
            if (current === target) return current === this || options.subtree;
          }
          return false;
        })) observer.records.push({ addedNodes: [child] });
      }
      return child;
    }
    removeChild(child) {
      this.children = this.children.filter((node) => node !== child);
      child.parentNode = null;
      return child;
    }
  }
  class MutationObserver {
    targets = [];
    records = [];
    constructor(callback) { this.callback = callback; observers.add(this); }
    observe(target, options) { observers.add(this); this.targets.push({ target, options }); }
    takeRecords() { const records = this.records; this.records = []; return records; }
    disconnect() { this.targets = []; this.records = []; observers.delete(this); }
  }
  const document = new Target();
  document.head = new Element('HEAD', true);
  document.body = new Element('BODY', true);
  const window = new Target();
  const timers = new Map();
  const frames = new Map();
  let nextTimer = 1;
  let nextFrame = 1;
  window.setTimeout = (callback) => { const id = nextTimer++; timers.set(id, { callback, repeat: false }); return id; };
  window.setInterval = (callback) => { const id = nextTimer++; timers.set(id, { callback, repeat: true }); return id; };
  window.clearTimeout = window.clearInterval = (id) => timers.delete(id);
  window.requestAnimationFrame = (callback) => { const id = nextFrame++; frames.set(id, callback); return id; };
  window.cancelAnimationFrame = (id) => frames.delete(id);
  const exports = {};
  const context = createContext({ window, document, Node: { ELEMENT_NODE: 1 }, MutationObserver, exports, queueMicrotask });
  new Script(compiled, { filename: 'sandbox.js' }).runInContext(context);
  return {
    Sandbox: exports.MicroSandbox, window, document, timers, frames, Element, observers,
    flushMutations() { for (const observer of observers) observer.callback(observer.takeRecords()); },
    fireTimer(id) {
      const timer = timers.get(id);
      if (!timer) return;
      if (!timer.repeat) timers.delete(id);
      timer.callback();
    },
    fireFrame(id) { const callback = frames.get(id); frames.delete(id); callback?.(16); },
  };
}

function assertEmpty(sandbox) {
  const stats = sandbox.getStats();
  assert.equal(stats.timers, 0);
  assert.equal(stats.listeners, 0);
  assert.equal(stats.insertedNodes, 0);
  assert.equal(stats.ownProps, 0);
  assert.equal(stats.tornDown, true);
}

test('run preserves real document identity and restores host methods on success, throw and Promise', async () => {
  const { Sandbox, document } = runtime();
  const sandbox = new Sandbox('jquery');
  const nativeAdd = document.addEventListener;
  const nativeRemove = document.removeEventListener;
  const host = () => {};
  document.addEventListener('host', host);
  sandbox.run(() => {
    assert.equal(sandbox.proxy.document, document);
    document.addEventListener('child', () => {});
    assert.equal(sandbox.getStats().listeners, 1);
  });
  assert.equal(document.addEventListener, nativeAdd);
  assert.equal(document.removeEventListener, nativeRemove);
  assert.equal(Object.hasOwn(document, 'addEventListener'), false);
  assert.throws(() => sandbox.run(() => { throw new Error('mount failed'); }), /mount failed/);
  assert.equal(document.addEventListener, nativeAdd);
  let resolve;
  const promise = sandbox.run(() => new Promise((done) => { resolve = done; }));
  assert.equal(document.addEventListener, nativeAdd);
  resolve();
  await promise;
  sandbox.teardown();
  assert.equal(document.registrations.length, 1);
  assert.equal(document.registrations[0].callback, host);
  assertEmpty(sandbox);
});

test('window and document listeners use native duplicate, capture and remove semantics', () => {
  const { Sandbox, window, document } = runtime();
  const sandbox = new Sandbox('events');
  let calls = 0;
  const listener = () => { calls++; };
  sandbox.proxy.addEventListener('resize', listener, false);
  sandbox.proxy.addEventListener('resize', listener, { capture: false, once: true });
  sandbox.proxy.addEventListener('resize', listener, true);
  sandbox.run(() => document.addEventListener('click', listener, { capture: true }));
  assert.equal(sandbox.getStats().listeners, 3);
  sandbox.proxy.removeEventListener('resize', listener, false);
  assert.equal(sandbox.getStats().listeners, 2);
  assert.equal(window.registrations.length, 1);
  window.dispatch('resize');
  document.dispatch('click');
  assert.equal(calls, 2);
  sandbox.run(() => document.removeEventListener('click', listener, false));
  assert.equal(sandbox.getStats().listeners, 2);
  sandbox.run(() => document.removeEventListener('click', listener, true));
  assert.equal(sandbox.getStats().listeners, 1);
  sandbox.teardown();
  window.dispatch('resize');
  document.dispatch('click');
  assert.equal(calls, 2);
  assert.equal(window.registrations.length + document.registrations.length, 0);
});

test('once and signal registrations stop bookkeeping and retain event callback context', () => {
  const { Sandbox, window, document } = runtime();
  const sandbox = new Sandbox('signals');
  const controller = new AbortController();
  let count = 0;
  const objectListener = { handleEvent() { assert.equal(this, objectListener); count++; } };
  const onceOptions = { once: true };
  sandbox.proxy.addEventListener('once', objectListener, onceOptions);
  onceOptions.once = false; // DOM 注册时读取配置，之后修改不能改变其行为。
  sandbox.proxy.addEventListener('abort', () => { count++; }, { signal: controller.signal });
  const alreadyAborted = AbortSignal.abort();
  sandbox.proxy.addEventListener('ignored', () => {}, { signal: alreadyAborted });
  assert.equal(sandbox.getStats().listeners, 2);
  window.dispatch('once');
  window.dispatch('once');
  assert.equal(sandbox.getStats().listeners, 1);
  controller.abort();
  assert.equal(sandbox.getStats().listeners, 0);
  window.dispatch('abort');
  sandbox.proxy.addEventListener('scope', function () {
    assert.equal(this, sandbox.proxy);
    document.addEventListener('from-window-event', () => {});
  });
  window.dispatch('scope');
  assert.equal(sandbox.getStats().listeners, 2);
  assert.equal(count, 1);
  sandbox.teardown();
  assert.equal(window.registrations.length + document.registrations.length, 0);
});

test('expired timers leave bookkeeping; timer and animation callbacks track document resources', () => {
  const env = runtime();
  const sandbox = new env.Sandbox('timers');
  const timeout = sandbox.proxy.setTimeout(function () {
    assert.equal(this, sandbox.proxy);
    env.document.addEventListener('timeout-listener', () => {});
  });
  const interval = sandbox.proxy.setInterval(() => {});
  const frame = sandbox.proxy.requestAnimationFrame(() => env.document.addEventListener('frame-listener', () => {}));
  assert.equal(sandbox.getStats().timers, 3);
  env.fireTimer(timeout);
  env.fireFrame(frame);
  assert.equal(sandbox.getStats().timers, 1);
  assert.equal(sandbox.getStats().listeners, 2);
  sandbox.proxy.clearTimeout(interval); // 浏览器的 clearTimeout/clearInterval 共用 ID 池。
  assert.equal(sandbox.getStats().timers, 0);
  const stringTimer = sandbox.proxy.setTimeout('window.childValue = 42');
  env.fireTimer(stringTimer);
  assert.equal(sandbox.proxy.childValue, 42);
  assert.equal(env.window.childValue, undefined);
  assert.equal(sandbox.getStats().timers, 0);
  sandbox.teardown();
  assert.equal(env.document.registrations.length, 0);
});

test('teardown cancels resources and saved APIs or queued callbacks cannot recreate them', () => {
  const env = runtime();
  const sandbox = new env.Sandbox('late');
  let calls = 0;
  const timeout = sandbox.proxy.setTimeout(() => { calls++; });
  const queuedTimer = env.timers.get(timeout).callback;
  const frame = sandbox.proxy.requestAnimationFrame(() => { calls++; });
  const queuedFrame = env.frames.get(frame);
  sandbox.proxy.setInterval(() => { calls++; });
  sandbox.proxy.addEventListener('late', () => { calls++; });
  const queuedListener = env.window.registrations[0].callback;
  const savedSetTimeout = sandbox.proxy.setTimeout;
  const savedSetInterval = sandbox.proxy.setInterval;
  const savedAdd = sandbox.proxy.addEventListener;
  sandbox.teardown();
  sandbox.teardown();
  queuedTimer(); queuedFrame(20); queuedListener({ type: 'late' });
  assert.equal(savedSetTimeout(() => { calls++; }), 0);
  assert.equal(savedSetInterval(() => { calls++; }), 0);
  savedAdd('late', () => { calls++; });
  assert.equal(sandbox.proxy.setTimeout(() => { calls++; }), 0);
  assert.equal(sandbox.proxy.requestAnimationFrame(() => { calls++; }), 0);
  assert.equal(sandbox.proxy.window, sandbox.proxy);
  assert.throws(() => sandbox.run(() => {}), /已卸载/);
  assert.equal(calls, 0);
  assert.equal(env.timers.size + env.frames.size + env.window.registrations.length, 0);
  assertEmpty(sandbox);
});

test('synchronous application styles and links are captured before observer delivery', () => {
  const env = runtime();
  const sandbox = new env.Sandbox('styles');
  const hostStyle = new env.Element('STYLE');
  hostStyle.attributes.add('data-vite-dev-id');
  env.document.head.appendChild(hostStyle);
  const { wrapper, style, link, script } = sandbox.run(() => {
    const wrapper = new env.Element('DIV');
    const style = wrapper.appendChild(new env.Element('STYLE'));
    env.document.body.appendChild(wrapper);
    const link = env.document.head.appendChild(new env.Element('LINK'));
    const script = env.document.body.appendChild(new env.Element('SCRIPT'));
    return { wrapper, style, link, script };
  });
  // No flushMutations: leaving the scope must drain records synchronously.
  sandbox.teardown();
  assert.equal(style.isConnected, false);
  assert.equal(link.isConnected, false);
  assert.equal(script.isConnected, false);
  assert.equal(hostStyle.isConnected, true);
  assert.equal(wrapper.isConnected, true);
  assert.equal(env.observers.size, 0);
  assertEmpty(sandbox);
});

test('host resources added between application callbacks survive micro application teardown', () => {
  const env = runtime();
  const sandbox = new env.Sandbox('business-app');
  const hostStyle = env.document.head.appendChild(new env.Element('STYLE'));
  env.flushMutations();
  const childStyle = sandbox.run(() => env.document.head.appendChild(new env.Element('STYLE')));
  const hostLink = env.document.head.appendChild(new env.Element('LINK'));
  const hostScript = env.document.body.appendChild(new env.Element('SCRIPT'));
  env.flushMutations();
  assert.equal(sandbox.getStats().insertedNodes, 1);
  sandbox.teardown();
  assert.equal(childStyle.isConnected, false);
  assert.equal(hostStyle.isConnected, true);
  assert.equal(hostLink.isConnected, true);
  assert.equal(hostScript.isConnected, true);
  assertEmpty(sandbox);
});

test('timer, animation and event callbacks retain their synchronous resource ownership', () => {
  const env = runtime();
  const sandbox = new env.Sandbox('callbacks');
  const children = [];
  const addStyle = () => children.push(env.document.head.appendChild(new env.Element('STYLE')));
  const timeout = sandbox.proxy.setTimeout(addStyle);
  const frame = sandbox.proxy.requestAnimationFrame(addStyle);
  sandbox.proxy.addEventListener('business-event', addStyle);
  env.fireTimer(timeout);
  env.fireFrame(frame);
  env.window.dispatch('business-event');
  const hostStyle = env.document.head.appendChild(new env.Element('STYLE'));
  env.flushMutations();
  assert.equal(sandbox.getStats().insertedNodes, 3);
  sandbox.teardown();
  assert.ok(children.every(child => !child.isConnected));
  assert.equal(hostStyle.isConnected, true);
});

test('nested sandbox scopes assign each resource to exactly one owner', () => {
  const env = runtime();
  const first = new env.Sandbox('first'), second = new env.Sandbox('second');
  let outerBefore, inner, outerAfter;
  first.run(() => {
    outerBefore = env.document.head.appendChild(new env.Element('STYLE'));
    second.run(() => { inner = env.document.head.appendChild(new env.Element('STYLE')); });
    outerAfter = env.document.head.appendChild(new env.Element('STYLE'));
  });
  env.flushMutations();
  assert.equal(first.getStats().insertedNodes, 2);
  assert.equal(second.getStats().insertedNodes, 1);
  first.teardown();
  assert.equal(outerBefore.isConnected, false);
  assert.equal(outerAfter.isConnected, false);
  assert.equal(inner.isConnected, true);
  second.teardown();
  assert.equal(inner.isConnected, false);
});

test('script scope failure restores document methods before later microtasks add host resources', async () => {
  const env = runtime();
  const sandbox = new env.Sandbox('script-error'), nativeAdd = env.document.addEventListener;
  sandbox.beginScriptScope();
  const scriptStyle = env.document.head.appendChild(new env.Element('STYLE'));
  // The dependency does not call afterExec when script evaluation throws.
  await Promise.resolve();
  assert.equal(env.document.addEventListener, nativeAdd);
  const hostStyle = env.document.head.appendChild(new env.Element('STYLE'));
  env.flushMutations();
  sandbox.teardown();
  assert.equal(scriptStyle.isConnected, false);
  assert.equal(hostStyle.isConnected, true);
});

test('an earlier script microtask cannot add host resources to a later failed script scope', async () => {
  const env = runtime(), sandbox = new env.Sandbox('script-batch');
  const nativeAdd = env.document.addEventListener;
  const leaveFirst = sandbox.beginScriptScope();
  const firstStyle = env.document.head.appendChild(new env.Element('STYLE'));
  let hostStyle;
  const hostWork = Promise.resolve().then(() => {
    assert.equal(env.document.addEventListener, nativeAdd);
    hostStyle = env.document.head.appendChild(new env.Element('STYLE'));
  });
  leaveFirst();
  sandbox.beginScriptScope();
  const failedScriptStyle = env.document.head.appendChild(new env.Element('STYLE'));
  // A later script throws and never calls afterExec. The batch cleanup must
  // already precede the first script's queued host callback.
  await hostWork;
  assert.equal(sandbox.getStats().insertedNodes, 2);
  sandbox.teardown();
  assert.equal(firstStyle.isConnected, false);
  assert.equal(failedScriptStyle.isConnected, false);
  assert.equal(hostStyle.isConnected, true);
});

test('script batch fallback releases nested sandbox scopes in reverse order', async () => {
  const env = runtime(), outer = new env.Sandbox('outer-script'), inner = new env.Sandbox('inner-script');
  const nativeAdd = env.document.addEventListener;
  outer.beginScriptScope();
  const outerStyle = env.document.head.appendChild(new env.Element('STYLE'));
  inner.beginScriptScope();
  const innerStyle = env.document.head.appendChild(new env.Element('STYLE'));
  await Promise.resolve();
  assert.equal(env.document.addEventListener, nativeAdd);
  assert.equal(env.observers.size, 0);
  assert.equal(outer.getStats().insertedNodes, 1);
  assert.equal(inner.getStats().insertedNodes, 1);
  const hostStyle = env.document.head.appendChild(new env.Element('STYLE'));
  outer.teardown();
  assert.equal(outerStyle.isConnected, false);
  assert.equal(innerStyle.isConnected, true);
  inner.teardown();
  assert.equal(innerStyle.isConnected, false);
  assert.equal(hostStyle.isConnected, true);
});

test('scripts beginning in a later microtask create a fresh cleanup batch', async () => {
  const env = runtime(), sandbox = new env.Sandbox('async-scripts');
  const nativeAdd = env.document.addEventListener;
  const firstExit = sandbox.beginScriptScope();
  const firstStyle = env.document.head.appendChild(new env.Element('STYLE'));
  firstExit();
  await Promise.resolve();
  sandbox.beginScriptScope();
  const secondStyle = env.document.head.appendChild(new env.Element('STYLE'));
  const hostWork = Promise.resolve().then(() => {
    assert.equal(env.document.addEventListener, nativeAdd);
    return env.document.head.appendChild(new env.Element('STYLE'));
  });
  const hostStyle = await hostWork;
  assert.equal(sandbox.getStats().insertedNodes, 2);
  assert.equal(env.observers.size, 0);
  sandbox.teardown();
  assert.equal(firstStyle.isConnected, false);
  assert.equal(secondStyle.isConnected, false);
  assert.equal(hostStyle.isConnected, true);
});

test('HTML loader enters a new synchronous scope for each script and releases it on success or error', async () => {
  const loaderSource = readFileSync(new URL('../src/components/micro/loader.ts', import.meta.url), 'utf8');
  const loaderCode = ts.transpileModule(loaderSource, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  for (const failed of [false, true]) {
    const env = runtime(), sandbox = new env.Sandbox('html-scripts'), exports = {};
    const appStyles = [], hostStyles = [];
    let scopes = 0;
    new Script(loaderCode, { filename: 'loader.js' }).runInNewContext({
      exports, AbortController, DOMException, Promise, URL, fetch() {},
      window: { setTimeout, clearTimeout, location: { origin: 'https://host.test', href: 'https://host.test/' } },
      require: () => ({ importEntry: async () => ({ template: '<main></main>',
        execScripts: async (_proxy, _strict, options) => {
          for (let index = 0; index < 2; index++) {
            options.beforeExec();
            appStyles.push(env.document.head.appendChild(new env.Element('STYLE')));
            if (failed && index === 1) throw new Error('entry script failed');
            options.afterExec();
            hostStyles.push(env.document.head.appendChild(new env.Element('STYLE')));
            await Promise.resolve();
          }
        } }),
      }),
    });
    const entry = await exports.loadHtmlEntry('/html-entry', {
      beforeScript: () => { scopes++; return sandbox.beginScriptScope(); },
    });
    const running = entry.execScripts(sandbox.proxy);
    if (failed) await assert.rejects(running, /entry script failed/);
    else await running;
    hostStyles.push(env.document.head.appendChild(new env.Element('STYLE')));
    assert.equal(scopes, 2);
    sandbox.teardown();
    assert.ok(appStyles.every(style => !style.isConnected));
    assert.ok(hostStyles.every(style => style.isConnected));
  }
});

test('the locked HTML evaluator records top-level script resources without claiming later host styles', async () => {
  const env = runtime(), sandbox = new env.Sandbox('html-evaluation');
  const oldWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const appStyles = [];
  Object.assign(env.window, { fetch: async () => { throw new Error('No network expected'); }, frames: [],
    makeStyle: () => { const style = new env.Element('STYLE'); appStyles.push(style); return style; } });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: env.window });
  try {
    const realEntry = createRequire(import.meta.url)('../node_modules/import-html-entry/lib/index.js');
    const script = '<script>document.head.appendChild(window.makeStyle())</script>';
    await realEntry.execScripts(script, [script], sandbox.proxy, {
      strictGlobal: true,
      beforeExec: () => { sandbox.beginScriptScope(); },
      // Deliberately omit afterExec: the throw-safe microtask disposer still
      // ends ownership before subsequent host work, without rewriting scripts.
    });
    const hostStyle = env.document.head.appendChild(new env.Element('STYLE'));
    env.flushMutations();
    sandbox.teardown();
    assert.equal(appStyles.length, 1);
    assert.equal(appStyles[0].isConnected, false);
    assert.equal(hostStyle.isConnected, true);
  } finally {
    sandbox.teardown();
    if (oldWindow) Object.defineProperty(globalThis, 'window', oldWindow);
    else delete globalThis.window;
  }
});

test('nested run scopes restore ownership and old teardown preserves the newer window.proxy', () => {
  const env = runtime();
  const first = new env.Sandbox('first');
  const second = new env.Sandbox('second');
  const native = env.document.addEventListener;
  first.run(() => {
    env.document.addEventListener('first', () => {});
    second.run(() => env.document.addEventListener('second', () => {}));
    env.document.addEventListener('first-again', () => {});
  });
  assert.equal(first.getStats().listeners, 2);
  assert.equal(second.getStats().listeners, 1);
  assert.equal(env.document.addEventListener, native);
  env.window.proxy = second.proxy;
  first.teardown();
  assert.equal(env.window.proxy, second.proxy);
  assert.equal(env.document.registrations.length, 1);
  second.teardown();
  assert.equal(Object.hasOwn(env.window, 'proxy'), false);
  assert.equal(env.document.registrations.length, 0);
});

test('twenty mount/unmount cycles leave no app resources or stale window snapshots', () => {
  const env = runtime();
  for (let cycle = 0; cycle < 20; cycle++) {
    const sandbox = new env.Sandbox(`cycle-${cycle}`);
    env.window.innerWidth = cycle;
    assert.equal(sandbox.proxy.innerWidth, cycle);
    env.window.innerWidth = cycle + 100;
    assert.equal(sandbox.proxy.innerWidth, cycle + 100);
    sandbox.run(() => env.document.addEventListener('delegated', () => {}));
    sandbox.proxy.addEventListener('resize', () => {});
    sandbox.proxy.setInterval(() => {});
    sandbox.proxy.setTimeout(() => {});
    sandbox.proxy.requestAnimationFrame(() => {});
    sandbox.run(() => env.document.head.appendChild(new env.Element('STYLE')));
    env.flushMutations();
    sandbox.teardown();
    assertEmpty(sandbox);
    assert.equal(env.document.head.children.length, 0);
    assert.equal(env.document.registrations.length + env.window.registrations.length, 0);
    assert.equal(env.timers.size + env.frames.size + env.observers.size, 0);
  }
});
