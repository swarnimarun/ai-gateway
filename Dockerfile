FROM rust:1.77-slim-bookworm as builder
RUN mkdir -p /code/src && echo "fn main() {}" > /code/src/main.rs
ADD Cargo.toml /code
ADD Cargo.lock /code
WORKDIR /code
RUN update-ca-certificates \
    && apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake
RUN cargo build
RUN rm -rf /code/src && mkdir /code/src
COPY src/main.rs /code/src/
RUN cargo build
RUN rm -rf /var/lib/apt/lists/*

FROM debian:stable-slim

WORKDIR /
COPY --from=builder /code/target/debug/ai-gateway /
# copy the aispec.toml for the gateway
COPY aispec.docker.toml /aispec.toml

EXPOSE 8000
EXPOSE 8080
CMD [ "./ai-gateway" ]
