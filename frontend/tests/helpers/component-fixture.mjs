import fs from 'node:fs'
import path from 'node:path'
import vm from 'node:vm'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const require = createRequire(path.join(root, 'package.json'))
const ts = require('typescript')
const symbols = new Proxy({}, { get: (_target, key) => key === '__esModule' ? true : String(key) })
const jsx = (type, props, key) => ({ type, props, key })

/** Execute real component callbacks with deterministic hook state and query results. */
export function componentFixture(relative, names, dependencies = {}, { stableCallbacks = false } = {}) {
  const states = [], refs = [], effectDeps = [], effectCleanups = [], effects = [], mutations = []
  const callbacks = []
  let stateIndex = 0, refIndex = 0, effectIndex = 0, mutationIndex = 0, callbackIndex = 0, dirty = false
  const queries = new Map(), queryOptions = new Map()
  const react = {
    useState(initial) {
      const slot = stateIndex++
      if (!(slot in states)) states[slot] = typeof initial === 'function' ? initial() : initial
      return [states[slot], value => {
        const next = typeof value === 'function' ? value(states[slot]) : value
        if (!Object.is(next, states[slot])) { states[slot] = next; dirty = true }
      }]
    },
    useMemo: callback => callback(),
    useCallback(callback, deps) {
      if (!stableCallbacks) return callback
      const slot = callbackIndex++, previous = callbacks[slot]
      if (!previous || !deps || !previous.deps || deps.length !== previous.deps.length ||
          deps.some((value, index) => !Object.is(value, previous.deps[index]))) callbacks[slot] = { callback, deps }
      return callbacks[slot].callback
    },
    useRef(initial) {
      const slot = refIndex++
      return refs[slot] ??= { current: initial }
    },
    useEffect(callback, deps) {
      const slot = effectIndex++, old = effectDeps[slot]
      if (!old || deps.some((value, index) => !Object.is(value, old[index]))) {
        effectDeps[slot] = deps
        effects.push(() => {
          effectCleanups[slot]?.()
          effectCleanups[slot] = callback()
        })
      }
    },
  }
  const readQuery = options => {
    const key = options.queryKey[0]
    queryOptions.set(key, options)
    return queries.get(key) ?? pending()
  }
  const queryClient = {
    invalidateQueries() {},
    setQueryData(key, data) { queries.set(key[0], success(data)) },
  }
  const deps = {
    react,
    'react/jsx-runtime': { jsx, jsxs: jsx },
    '@tanstack/react-query': {
      useQuery: readQuery,
      useQueries: ({ queries: options }) => options.map(readQuery),
      useQueryClient: () => queryClient,
      useMutation(options) {
        const slot = mutationIndex++
        const entry = mutations[slot] ??= { isPending: false }
        entry.options = options
        entry.mutate = variables => {
          entry.variables = variables
          entry.isPending = true
          entry.promise = Promise.resolve().then(() => options.mutationFn(variables)).then(
            data => { entry.isPending = false; options.onSuccess?.(data, variables); return data },
            error => { entry.isPending = false; options.onError?.(error, variables) },
          )
        }
        return entry
      },
    },
    '@/lib/contact': { telephoneSchema: require('zod').z.string() },
    '@/lib/api': { asArray: data => Array.isArray(data) ? data : data.records ?? [], asPage: data => data },
    '@/lib/tree': { buildTree: () => [] },
    '@/lib/utils': { cn: (...values) => values.filter(Boolean).join(' ') },
    sonner: { toast: { success() {}, error() {} } },
    'framer-motion': { motion: new Proxy({}, { get: (_target, key) => `motion.${String(key)}` }) },
    ...dependencies,
  }
  const source = fs.readFileSync(path.join(root, relative), 'utf8') + `\nexport { ${names.join(', ')} };`
  const compiled = ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX,
  } }).outputText
  const exports = {}
  vm.runInThisContext(`(function(require,exports){${compiled}\n})`, { filename: relative })(name => {
    if (name in deps) return deps[name]
    if (name.startsWith('@/') || name === 'lucide-react') return symbols
    return require(name)
  }, exports)
  return {
    exports, queries, queryOptions, mutations,
    render(name, props = {}, flush = true) {
      let tree
      for (let pass = 0; pass < 10; pass++) {
        stateIndex = 0; refIndex = 0; effectIndex = 0; mutationIndex = 0; callbackIndex = 0; dirty = false
        tree = exports[name](props)
        if (!flush) return tree
        while (effects.length) effects.shift()()
        if (!dirty) return tree
      }
      throw new Error('Component failed to settle')
    },
    unmount() { for (const cleanup of effectCleanups) if (typeof cleanup === 'function') cleanup() },
  }
}

export const success = data => ({ data, isSuccess: true, isLoading: false, isError: false, refetch() {} })
export const pending = () => ({ isSuccess: false, isLoading: true, isError: false, refetch() {} })
export const failure = () => ({ isSuccess: false, isLoading: false, isError: true, error: new Error('read failed'), refetch() {} })
export function elements(tree, predicate) {
  if (Array.isArray(tree)) return tree.flatMap(value => elements(value, predicate))
  if (!tree || typeof tree !== 'object') return []
  return [...(predicate(tree) ? [tree] : []), ...elements(tree.props?.children, predicate)]
}
export function button(tree, label) {
  return elements(tree, item => item.type === 'Button' && JSON.stringify(item.props.children).includes(label))[0]
}
