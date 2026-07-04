FROM rust:1-bookworm AS builder

WORKDIR /app

# Toolchain for building the Dioxus SPA: the wasm target plus the pinned
# `dx` CLI (matches the version in mise.toml / crates/ui/Cargo.toml).
RUN rustup target add wasm32-unknown-unknown \
    && cargo install dioxus-cli --version 0.7.9 --locked

COPY . .

# Build the SPA bundle first and stage it into crates/web/ui-dist/ so
# rust-embed embeds it into the release binary at compile time. (This mirrors
# `mise run ui:bundle`; ui-dist is gitignored and rebuilt here.)
RUN cd crates/ui && dx bundle --release \
    && rm -rf /app/crates/web/ui-dist \
    && mkdir -p /app/crates/web/ui-dist \
    && cp -r /app/target/dx/stomatopod-ui/release/web/public/. /app/crates/web/ui-dist/

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
