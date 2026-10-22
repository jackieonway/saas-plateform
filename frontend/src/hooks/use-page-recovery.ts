import { useEffect } from "react"

/** Recover a page removed by deletion or retention only after its own query completes. */
export function usePageRecovery(
  page: number,
  size: number,
  query: { data?: { total: number }; isSuccess: boolean; isPlaceholderData?: boolean },
  onPageChange: (page: number) => void,
) {
  const total = query.data?.total
  useEffect(() => {
    if (!query.isSuccess || query.isPlaceholderData || total === undefined) return
    const lastPage = Math.max(1, Math.ceil(total / size))
    if (page > lastPage) onPageChange(lastPage)
  }, [page, size, total, query.isSuccess, query.isPlaceholderData, onPageChange])
}
