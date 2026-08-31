use dioxus::prelude::*;

use crate::seo::PageMeta;

/// Unique title, description, apex canonical, and Open Graph tags for one page.
#[component]
pub fn PageHead(meta: PageMeta, #[props(default = false)] json_ld: bool) -> Element {
    let canonical = meta.canonical();
    let json_ld_body = if json_ld {
        Some(crate::seo::home_json_ld())
    } else {
        None
    };

    rsx! {
        document::Title { "{meta.title}" }
        document::Meta { name: "description", content: "{meta.description}" }
        document::Link { rel: "canonical", href: "{canonical}" }
        document::Meta { property: "og:title", content: "{meta.title}" }
        document::Meta { property: "og:description", content: "{meta.description}" }
        document::Meta { property: "og:url", content: "{canonical}" }
        document::Meta { property: "og:type", content: "website" }
        if let Some(json_ld_body) = json_ld_body {
            document::Script { r#type: "application/ld+json", "{json_ld_body}" }
        }
    }
}
