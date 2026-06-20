FROM rust:1-bookworm AS builder

WORKDIR /app

COPY . .
RUN cargo build --release --bin stomatopod

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system stomatopod \
    && useradd --system --gid stomatopod --create-home --home-dir /home/stomatopod stomatopod

WORKDIR /app

COPY --from=builder /app/target/release/stomatopod /usr/local/bin/stomatopod

RUN mkdir -p /app/data \
    && chown -R stomatopod:stomatopod /app

USER stomatopod

EXPOSE 8080

ENV RUST_LOG=info

CMD ["stomatopod", "serve"]
