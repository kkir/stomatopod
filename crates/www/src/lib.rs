//! Stomatopod marketing site: Dioxus fullstack app pre-rendered with SSG for
//! static hosting (GitHub Pages). Shares design tokens and components with the
//! dashboard via `stomatopod-ui`.

mod app;
mod components;
mod pages;
mod routes;

pub use app::App;
pub use routes::Route;
