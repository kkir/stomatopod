//! Stomatopod's web layer: a Dioxus fullstack application.
//!
//! One crate, two builds. The [`ui`] module (the Dioxus app, router, pages,
//! and components) is shared: `dx` compiles it to wasm for the browser (the
//! `web` feature) and the native server renders the same components server-side
//! (the `server` feature). Everything else here is server-only — the axum
//! handlers, background workers, and the fullstack wiring in [`server`].

pub mod ui;
pub use ui::App;

#[cfg(feature = "server")]
pub mod alerts;
#[cfg(feature = "server")]
pub mod digest;
#[cfg(feature = "server")]
pub mod error;
#[cfg(feature = "server")]
pub mod extractors;
#[cfg(feature = "server")]
pub mod middleware;
#[cfg(feature = "server")]
pub mod router;
#[cfg(feature = "server")]
pub mod routes;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
pub mod state;
