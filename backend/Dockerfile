# ===== 构建阶段 =====
FROM rust:1-slim AS build
WORKDIR /app

# 依赖缓存层
COPY backend/Cargo.toml backend/Cargo.lock ./
RUN mkdir -p src && echo 'fn main() { println!("cache"); }' > src/main.rs \
    && cargo build --release && rm -rf src target/release/dante-cloud-backend*

# 实际源码
COPY backend/src ./src
COPY backend/migrations ./migrations
RUN touch src/main.rs && cargo build --release

# ===== 运行阶段 =====
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*
ENV TZ=Asia/Shanghai RUST_LOG=info BIND_ADDR=0.0.0.0:3001
COPY --from=build /app/target/release/dante-cloud-backend /usr/local/bin/dante-cloud-backend
EXPOSE 3001
ENTRYPOINT ["dante-cloud-backend"]
