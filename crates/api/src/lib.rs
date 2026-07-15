//! Stomatopod HTTP API: REST routes, auth middleware, digests, and OpenAPI.
//!
//! No Dioxus dashboard - that lives in `stomatopod-web`, which mounts this
//! router next to SSR.

pub mod digest;
pub mod error;
pub mod extractors;
pub mod html;
pub mod middleware;
pub mod openapi;
pub mod router;
pub mod routes;
pub mod state;

pub use router::build_router;
pub use state::AppState;
