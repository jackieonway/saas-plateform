# ===== 构建阶段 =====
FROM rust:1.99-slim-bookworm AS build
WORKDIR /app

# 依赖缓存层
COPY backend/Cargo.toml backend/Cargo.lock ./
RUN mkdir -p src && echo 'fn main() { println!("cache"); }' > src/main.rs \
    && cargo build --release --locked && rm -rf src target/release/dante-cloud-backend*

# 实际源码
COPY backend/src ./src
COPY backend/migrations ./migrations
RUN touch src/main.rs && cargo build --release --locked

# ===== 运行阶段 =====
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata curl \
    && rm -rf /var/lib/apt/lists/*
RUN groupadd --system --gid 10001 dante && useradd --system --uid 10001 --gid dante --no-create-home dante
ENV APP_ENV=production APPLICATION_ENVIRONMENT=production TZ=Asia/Shanghai RUST_LOG=info BIND_ADDR=0.0.0.0:3001
COPY --from=build /app/target/release/dante-cloud-backend /usr/local/bin/dante-cloud-backend
USER dante
EXPOSE 3001
HEALTHCHECK --interval=30s --timeout=4s --start-period=60s --retries=3 CMD curl --fail --silent http://127.0.0.1:3001/api/health > /dev/null || exit 1
ENTRYPOINT ["dante-cloud-backend"]
