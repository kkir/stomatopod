FROM rust:1-bookworm AS builder

WORKDIR /app

# Toolchain for the Dioxus fullstack build: the wasm target plus the pinned
# `dx` CLI (matches the version in mise.toml).
RUN rustup target add wasm32-unknown-unknown \
    && cargo install dioxus-cli --version 0.7.9 --locked

COPY . .

# One command builds the whole fullstack app: the wasm client (hydration
# bundle) and the native server binary that SSRs it. dx arranges the output as
# `.../web/{server, public/}` — the server binary next to its static assets.
RUN cd crates/web && dx build --platform web --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates gosu \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system stomatopod \
    && useradd --system --gid stomatopod --create-home --home-dir /home/stomatopod stomatopod

WORKDIR /app

# The server binary and the client bundle it renders/serves. `DIOXUS_PUBLIC_PATH`
# points the server at the bundle (index.html + hashed wasm/JS/CSS assets).
COPY --from=builder /app/target/dx/stomatopod/release/web/server /usr/local/bin/stomatopod
COPY --from=builder /app/target/dx/stomatopod/release/web/public /app/public
ENV DIOXUS_PUBLIC_PATH=/app/public

RUN mkdir -p /app/data \
    && chown -R stomatopod:stomatopod /app

# No `VOLUME /app/data` by design: an anonymous volume would make the data dir
# look "mounted" yet still be orphaned on redeploy, defeating the startup
# persistence guard. Mount a named volume / bind mount instead — see DEPLOY.md.

# Container starts as root (default, no USER here) so the entrypoint can chown
# a freshly mounted data dir at boot before dropping to the unprivileged
# `stomatopod` user via gosu. This matters because a bind-mounted host dir
# (e.g. Coolify's persistent storage) is typically root-owned and does not
# inherit this image's pre-chowned /app/data ownership at any mount path.
COPY --chmod=0755 docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]

EXPOSE 8080

ENV RUST_LOG=info

CMD ["stomatopod", "serve"]
