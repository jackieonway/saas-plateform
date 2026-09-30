#!/usr/bin/env node
/** Copy a built HTML micro app into Vite's public/sub-apps/<code> directory. */
import { cpSync, existsSync, mkdirSync, statSync } from "node:fs"
import { resolve, join } from "node:path"
import { fileURLToPath } from "node:url"

const [code, sourceArg] = process.argv.slice(2)
if (!code || !/^[a-z0-9-]{2,32}$/.test(code) || !sourceArg) {
  console.error("用法: node scripts/sync-microapp.mjs <应用编码> <子应用构建目录>")
  process.exit(1)
}

const source = resolve(sourceArg)
if (!existsSync(source) || !statSync(source).isDirectory() || !existsSync(join(source, "index.html"))) {
  console.error(`构建目录必须存在并包含 index.html: ${source}`)
  process.exit(1)
}

const projectRoot = resolve(fileURLToPath(new URL("..", import.meta.url)))
const destination = join(projectRoot, "frontend", "public", "sub-apps", code)
if (source === destination || source.startsWith(destination + "\\") || source.startsWith(destination + "/")) {
  console.error("源目录不能位于目标目录内")
  process.exit(1)
}

mkdirSync(destination, { recursive: true })
cpSync(source, destination, { recursive: true, force: true })
console.log(`已同步 ${code}: ${source} -> ${destination}`)
console.log(`入口地址: /sub-apps/${code}/index.html`)
