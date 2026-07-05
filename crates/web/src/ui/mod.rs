//! The Dioxus dashboard: app component, client-side router, pages, components,
//! and the JSON API DTOs.
//!
//! This module is compiled for BOTH targets. On wasm it is the hydrating
//! client; on the native server it is server-rendered (SSR) by
//! `dioxus-server`. Nothing here may reference a browser-only crate outside of
//! the `cfg(target_arch = "wasm32")` gate in [`api`], or the server build
//! breaks.

pub mod api;
pub mod app;
pub mod components;
pub mod pages;
pub mod query;
pub mod routes;
pub mod types;

pub use app::App;
