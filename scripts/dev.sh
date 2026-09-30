#!/bin/bash
# Dante Cloud 开发环境启动脚本: PostgreSQL -> Rust 后端(3001) -> Vite 前端(3000)
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"

# 1. 确保 PostgreSQL 运行
~/pgsql/bin-scripts/start-pg.sh >/dev/null 2>&1 || true

# 2. 确保 Rust 后端运行在 3001
if ! ss -tln 2>/dev/null | grep -q ':3001 '; then
  if [ -f backend/Cargo.toml ]; then
    (cd backend && nohup cargo run >> ../backend.log 2>&1 &)
    echo "[dev] Rust 后端正在启动于 :3001 (日志: backend.log)"
  fi
else
  echo "[dev] Rust 后端已运行于 :3001"
fi

# 3. 前台启动 Vite (端口 3000), 日志 tee 到 dev.log
cd frontend
bun run dev 2>&1 | tee -a ../dev.log
