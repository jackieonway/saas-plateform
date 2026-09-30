/** Dante Cloud 品牌 Logo(云 + 盾组合) */
export function BrandMark({ className }: { className?: string }) {
  return (
    <span className={className} aria-hidden>
      <svg viewBox="0 0 48 48" className="size-full">
        <defs>
          <linearGradient id="brand-g" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="currentColor" stopOpacity="0.95" />
            <stop offset="1" stopColor="currentColor" stopOpacity="0.55" />
          </linearGradient>
        </defs>
        <path
          d="M12 30a7 7 0 0 1 .5-13.96A9.5 9.5 0 0 1 30.6 14a8 8 0 0 1 7.9 9.4A6 6 0 0 1 37 31H12Z"
          fill="url(#brand-g)"
          opacity="0.9"
        />
        <path
          d="M24 26l4.2 4.2-1.7 1.7-1.5-1.5v6.1h-2v-6.1l-1.5 1.5-1.7-1.7L24 26Z"
          fill="currentColor"
        />
        <path
          d="M18.2 33.8a6.4 6.4 0 0 1-5.7-3.5"
          stroke="currentColor"
          strokeWidth="2.2"
          strokeLinecap="round"
          fill="none"
        />
      </svg>
    </span>
  )
}

export function BrandWordmark({ className, light }: { className?: string; light?: boolean }) {
  return (
    <div className={`flex items-center gap-2.5 ${className ?? ""}`}>
      <span
        className={`flex size-9 items-center justify-center rounded-xl ${
          light ? "bg-white/10 text-white" : "bg-primary/10 text-primary"
        }`}
      >
        <BrandMark className="size-6" />
      </span>
      <div className="leading-tight">
        <p className={`text-base font-semibold tracking-tight ${light ? "text-white" : ""}`}>Dante Cloud</p>
        <p className={`text-[11px] ${light ? "text-white/60" : "text-muted-foreground"}`}>统一权限平台</p>
      </div>
    </div>
  )
}
