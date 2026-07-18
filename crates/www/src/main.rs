//! Marketing site entry point. `dx` compiles this crate twice:
//!
//! - to wasm with the `web` feature (hydrating client);
//! - to native with the `server` feature (SSG pre-render + optional local serve).

use dioxus::prelude::*;

fn main() {
    dioxus::LaunchBuilder::new()
        .with_cfg(server_only! {
            // Incremental renderer writes pre-rendered HTML for `dx build --ssg`.
            // Avoid out-of-order streaming: it has caused client-side
            // `RefCell already borrowed` panics during hydration on this app.
            ServeConfig::builder().incremental(
                dioxus::server::IncrementalRendererConfig::new()
                    .static_dir(
                        std::env::current_exe()
                            .unwrap()
                            .parent()
                            .unwrap()
                            .join("public"),
                    )
                    // Keep wasm/JS/CSS assets already placed in public/.
                    .clear_cache(false),
            )
        })
        .launch(stomatopod_www::App);
}
