#!/usr/bin/env node
/** Phase 1 read-only API isolation probe against an isolated two-tenant test environment. */
import { existsSync, readFileSync } from "node:fs"

if (existsSync(".env.test")) {
  for (const line of readFileSync(".env.test", "utf8").split(/\r?\n/)) {
    const match = /^([A-Z][A-Z0-9_]*)=(.*)$/.exec(line.trim())
    if (!match || process.env[match[1]]) continue
    const raw = match[2].trim()
    process.env[match[1]] =
      (raw.startsWith('"') && raw.endsWith('"')) || (raw.startsWith("'") && raw.endsWith("'"))
        ? raw.slice(1, -1)
        : raw
  }
}

const required = [
  "BASELINE_API_URL",
  "BASELINE_TOKEN_A",
  "BASELINE_TOKEN_B",
  "BASELINE_TENANT_A_ID",
  "BASELINE_TENANT_B_ID",
  "BASELINE_SHARED_USER_ID",
]
const missing = required.filter((key) => !process.env[key])
if (missing.length) {
  console.error(`缺少环境变量: ${missing.join(", ")}`)
  console.error("仅在隔离测试环境运行；A/B token 应是具备用户和角色管理权限的非平台租户管理员 session JWT。")
  process.exit(2)
}

const base = process.env.BASELINE_API_URL.replace(/\/$/, "")
const tenantA = process.env.BASELINE_TENANT_A_ID.toLowerCase()
const tenantB = process.env.BASELINE_TENANT_B_ID.toLowerCase()
const sharedUser = process.env.BASELINE_SHARED_USER_ID

async function get(path, token) {
  const res = await fetch(`${base}${path}`, {
    headers: { Authorization: `Bearer ${token}`, Accept: "application/json" },
  })
  const body = await res.json().catch(() => null)
  if (!res.ok) throw new Error(`${path}: HTTP ${res.status} ${JSON.stringify(body)}`)
  return body
}

const cases = [
  ["A tenant list excludes B", "/api/tenants?size=200", process.env.BASELINE_TOKEN_A, tenantB],
  ["A organization tree excludes B", `/api/organizations/tree?tenantId=${tenantB}`, process.env.BASELINE_TOKEN_A, tenantB],
  ["A user list excludes B memberships", `/api/users?size=200&tenantId=${tenantB}`, process.env.BASELINE_TOKEN_A, tenantB],
  ["A shared-user memberships exclude B", `/api/users/${sharedUser}/memberships`, process.env.BASELINE_TOKEN_A, tenantB],
  ["A role list excludes B", `/api/roles?size=200&tenantId=${tenantB}`, process.env.BASELINE_TOKEN_A, tenantB],
  ["B tenant list excludes A", "/api/tenants?size=200", process.env.BASELINE_TOKEN_B, tenantA],
  ["B organization tree excludes A", `/api/organizations/tree?tenantId=${tenantA}`, process.env.BASELINE_TOKEN_B, tenantA],
  ["B user list excludes A memberships", `/api/users?size=200&tenantId=${tenantA}`, process.env.BASELINE_TOKEN_B, tenantA],
  ["B shared-user memberships exclude A", `/api/users/${sharedUser}/memberships`, process.env.BASELINE_TOKEN_B, tenantA],
  ["B role list excludes A", `/api/roles?size=200&tenantId=${tenantA}`, process.env.BASELINE_TOKEN_B, tenantA],
]

let failures = 0
for (const [name, path, token, forbiddenId] of cases) {
  try {
    const body = await get(path, token)
    const leaked = JSON.stringify(body).toLowerCase().includes(forbiddenId)
    console.log(`${leaked ? "FAIL" : "PASS"} ${name}`)
    if (leaked) failures++
  } catch (error) {
    console.error(`ERROR ${name}: ${error.message}`)
    failures++
  }
}

if (failures) {
  console.error(`${failures}/${cases.length} 项未通过；这是后续隔离修复的基线。`)
  process.exit(1)
}
console.log(`${cases.length}/${cases.length} 项通过。`)
