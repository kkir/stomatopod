use dioxus::prelude::*;

use crate::routes::Route;

#[component]
pub fn App() -> Element {
    rsx! {
        document::Title { "Stomatopod" }
        document::Meta { name: "theme-color", content: "#04080b" }
        document::Link { rel: "preconnect", href: "https://fonts.googleapis.com" }
        document::Link { rel: "preconnect", href: "https://fonts.gstatic.com", crossorigin: "" }
        document::Link {
            rel: "stylesheet",
            href: "https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@500;600;700&display=swap",
        }
        document::Stylesheet { href: asset!("/assets/tailwind.css") }
        Router::<Route> {}
    }
}
