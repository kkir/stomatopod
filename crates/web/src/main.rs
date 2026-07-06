//! Fullstack entry point. `dx` compiles this crate twice:
//!
//! - to wasm with the `web` feature — the client, which hydrates the
//!   server-rendered DOM;
//! - to native with the `server` feature — the axum server, which loads config,
//!   opens storage, and serves the SSR application plus the REST API.

// Client (browser) build: hydrate the app that the server rendered.
#[cfg(feature = "web")]
fn main() {
    dioxus::launch(stomatopod_web::App);
}

// Server build: run the analytics server. Gated `not(web)` so an accidental
// `--all-features` native build doesn't try to launch the wasm client.
#[cfg(all(feature = "server", not(feature = "web")))]
fn main() -> anyhow::Result<()> {
    stomatopod_web::server::run()
}

// A build with neither platform feature has nothing to run; fail loudly rather
// than producing a silent no-op binary.
#[cfg(not(any(feature = "web", feature = "server")))]
fn main() {
    compile_error!("enable exactly one of the `web` or `server` features");
}
