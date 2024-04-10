FROM rust:1.77-slim-bookworm as builder
ADD . /code
WORKDIR /code
RUN update-ca-certificates \
    && apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake
RUN cargo build --release
RUN rm -rf /var/lib/apt/lists/*

FROM debian:stable-slim
COPY --from=builder /code/target/release/dummyai /
EXPOSE 8000
CMD [ "./dummyai" ]
