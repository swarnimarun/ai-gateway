FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder 
RUN update-ca-certificates \
    && apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release --bin ai-gateway 

# We do not need the Rust toolchain to run the binary!
FROM debian:bookworm-slim AS runtime
COPY --from=builder /app/target/release/ai-gateway /
COPY aispec.docker.toml /aispec.toml
EXPOSE 6191 
ENTRYPOINT ["./ai-gateway"]