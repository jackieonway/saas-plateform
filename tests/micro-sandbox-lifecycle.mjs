import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createContext, Script } from 'node:vm';
import ts from '../frontend/node_modules/typescript/lib/typescript.js';

const source = readFileSync(new URL('../frontend/src/components/micro/sandbox.ts', import.meta.url), 'utf8');
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
    observe(target, options) { this.targets.push({ target, options }); }
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
  const context = createContext({ window, document, Node: { ELEMENT_NODE: 1 }, MutationObserver, exports });
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

test('pending mutation records, nested styles and links are removed even before observer delivery', () => {
  const env = runtime();
  const sandbox = new env.Sandbox('styles');
  const hostStyle = new env.Element('STYLE');
  hostStyle.attributes.add('data-vite-dev-id');
  env.document.head.appendChild(hostStyle);
  const wrapper = new env.Element('DIV');
  const style = wrapper.appendChild(new env.Element('STYLE'));
  env.document.body.appendChild(wrapper);
  const link = env.document.head.appendChild(new env.Element('LINK'));
  const script = env.document.body.appendChild(new env.Element('SCRIPT'));
  // No flushMutations: teardown must drain records synchronously.
  sandbox.teardown();
  assert.equal(style.isConnected, false);
  assert.equal(link.isConnected, false);
  assert.equal(script.isConnected, false);
  assert.equal(hostStyle.isConnected, true);
  assert.equal(wrapper.isConnected, true);
  assert.equal(env.observers.size, 0);
  assertEmpty(sandbox);
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
    env.document.head.appendChild(new env.Element('STYLE'));
    env.flushMutations();
    sandbox.teardown();
    assertEmpty(sandbox);
    assert.equal(env.document.head.children.length, 0);
    assert.equal(env.document.registrations.length + env.window.registrations.length, 0);
    assert.equal(env.timers.size + env.frames.size + env.observers.size, 0);
  }
});
