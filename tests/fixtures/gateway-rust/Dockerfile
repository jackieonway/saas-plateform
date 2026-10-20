# Test-only image. Never copy this peer into a production service image.
FROM rust:1.97.0-bookworm AS build
WORKDIR /fixture
COPY tests/fixtures/gateway-rust/Cargo.toml tests/fixtures/gateway-rust/Cargo.lock ./
COPY tests/fixtures/gateway-rust/src ./src
RUN cargo build --locked --release

FROM debian:bookworm-slim
RUN groupadd --gid 10001 fixture && useradd --uid 10001 --gid fixture --no-create-home fixture
COPY --from=build /fixture/target/release/gateway-rust-transport-fixture /usr/local/bin/gateway-rust-transport-fixture
USER 10001:10001
# Bind and container wildcard opt-in must be explicit in the isolated test Compose.
ENTRYPOINT ["/usr/local/bin/gateway-rust-transport-fixture"]
