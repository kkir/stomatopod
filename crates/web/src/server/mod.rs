//! Native server entry point and its supporting pieces: the CLI + config +
//! storage bootstrap (formerly the `stomatopod` binary crate), server-rendered
//! HTML for the non-SPA pages ([`html`]), and the fullstack wiring that merges
//! the Dioxus SSR application onto the REST router ([`launch`]).

pub mod html;
mod launch;

pub use launch::{bootstrap_self_hosted, run};
