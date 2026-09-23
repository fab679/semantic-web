# Multi-stage build for the Rust service (workspace crates/semweb).
# Stage 1 compiles the release binary; stage 2 is a slim runtime with
# only the binary, the demo seed and the certs reqwest needs for HTTPS.

FROM rust:1-slim AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p semweb

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /build/target/release/semweb /app/semweb
COPY --from=build /build/target/release/demo-subscriber /app/demo-subscriber
COPY crates/semweb/sample_data.ttl /app/sample_data.ttl
COPY crates/semweb/shapes.ttl /app/shapes.ttl

EXPOSE 8000
CMD ["/app/semweb"]