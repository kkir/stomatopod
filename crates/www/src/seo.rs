//! Crawl metadata for the four public marketing URLs plus the static 404 page.
//!
//! Titles, descriptions, share-card image, and slash-canonical apex URLs live
//! here so the sitemap, robots.txt, and `<head>` tags cannot drift. GitHub
//! Pages may serve `www.stoma.top` only after a platform cert/DNS step; every
//! canonical still points at `https://stoma.top/…` so crawlers consolidate
//! on the apex even before that 301 exists.
//!
//! The 404 template is an error document, not a marketing URL: it is omitted
//! from the sitemap, emits `noindex, follow`, and does not claim a canonical.
//!
//! GitHub Pages cannot return HTTP 404 for a direct request to `404.html`
//! (or clean-URL `/404`) while that file is the custom error document.
//! `/404/` and unknown paths are the URLs that get a real 404 status.

/// Apex origin. Canonicals, the sitemap, and og:image always use this host.
pub const SITE_ORIGIN: &str = "https://stoma.top";

/// Default Open Graph / Twitter card. Stable path (not a Dioxus-hashed
/// asset) so the URL in `<head>` matches the file in `public/assets/`.
pub const OG_IMAGE_PATH: &str = "/assets/og.png";
pub const OG_IMAGE_WIDTH: u32 = 1200;
pub const OG_IMAGE_HEIGHT: u32 = 630;
pub const OG_IMAGE_TYPE: &str = "image/png";
pub const OG_IMAGE_ALT: &str = "Stomatopod mascot - a teal and purple mantis shrimp";
pub const TWITTER_CARD: &str = "summary_large_image";

/// Public pages listed in `sitemap.xml` (slash-canonical paths).
pub const PUBLIC_PAGES: &[PageMeta] = &[HOME, FEATURES, COMPARE, GET_STARTED];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageMeta {
    /// Slash-canonical path: `/` or `/features/` (never `/features`).
    pub path: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}

impl PageMeta {
    pub fn canonical(&self) -> String {
        format!("{SITE_ORIGIN}{}", self.path)
    }

    /// Absolute HTTPS share-card URL. Same default on every public page.
    pub fn og_image(&self) -> String {
        format!("{SITE_ORIGIN}{OG_IMAGE_PATH}")
    }

    /// Money pages are indexable. The 404 template is an error document.
    pub fn indexable(&self) -> bool {
        self.path != "/404/"
    }
}

/// Home: repeat self-hosted / cookieless / Docker / one binary so the Pages
/// URL can compete with the GitHub repo on those SERPs.
pub const HOME: PageMeta = PageMeta {
    path: "/",
    title: "Self-hosted cookieless analytics in one Docker binary - Stomatopod",
    description: "MIT open source, self-hosted, cookieless privacy analytics. One binary, Docker on a small VPS. A Plausible self-hosted and Umami alternative you run yourself.",
};

pub const FEATURES: PageMeta = PageMeta {
    path: "/features/",
    title: "Features: tracker, funnels, API, and CLI - Stomatopod",
    description: "Cookieless tracker, dashboard, funnels, alerts, REST API, and the stoma CLI. Self-hosted privacy analytics in one process.",
};

/// Compare: name the open-source / Plausible–Umami cluster without dropping
/// the operator framing (single binary, embedded storage, cookieless, MIT).
pub const COMPARE: PageMeta = PageMeta {
    path: "/compare/",
    title: "Open source Plausible/Umami alternative on a small VPS - Stomatopod",
    description: "MIT open source self-hosted analytics compared to Plausible CE, Umami, and GoatCounter. Cookieless, single binary, embedded storage on a small VPS.",
};

pub const GET_STARTED: PageMeta = PageMeta {
    path: "/get-started/",
    title: "Get started with Docker: self-hosted analytics - Stomatopod",
    description: "Install Stomatopod locally or with Docker Compose. One binary, cookieless privacy analytics, persistent volume, first-boot admin.",
};

/// Pre-rendered into root `404.html` (the Pages error document). Not a
/// public URL: `indexable()` is false, and the artifact step deletes `404/`
/// so `/404/` is a missing path (HTTP 404). `/404` and `/404.html` remain
/// host-level 200s of this same noindex body; see issue #60.
pub const NOT_FOUND: PageMeta = PageMeta {
    path: "/404/",
    title: "Page not found - Stomatopod",
    description: "That URL is not a page on stoma.top.",
};

/// Robots token for the error document. `follow` keeps link equity on
/// "Back home"; `noindex` stops the soft-404 from entering the index.
pub const NOT_FOUND_ROBOTS: &str = "noindex, follow";

/// SoftwareApplication JSON-LD for the homepage only.
pub fn home_json_ld() -> String {
    format!(
        r#"{{"@context":"https://schema.org","@type":"SoftwareApplication","name":"Stomatopod","url":"{url}","description":"{desc}","applicationCategory":"DeveloperApplication","operatingSystem":"Linux","offers":{{"@type":"Offer","price":"0","priceCurrency":"USD"}}}}"#,
        url = HOME.canonical(),
        desc = HOME.description,
    )
}

pub fn robots_txt() -> String {
    format!("User-agent: *\nAllow: /\n\nSitemap: {SITE_ORIGIN}/sitemap.xml\n")
}

pub fn sitemap_xml() -> String {
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for page in PUBLIC_PAGES {
        body.push_str("  <url>\n    <loc>");
        body.push_str(&page.canonical());
        body.push_str("</loc>\n  </url>\n");
    }
    body.push_str("</urlset>\n");
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn public_file(name: &str) -> String {
        let path = format!("{}/public/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read public/{name}: {e}"))
    }

    #[test]
    fn public_pages_have_unique_title_description_and_canonical() {
        let mut titles = HashSet::new();
        let mut descriptions = HashSet::new();
        let mut canonicals = HashSet::new();
        for page in PUBLIC_PAGES {
            assert!(titles.insert(page.title), "duplicate title: {}", page.title);
            assert!(
                descriptions.insert(page.description),
                "duplicate description: {}",
                page.description
            );
            let canonical = page.canonical();
            assert!(
                canonical.starts_with("https://stoma.top/"),
                "canonical must be apex HTTPS: {canonical}"
            );
            assert!(
                !canonical.contains("www."),
                "canonical must not use www: {canonical}"
            );
            if page.path != "/" {
                assert!(
                    page.path.starts_with('/') && page.path.ends_with('/'),
                    "slash-canonical path: {}",
                    page.path
                );
            }
            assert!(
                canonicals.insert(canonical),
                "duplicate canonical: {}",
                page.canonical()
            );
        }
        assert_eq!(PUBLIC_PAGES.len(), 4);
    }

    #[test]
    fn home_repeats_self_hosted_cookieless_docker_one_binary() {
        for hay in [HOME.title, HOME.description] {
            let lower = hay.to_ascii_lowercase();
            assert!(lower.contains("self-hosted"), "{hay}");
            assert!(lower.contains("cookieless"), "{hay}");
            assert!(lower.contains("docker"), "{hay}");
            assert!(
                lower.contains("one binary") || lower.contains("one docker binary"),
                "{hay}"
            );
        }
    }

    #[test]
    fn public_pages_have_absolute_og_image() {
        let mut images = HashSet::new();
        for page in PUBLIC_PAGES {
            let image = page.og_image();
            assert!(!image.is_empty(), "empty og:image for {}", page.path);
            assert!(
                image.starts_with("https://stoma.top/"),
                "og:image must be apex HTTPS: {image}"
            );
            assert!(
                !image.contains("www."),
                "og:image must not use www: {image}"
            );
            images.insert(image);
        }
        assert_eq!(images.len(), 1, "one default share-card URL for all pages");
        assert_eq!(
            PUBLIC_PAGES[0].og_image(),
            format!("{SITE_ORIGIN}{OG_IMAGE_PATH}")
        );
    }

    #[test]
    fn og_image_file_is_1200x630_png() {
        let path = format!("{}/public{OG_IMAGE_PATH}", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        assert!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "og:image must be a PNG"
        );
        assert_eq!(&bytes[12..16], b"IHDR");
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        assert_eq!(width, OG_IMAGE_WIDTH);
        assert_eq!(height, OG_IMAGE_HEIGHT);
        assert_eq!(OG_IMAGE_TYPE, "image/png");
        assert_eq!(TWITTER_CARD, "summary_large_image");
    }

    #[test]
    fn compare_title_names_open_source_alternative() {
        let title = COMPARE.title.to_ascii_lowercase();
        assert!(title.contains("open source"), "{}", COMPARE.title);
        assert!(
            title.contains("plausible") && title.contains("umami"),
            "{}",
            COMPARE.title
        );
        assert_eq!(
            COMPARE.title,
            "Open source Plausible/Umami alternative on a small VPS - Stomatopod"
        );
    }

    #[test]
    fn compare_description_mentions_license_and_peers() {
        let desc = COMPARE.description.to_ascii_lowercase();
        assert!(
            desc.contains("mit") || desc.contains("open source"),
            "{}",
            COMPARE.description
        );
        assert!(
            desc.contains("plausible") || desc.contains("umami"),
            "{}",
            COMPARE.description
        );
        assert!(desc.contains("cookieless"), "{}", COMPARE.description);
        assert!(desc.contains("single binary"), "{}", COMPARE.description);
        assert!(desc.contains("embedded storage"), "{}", COMPARE.description);
    }

    #[test]
    fn robots_txt_allows_root_and_names_sitemap() {
        let body = robots_txt();
        assert!(body.contains("User-agent: *"));
        assert!(body.contains("Allow: /"));
        assert!(body.contains("Sitemap: https://stoma.top/sitemap.xml"));
        assert_eq!(body, public_file("robots.txt"));
    }

    #[test]
    fn not_found_is_not_indexable_and_not_in_sitemap() {
        assert!(!NOT_FOUND.indexable(), "404 template must not be indexed");
        assert_eq!(NOT_FOUND_ROBOTS, "noindex, follow");
        // Host 200s at /404 and /404.html (Pages clean-URL + the error file)
        // must not be advertised as public URLs.
        for page in PUBLIC_PAGES {
            assert!(page.indexable(), "{} must stay indexable", page.path);
            assert!(
                !page.path.contains("404"),
                "public page path must not be a 404 URL: {}",
                page.path
            );
        }
        let sitemap = sitemap_xml();
        for needle in ["/404", "404.html", "404/"] {
            assert!(
                !sitemap.contains(needle),
                "sitemap must not list {needle}: {sitemap}"
            );
        }
        assert_eq!(sitemap, public_file("sitemap.xml"));
    }

    #[test]
    fn sitemap_xml_lists_slash_canonical_public_pages() {
        let body = sitemap_xml();
        assert!(body.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(body.contains("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">"));
        assert!(body.contains("</urlset>"));
        for page in PUBLIC_PAGES {
            let loc = format!("<loc>{}</loc>", page.canonical());
            assert!(body.contains(&loc), "missing {loc}");
        }
        assert!(
            !body.contains("www.stoma.top"),
            "sitemap must list apex URLs only"
        );
        assert_eq!(body, public_file("sitemap.xml"));
    }

    #[test]
    fn home_json_ld_is_software_application() {
        let json = home_json_ld();
        assert!(json.contains("\"@type\":\"SoftwareApplication\""));
        assert!(json.contains("\"@context\":\"https://schema.org\""));
        assert!(json.contains(&format!("\"url\":\"{}\"", HOME.canonical())));
        assert!(json.contains(&format!("\"description\":\"{}\"", HOME.description)));
    }
}
