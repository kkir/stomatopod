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

# No `VOLUME /app/data` by design: an anonymous volume would make the data dir
# look "mounted" yet still be orphaned on redeploy, defeating the startup
# persistence guard. Mount a named volume / bind mount instead — see DEPLOY.md.

USER stomatopod

EXPOSE 8080

ENV RUST_LOG=info

CMD ["stomatopod", "serve"]
