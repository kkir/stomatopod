use dioxus::prelude::*;

use crate::seo::{self, PageMeta};

/// Unique title, description, apex canonical, and Open Graph tags for one page.
#[component]
pub fn PageHead(meta: PageMeta, #[props(default = false)] json_ld: bool) -> Element {
    let canonical = meta.canonical();
    let og_image = meta.og_image();
    let og_width = seo::OG_IMAGE_WIDTH.to_string();
    let og_height = seo::OG_IMAGE_HEIGHT.to_string();
    let og_type = seo::OG_IMAGE_TYPE;
    let og_alt = seo::OG_IMAGE_ALT;
    let twitter_card = seo::TWITTER_CARD;
    let json_ld_body = if json_ld {
        Some(seo::home_json_ld())
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
        document::Meta { property: "og:image", content: "{og_image}" }
        document::Meta { property: "og:image:type", content: "{og_type}" }
        document::Meta { property: "og:image:width", content: "{og_width}" }
        document::Meta { property: "og:image:height", content: "{og_height}" }
        document::Meta { property: "og:image:alt", content: "{og_alt}" }
        document::Meta { name: "twitter:card", content: "{twitter_card}" }
        document::Meta { name: "twitter:image", content: "{og_image}" }
        if let Some(json_ld_body) = json_ld_body {
            document::Script { r#type: "application/ld+json", "{json_ld_body}" }
        }
    }
}
