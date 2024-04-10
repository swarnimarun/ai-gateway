FROM rust:1.77-slim-bookworm as builder
ADD . /code
WORKDIR /code
RUN update-ca-certificates \
    && apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake
RUN cargo build --release
RUN rm -rf /var/lib/apt/lists/*

FROM debian:stable-slim
COPY --from=builder /code/target/release/ai-gateway /
# copy the aispec.toml for the gateway
COPY --from=builder /code/.aispec.toml /
EXPOSE 8000
EXPOSE 8080
CMD [ "./ai-gateway" ]
