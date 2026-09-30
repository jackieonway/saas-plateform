export type FlatNode = {
  id: string
  parentId: string | null
  sortOrder?: number
}

export type TreeNode<T> = {
  node: T
  children: TreeNode<T>[]
}

/** 将平铺列表构建为树,按 sortOrder 排序 */
export function buildTree<T extends FlatNode>(flat: T[]): TreeNode<T>[] {
  const map = new Map<string, TreeNode<T>>()
  const roots: TreeNode<T>[] = []
  const sorted = [...flat].sort(
    (a, b) => (a.sortOrder ?? 0) - (b.sortOrder ?? 0) || String(a.id).localeCompare(String(b.id))
  )
  for (const item of sorted) {
    map.set(item.id, { node: item, children: [] })
  }
  for (const item of sorted) {
    const treeNode = map.get(item.id)!
    const parent = item.parentId ? map.get(item.parentId) : undefined
    if (parent) parent.children.push(treeNode)
    else roots.push(treeNode)
  }
  return roots
}

/** 树转平铺 */
export function flattenTree<T>(nodes: TreeNode<T>[]): { node: T; depth: number }[] {
  const out: { node: T; depth: number }[] = []
  const walk = (list: TreeNode<T>[], depth: number) => {
    for (const n of list) {
      out.push({ node: n.node, depth })
      if (n.children.length > 0) walk(n.children, depth + 1)
    }
  }
  walk(nodes, 0)
  return out
}

/** 对已构建的树按 sortOrder 递归排序(返回新树) */
export function sortTree<T extends FlatNode & { title?: string }>(nodes: TreeNode<T>[]): TreeNode<T>[] {
  const sorted = [...nodes].sort(
    (a, b) =>
      (a.node.sortOrder ?? 0) - (b.node.sortOrder ?? 0) ||
      (a.node.title ?? "").localeCompare(b.node.title ?? "", "zh") ||
      String(a.node.id).localeCompare(String(b.node.id))
  )
  return sorted.map((n) => ({ node: n.node, children: n.children.length ? sortTree(n.children) : [] }))
}

/** 对后端返回的菜单树(已是嵌套结构)按 sortOrder 递归排序 */
export function sortMenuTree<T extends FlatNode & { title?: string; children?: T[] }>(nodes: T[]): T[] {
  const sorted = [...nodes].sort(
    (a, b) =>
      (a.sortOrder ?? 0) - (b.sortOrder ?? 0) ||
      (a.title ?? "").localeCompare(b.title ?? "", "zh") ||
      String(a.id).localeCompare(String(b.id))
  )
  return sorted.map((n) => (n.children?.length ? { ...n, children: sortMenuTree(n.children) } : { ...n }))
}

/** 收集某节点全部后代 id */
export function descendantIds<T extends FlatNode>(flat: T[], rootId: string): string[] {
  const childrenMap = new Map<string, string[]>()
  for (const item of flat) {
    const key = item.parentId ?? "__root__"
    if (!childrenMap.has(key)) childrenMap.set(key, [])
    childrenMap.get(key)!.push(item.id)
  }
  const out: string[] = []
  const stack = [...(childrenMap.get(rootId) ?? [])]
  while (stack.length > 0) {
    const id = stack.pop()!
    out.push(id)
    stack.push(...(childrenMap.get(id) ?? []))
  }
  return out
}

/** 在树中查找节点及其祖先链(用于面包屑) */
export function findPath<T extends FlatNode>(flat: T[], id: string): T[] {
  const map = new Map(flat.map((f) => [f.id, f]))
  const path: T[] = []
  let cur = map.get(id)
  const guard = new Set<string>()
  while (cur && !guard.has(cur.id)) {
    guard.add(cur.id)
    path.unshift(cur)
    cur = cur.parentId ? map.get(cur.parentId) : undefined
  }
  return path
}
