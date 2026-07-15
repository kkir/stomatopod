//! Native server entry point: CLI, config, storage bootstrap, and the
//! fullstack wiring that merges the Dioxus SSR application onto the REST
//! router ([`launch`]). Login HTML lives in `stomatopod-api`.

mod launch;

pub use launch::{bootstrap_self_hosted, run};
pub use stomatopod_api::html;
