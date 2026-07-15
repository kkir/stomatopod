//! Marketing site entry point. `dx` compiles this crate twice:
//!
//! - to wasm with the `web` feature (hydrating client);
//! - to native with the `server` feature (SSG pre-render + optional local serve).

use dioxus::prelude::*;

fn main() {
    dioxus::LaunchBuilder::new()
        .with_cfg(server_only! {
            ServeConfig::builder()
                .incremental(
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
                .enable_out_of_order_streaming()
        })
        .launch(stomatopod_www::App);
}
