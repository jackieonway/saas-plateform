#!/usr/bin/env node
/** Build an immutable static release and its POST /applications/:id/releases body. */
import { constants, fstatSync, lstatSync, mkdirSync, openSync, closeSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { basename, dirname, extname, isAbsolute, join, parse, relative, resolve, sep } from "node:path"
import { fileURLToPath } from "node:url"

const MAX_FILE_BYTES = 4 * 1024 * 1024
const MAX_TOTAL_BYTES = 16 * 1024 * 1024
const MAX_RESOURCES = 48
const projectRoot = fileURLToPath(new URL("../", import.meta.url))
const usage = `用法: node scripts/package-microapp.mjs --app <应用编码> --environment <环境> --version <版本> --input <构建目录>
  [--output-root <存储根目录>] [--entry-type HTML|ESM] [--entry <入口相对路径>] [--health-url <地址>]
默认输出 deploy/micro-apps/<应用>/versions/<版本>/；默认 HTML 入口 index.html，ESM 入口 entry.mjs。
版本只登记元数据，不自动预检、激活、变更应用运行方式或授权。已有版本不可覆盖。`

function parseArgs(args) {
  if (args.length === 1 && args[0] === "--help") return null
  const allowed = new Set(["app", "environment", "version", "input", "output-root", "entry-type", "entry", "health-url"])
  const values = {}
  for (let index = 0; index < args.length; index += 2) {
    const name = args[index]?.startsWith("--") ? args[index].slice(2) : ""
    if (!allowed.has(name)) throw new Error(`未知参数：${args[index] ?? ""}`)
    if (Object.hasOwn(values, name)) throw new Error(`重复参数：--${name}`)
    const value = args[index + 1]
    if (!value || value.startsWith("--")) throw new Error(`参数 --${name} 缺少值`)
    values[name] = value
  }
  for (const name of ["app", "environment", "version", "input"]) {
    if (!values[name]) throw new Error(`缺少参数 --${name}`)
  }
  return values
}

function containedBy(parent, candidate) {
  const path = relative(parent, candidate)
  return path !== "" && !isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`)
}

function noSymlinkAncestors(path) {
  const absolute = resolve(path)
  let current = parse(absolute).root
  for (const segment of absolute.slice(current.length).split(sep).filter(Boolean)) {
    current = join(current, segment)
    let entry
    try { entry = lstatSync(current) } catch (error) {
      if (error.code === "ENOENT") break
      throw error
    }
    if (entry.isSymbolicLink()) throw new Error(`不接受符号链接或目录 junction：${current}`)
  }
}

function validateEntryPath(value) {
  if (!value || isAbsolute(value) || value.includes("\\") || /[?#\x00-\x1f]/.test(value)) {
    throw new Error("入口必须是构建目录内的相对文件路径，不能包含查询参数、锚点或路径穿越")
  }
  for (const part of value.split("/")) {
    let decoded
    try { decoded = decodeURIComponent(part) } catch { throw new Error("入口路径包含无效编码") }
    if (!part || decoded === "." || decoded === ".." || /[\\/\x00-\x1f]/.test(decoded)) {
      throw new Error("入口不能包含路径穿越或空目录段")
    }
  }
}

function encodePath(value) {
  return value.split(sep).map((part) => part === ".." ? part : encodeURIComponent(part)).join("/")
}

function resourceKind(path) {
  const extension = extname(path).toLowerCase()
  if ([".js", ".mjs", ".cjs"].includes(extension)) return "SCRIPT"
  if (extension === ".css") return "STYLE"
  return "ASSET"
}

function inventory(source, entryPath) {
  const files = []
  let bytes = 0
  function visit(directory) {
    for (const name of readdirSync(directory).sort()) {
      if (/^\.env/i.test(name) || [".git", ".npmrc", ".yarnrc", ".yarnrc.yml"].includes(name.toLowerCase())) {
        throw new Error(`构建目录含敏感文件或目录，禁止发布：${name}`)
      }
      if (/[\\\x00-\x1f]/.test(name)) throw new Error(`构建目录含不安全文件名：${name}`)
      const path = join(directory, name)
      const stat = lstatSync(path)
      if (stat.isSymbolicLink()) throw new Error(`不接受符号链接或目录 junction：${path}`)
      if (stat.isDirectory()) visit(path)
      else if (stat.isFile()) {
        const relativePath = relative(source, path)
        if (relativePath === "release.json") throw new Error("构建目录根的 release.json 为打包器保留文件名")
        if (stat.size === 0) throw new Error(`预检不接受空资源：${relativePath}`)
        if (stat.size > MAX_FILE_BYTES) throw new Error(`资源超过预检 4 MiB 限制：${relativePath}`)
        if (path !== entryPath && /\.html?$/i.test(path)) {
          throw new Error(`非入口 HTML 不能登记为 ASSET：${relativePath}；请使用单入口构建目录`)
        }
        bytes += stat.size
        if (bytes > MAX_TOTAL_BYTES) throw new Error("构建目录超过预检 16 MiB 总响应限制")
        files.push({ path, relativePath, stat })
      } else throw new Error(`只接受普通文件和目录：${path}`)
    }
  }
  visit(source)
  if (!files.some((file) => file.path === entryPath)) throw new Error("入口不存在或不是普通文件")
  if (files.length - 1 > MAX_RESOURCES) throw new Error(`资源清单超过 API 的 ${MAX_RESOURCES} 项限制，请精简构建目录`)
  return files
}

function packageRelease(values) {
  const app = values.app
  const environment = values.environment
  const version = values.version
  const entryType = values["entry-type"] ?? "HTML"
  if (!/^[a-z0-9][a-z0-9-]{1,31}$/.test(app)) throw new Error("应用编码须为 2–32 位小写字母、数字或中划线，首位为字母或数字")
  if (!/^[a-z0-9][a-z0-9_-]{0,31}$/.test(environment)) throw new Error("环境须为 1–32 位小写字母、数字、下划线或中划线")
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(version) || version.includes("..")) throw new Error("版本须为 1–64 位字母、数字、点、下划线或中划线，不能包含路径穿越")
  if (!["HTML", "ESM"].includes(entryType)) throw new Error("入口类型须为 HTML 或 ESM")
  const entry = values.entry ?? (entryType === "HTML" ? "index.html" : "entry.mjs")
  validateEntryPath(entry)
  if (entryType === "HTML" && !/\.html?$/i.test(entry)) throw new Error("HTML 入口须为 .html 或 .htm 文件")
  if (entryType === "ESM" && !/\.(?:mjs|js)$/i.test(entry)) throw new Error("ESM 入口须为 .mjs 或 .js 文件")
  const source = resolve(values.input)
  const outputRoot = resolve(values["output-root"] ?? join(projectRoot, "deploy", "micro-apps"))
  noSymlinkAncestors(source)
  noSymlinkAncestors(outputRoot)
  if (source === outputRoot || containedBy(source, outputRoot) || containedBy(outputRoot, source)) {
    throw new Error("源构建目录和输出根目录不能相同或互相包含")
  }
  if (!lstatSync(source).isDirectory()) throw new Error("源构建目录必须是普通目录")
  const entryPath = resolve(source, entry)
  if (!containedBy(source, entryPath)) throw new Error("入口路径必须位于构建目录内")
  const destination = join(outputRoot, app, "versions", version)
  if (!containedBy(outputRoot, destination)) throw new Error("版本输出路径必须位于输出根目录内")
  noSymlinkAncestors(destination)
  // lstat detects dangling links too; mkdir below reserves the version without overwrite.
  try { lstatSync(destination); throw new Error("版本目录已存在，不可覆盖；请使用新版本号") } catch (error) {
    if (error.code !== "ENOENT") throw error
  }
  const files = inventory(source, entryPath)
  const release = {
    environment,
    version,
    entryType,
    entryUrl: `/sub-apps/${app}/versions/${version}/${encodePath(entry.split("/").join(sep))}`,
  }
  if (values["health-url"]) {
    const healthUrl = values["health-url"]
    if (/\s/.test(healthUrl)) throw new Error("健康检查地址不能包含空白字符")
    const parsed = new URL(healthUrl, "https://gateway.invalid")
    if (!["http:", "https:"].includes(parsed.protocol) || parsed.username || parsed.password || parsed.hash || healthUrl.startsWith("//")) {
      throw new Error("健康检查地址必须是无凭据、无锚点的 HTTP(S) 地址或相对路径")
    }
    if (environment === "production" && parsed.protocol !== "https:") throw new Error("production 健康检查必须使用 HTTPS")
    release.healthUrl = healthUrl
  }
  release.resources = files.filter((file) => file.path !== entryPath).map((file) => ({
    url: encodePath(relative(dirname(entryPath), file.path)),
    kind: resourceKind(file.path),
  }))
  mkdirSync(dirname(destination), { recursive: true })
  noSymlinkAncestors(dirname(destination))
  mkdirSync(destination) // exclusive; concurrent attempts cannot reuse this directory
  let complete = false
  try {
    for (const file of files) {
      noSymlinkAncestors(file.path)
      const current = lstatSync(file.path)
      if (!current.isFile() || current.ino !== file.stat.ino || current.dev !== file.stat.dev) throw new Error(`源文件在打包期间改变：${file.relativePath}`)
      const descriptor = openSync(file.path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0))
      let content
      try {
        const opened = fstatSync(descriptor)
        if (!opened.isFile() || opened.ino !== file.stat.ino || opened.dev !== file.stat.dev || opened.size !== file.stat.size) throw new Error(`源文件在打包期间改变：${file.relativePath}`)
        content = readFileSync(descriptor)
      } finally { closeSync(descriptor) }
      if (content.length !== file.stat.size) throw new Error(`源文件在打包期间改变：${file.relativePath}`)
      const target = join(destination, file.relativePath)
      if (!containedBy(destination, target)) throw new Error("资源输出路径越界")
      mkdirSync(dirname(target), { recursive: true })
      writeFileSync(target, content, { flag: "wx" })
    }
    writeFileSync(join(destination, "release.json"), `${JSON.stringify(release, null, 2)}\n`, { flag: "wx" })
    complete = true
  } finally {
    if (!complete && containedBy(outputRoot, destination) && basename(destination) === version) {
      rmSync(destination, { recursive: true, force: true })
    }
  }
  console.log(`已打包 ${app} ${environment} ${version}：${destination}`)
  console.log(`入口地址：${release.entryUrl}`)
  console.log(`注册请求：${join(destination, "release.json")}（${release.resources.length} 项资源）`)
}

try {
  const values = parseArgs(process.argv.slice(2))
  if (!values) console.log(usage)
  else packageRelease(values)
} catch (error) {
  console.error(`打包失败：${error.message}\n${usage}`)
  process.exitCode = 1
}
