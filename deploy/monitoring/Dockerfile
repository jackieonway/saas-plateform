ARG RUST_IMAGE=rust:1.97.0-slim-bookworm
FROM ${RUST_IMAGE} AS build
WORKDIR /app/deploy/monitoring
COPY deploy/monitoring/Cargo.toml deploy/monitoring/Cargo.lock ./
COPY deploy/monitoring/src ./src
RUN cargo build --locked --release -j 2
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && groupadd --system --gid 10004 monitor && useradd --system --uid 10004 --gid monitor monitor
COPY --from=build /app/deploy/monitoring/target/release/saas-monitor /usr/local/bin/saas-monitor
USER 10004:10004
EXPOSE 9098
ENTRYPOINT ["saas-monitor"]
CMD ["/run/secrets/monitor-config.json"]
