/**
 * 轻量 User-Agent 解析(纯前端, 无第三方依赖):
 * 识别常见浏览器(Chrome/Edge/Firefox/Safari/Opera/curl/Node 等)与
 * 操作系统(Windows/macOS/Linux/Android/iOS 等);
 * 两者均无法识别时返回 null(调用方回退为显示原文截断)。
 */

export interface ParsedUserAgent {
  /** 浏览器名(如 Chrome / Edge / curl), 单侧无法识别时为「未知」 */
  browser: string
  /** 操作系统名(如 Windows / macOS / iOS), 单侧无法识别时为「未知」 */
  os: string
}

/** 浏览器识别规则: 顺序敏感(Edge/Opera 等基于 Chromium 的 UA 须排在 Chrome 之前) */
const BROWSER_RULES: [RegExp, string][] = [
  // Headless Chrome: 其 UA 中 "Chrome" 前接 "Headless" 的字母, \bChrome/ 不满足单词边界,
  // 若不前置匹配会一路落到 Safari 规则(HeadlessChrome/153 Safari/537 → 误判 Safari), 故必须最先
  [/\bHeadlessChrome\//, "Headless Chrome"],
  [/\bEdg(?:e|A|iOS)?\//, "Edge"],
  [/\bEdg\b/, "Edge"],
  [/\bOPR\//, "Opera"],
  [/\bOpera[/ ]/, "Opera"],
  [/\bSamsungBrowser\//, "Samsung Internet"],
  [/\bFxiOS\//, "Firefox"],
  [/\bFirefox\//, "Firefox"],
  [/\bCriOS\//, "Chrome"],
  [/\bChrome\//, "Chrome"],
  [/\bVersion\/[\d.]+\s+Safari/, "Safari"],
  [/\bSafari\b(?!.*(?:Chrome|Chromium|Edg|OPR))/, "Safari"],
  [/\bMSIE\s/, "IE"],
  [/\bTrident\//, "IE"],
  [/\bcurl\//, "curl"],
  [/\bWget\//, "Wget"],
  [/\bPostmanRuntime\//, "Postman"],
  [/^python-requests\//, "Python requests"],
  [/^Java\//, "Java"],
  [/\baxios\//, "axios"],
  [/\bnode-fetch\//, "Node"],
  [/\bundici\//, "Node"],
  [/^node$/i, "Node"],
  [/\bNode\.js\b/i, "Node"],
  [/\bokhttp\//, "okhttp"],
  [/\bDarwin\b.*\bCFNetwork\//, "CFNetwork"],
]

/** 操作系统识别规则: 顺序敏感(iPhone/iPad 的 UA 含 "like Mac OS X", 须在 macOS 之前) */
const OS_RULES: [RegExp, string][] = [
  [/iPhone/, "iOS"],
  [/iPad/, "iPadOS"],
  [/Android/, "Android"],
  [/Windows|Win64|Win32|WOW64/, "Windows"],
  [/Mac OS X|Macintosh/, "macOS"],
  [/\bCrOS\b/, "ChromeOS"],
  [/\bLinux\b|X11/, "Linux"],
  [/\bFreeBSD\b/, "FreeBSD"],
]

function matchFirst(rules: [RegExp, string][], ua: string): string | null {
  for (const [re, name] of rules) if (re.test(ua)) return name
  return null
}

/**
 * 解析 User-Agent 字符串:
 * - 两侧均识别 → { browser, os }
 * - 仅识别一侧 → 另一侧填「未知」
 * - 均无法识别(或空串) → null
 */
export function parseUserAgent(ua: string): ParsedUserAgent | null {
  if (!ua) return null
  const browser = matchFirst(BROWSER_RULES, ua)
  const os = matchFirst(OS_RULES, ua)
  if (!browser && !os) return null
  return { browser: browser ?? "未知", os: os ?? "未知" }
}
