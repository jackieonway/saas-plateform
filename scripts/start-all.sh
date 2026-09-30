#!/bin/bash
# Dante Cloud 全部服务启动脚本 (沙箱安全版)
# 重要: 本沙箱会在工具会话结束后回收普通后台子进程 (nohup/setsid+& 均无效),
# 必须使用 double-fork 模式: ( ( exec setsid CMD > log 2>&1 < /dev/null ) & )
cd /home/z/my-project
export PATH="$HOME/.cargo/bin:$PATH"

# 1. PostgreSQL (5432) - pg_ctl 自身就是守护化启动, 直接调用
~/pgsql/bin-scripts/start-pg.sh >/dev/null 2>&1 || true

# 2. Rust 后端 (3001)
if ! ss -tln 2>/dev/null | grep -q ':3001 '; then
  ( ( exec setsid bash -c 'cd /home/z/my-project/backend && export PATH="$HOME/.cargo/bin:$PATH" && exec ~/.cargo/bin/cargo run >> /home/z/my-project/backend.log 2>&1' < /dev/null ) & )
  echo "[start-all] Rust 后端启动中 :3001 (backend.log)"
else
  echo "[start-all] Rust 后端已运行 :3001"
fi

# 3. Vite 前端 (3000) — npm + vite (Task 18-d 去 bun 化)
if ! ss -tln 2>/dev/null | grep -q ':3000 '; then
  ( ( exec setsid bash -c 'cd /home/z/my-project/frontend && exec npm run dev >> /home/z/my-project/dev.log 2>&1' < /dev/null ) & )
  echo "[start-all] Vite 前端启动中 :3000 (dev.log)"
else
  echo "[start-all] Vite 前端已运行 :3000"
fi

# 4. 通知 WebSocket 服务 (3003, mini-services/notification-ws) — node --watch (Task 18-d)
if ! ss -tln 2>/dev/null | grep -q ':3003 '; then
  ( ( exec setsid bash -c 'cd /home/z/my-project/mini-services/notification-ws && exec npm run dev >> /home/z/my-project/mini-services/notification-ws/service.log 2>&1' < /dev/null ) & )
  echo "[start-all] 通知 WS 服务启动中 :3003 (notification-ws/service.log)"
else
  echo "[start-all] 通知 WS 服务已运行 :3003"
fi

# 5. OAuth 测试服务 (3004, mini-services/oauth-playground) — Task 16-c 建, Task 18-d 改纯 node:http
if ! ss -tln 2>/dev/null | grep -q ':3004 '; then
  ( ( exec setsid bash -c 'cd /home/z/my-project/mini-services/oauth-playground && exec npm run dev >> /home/z/my-project/mini-services/oauth-playground/service.log 2>&1' < /dev/null ) & )
  echo "[start-all] OAuth 测试服务启动中 :3004 (oauth-playground/service.log)"
else
  echo "[start-all] OAuth 测试服务已运行 :3004"
fi

# 6. 微前端 Vue 子应用 (3005, mini-services/micro-vue) — Task 21-b, bun --hot 静态服务
if ! ss -tln 2>/dev/null | grep -q ':3005 '; then
  ( ( exec setsid bash -c 'cd /home/z/my-project/mini-services/micro-vue && exec npm run dev >> /home/z/my-project/mini-services/micro-vue/service.log 2>&1' < /dev/null ) & )
  echo "[start-all] 微前端 Vue 子应用启动中 :3005 (micro-vue/service.log)"
else
  echo "[start-all] 微前端 Vue 子应用已运行 :3005"
fi

# 7. 微前端 jQuery 遗留子应用 (3006, mini-services/micro-jquery) — Task 21-b, bun --hot 静态服务
if ! ss -tln 2>/dev/null | grep -q ':3006 '; then
  ( ( exec setsid bash -c 'cd /home/z/my-project/mini-services/micro-jquery && exec npm run dev >> /home/z/my-project/mini-services/micro-jquery/service.log 2>&1' < /dev/null ) & )
  echo "[start-all] 微前端 jQuery 遗留子应用启动中 :3006 (micro-jquery/service.log)"
else
  echo "[start-all] 微前端 jQuery 遗留子应用已运行 :3006"
fi

# 8. 微前端子应用静态产物同步 (Task 23: 生产形态挂载)
#    mini-services/micro-*/public = 子应用源码; frontend/public/sub-apps/* = 部署产物
#    (零构建子应用的 "build" = 复制)。主应用 entry_url 已默认指向静态路径
#    (/sub-apps/{vue|jquery}/index.html), 挂载不依赖 3005/3006 端口存活。
#    touch 必不可少: Vite 对 public 目录新复制文件存在 watcher 缓存投毒现象
#    (Task 23 实证: 复制后首个请求可能被 SPA fallback 以 793B index.html 冒充 JS,
#    且 Vite 源码 fallback 条件 accept.includes("text/html")||accept.includes("*/*")
#    会劫持 fetch 的 */* 请求), touch 触发 change 事件即可解除并预防。
#    开发形态切换: entry_url 改回 /sub/vue/ 或 /sub/jquery/ 即走 3005/3006 代理。
for pair in "vue:micro-vue" "jquery:micro-jquery"; do
  name="${pair%%:*}"; svc="${pair##*:}"
  src="/home/z/my-project/mini-services/$svc/public"
  dst="/home/z/my-project/frontend/public/sub-apps/$name"
  mkdir -p "$dst/vendor"
  cp -f "$src/index.html" "$dst/" 2>/dev/null
  cp -f "$src/app.js" "$dst/" 2>/dev/null
  cp -f "$src/vendor/"* "$dst/vendor/" 2>/dev/null
  touch "$dst/index.html" "$dst/app.js" "$dst/vendor/"* 2>/dev/null
done
echo "[start-all] 微前端子应用静态产物已同步+失效 → frontend/public/sub-apps/* (entry_url 默认静态形态)"

sleep 2
ss -tln 2>/dev/null | grep -E ':(3000|3001|3003|3004|3005|3006|5432) ' || echo "等待服务就绪..."
