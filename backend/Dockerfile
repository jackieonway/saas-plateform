# ===== 构建阶段 =====
FROM rust:1.97.0-slim-bookworm AS build
WORKDIR /app/backend
# Bound compiler concurrency for clean CI runners and isolated local builds.
ARG CARGO_BUILD_JOBS=2

# 依赖缓存层
COPY Cargo.toml Cargo.lock ./
COPY crates/operational-telemetry ./crates/operational-telemetry
COPY crates/deployment-contract ./crates/deployment-contract
RUN mkdir -p src && echo 'fn main() { println!("cache"); }' > src/main.rs \
    && cargo build --release --locked && rm -rf src target/release/saas-platform-backend*

# 实际源码
COPY src ./src
COPY assets ./assets
COPY migrations ./migrations
COPY initialization ./initialization
RUN touch src/main.rs && cargo build --release --locked

# ===== 运行阶段 =====
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata curl \
    && rm -rf /var/lib/apt/lists/*
RUN groupadd --system --gid 10001 platform && useradd --system --uid 10001 --gid platform --no-create-home platform
ENV APP_ENV=production APPLICATION_ENVIRONMENT=production TZ=Asia/Shanghai RUST_LOG=info BIND_ADDR=0.0.0.0:3001
COPY --from=build /app/backend/target/release/saas-platform-backend /usr/local/bin/saas-platform-backend
USER platform
EXPOSE 3001
HEALTHCHECK --interval=30s --timeout=4s --start-period=60s --retries=3 CMD curl --fail --silent http://127.0.0.1:3001/api/health > /dev/null || exit 1
ENTRYPOINT ["saas-platform-backend"]
