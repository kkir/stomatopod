# Shared toolchain base. This layer only busts when the toolchain block below
# changes, so the GHA layer cache restores it (and everything it installs)
# across CI runs instead of reinstalling. `dx` and cargo-chef come from
# cargo-binstall as prebuilt releases (seconds) rather than compiling from
# source (minutes, the old `cargo install dioxus-cli` cost).
FROM rust:1-bookworm AS chef

WORKDIR /app

RUN rustup target add wasm32-unknown-unknown \
    && curl -L --proto '=https' --tlsv1.2 -sSf \
        https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && cargo binstall cargo-chef dioxus-cli@0.7.9 --locked --no-confirm

# Capture the dependency graph. This stage sees the full source but its only
# output is recipe.json, so it busts only when Cargo.toml/Cargo.lock change —
# not on ordinary source edits.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

# The release profile (thin LTO, codegen-units=1) lives in .cargo/config.toml;
# it must be present during `cook` so the cached dependency artifacts share the
# fingerprint dx's build expects, otherwise they'd recompile.
COPY .cargo .cargo
COPY --from=planner /app/recipe.json recipe.json

# Compile just the dependencies against the recipe. This heavy layer (datafusion,
# arrow, parquet, sqlx, ...) is keyed on the recipe, so app-only changes reuse it
# from the GHA cache and skip straight to compiling our own crates.
# Native only: cooking the whole workspace for wasm32 would try to build
# datafusion for wasm and fail; the wasm client's own deps are small and compile
# during `dx build` below.
RUN cargo chef cook --release --recipe-path recipe.json

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
