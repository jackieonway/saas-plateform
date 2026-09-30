import * as React from "react"
import { useSearchParams } from "react-router-dom"

/**
 * 列表页关键字与 URL ?q= 双向同步:
 * - 挂载时读取 ?q= 预填关键字(命令面板实体跳转带入)
 * - 用户输入/清空时 replace 回写 URL(清空移除参数)
 * - 命令面板在当前页再次跳转(?q= 变化)时同步进输入框
 */
export function useSyncedKeyword(): [string, React.Dispatch<React.SetStateAction<string>>] {
  const [searchParams, setSearchParams] = useSearchParams()
  const urlQ = searchParams.get("q") ?? ""
  const [keyword, setKeyword] = React.useState(urlQ)

  // 记录最近一次同步值,区分「URL → state」与「state → URL」两个方向,避免回环
  const lastSynced = React.useRef(urlQ)

  // 外部 ?q= 变化(命令面板跳转 / 前进后退) → 写入输入框
  React.useEffect(() => {
    if (urlQ !== lastSynced.current) {
      lastSynced.current = urlQ
      setKeyword(urlQ)
    }
  }, [urlQ])

  // 输入框变化 → 回写 URL(replace 不产生历史记录)
  React.useEffect(() => {
    if (keyword === lastSynced.current) return
    lastSynced.current = keyword
    setSearchParams(
      (prev) => {
        const next = new URLSearchParams(prev)
        if (keyword) next.set("q", keyword)
        else next.delete("q")
        return next
      },
      { replace: true }
    )
  }, [keyword, setSearchParams])

  return [keyword, setKeyword]
}
