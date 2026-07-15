//! Stomatopod's web layer: a Dioxus fullstack application.
//!
//! One crate, two builds. The [`ui`] module (the Dioxus app, router, pages,
//! and components) is shared: `dx` compiles it to wasm for the browser (the
//! `web` feature) and the native server renders the same components server-side
//! (the `server` feature). Server-side HTTP (REST, auth, digests) lives in
//! `stomatopod-api`; alert evaluation in `stomatopod-alerts`. This crate owns
//! the dashboard UI and the process wiring that starts storage, workers, and
//! SSR.

pub mod ui;
pub use ui::App;

#[cfg(feature = "server")]
pub use stomatopod_alerts as alerts;
#[cfg(feature = "server")]
pub use stomatopod_api::{digest, error, extractors, middleware, openapi, router, routes, state};
#[cfg(feature = "server")]
pub mod server;
