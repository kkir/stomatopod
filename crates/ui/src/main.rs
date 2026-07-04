mod api;
mod app;
mod components;
mod pages;
mod query;
mod routes;
mod types;

use app::App;

fn main() {
    dioxus::launch(App);
}
