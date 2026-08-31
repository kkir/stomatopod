//! Crawl metadata for the four public marketing URLs plus the static 404 page.
//!
//! Titles, descriptions, and slash-canonical apex URLs live here so the
//! sitemap, robots.txt, and `<head>` tags cannot drift. GitHub Pages may
//! serve `www.stoma.top` only after a platform cert/DNS step; every
//! canonical still points at `https://stoma.top/…` so crawlers consolidate
//! on the apex even before that 301 exists.

/// Apex origin. Canonicals and the sitemap always use this host, never `www`.
pub const SITE_ORIGIN: &str = "https://stoma.top";

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
}

/// Home: repeat self-hosted / cookieless / Docker / one binary so the Pages
/// URL can compete with the GitHub repo on those SERPs.
pub const HOME: PageMeta = PageMeta {
    path: "/",
    title: "Self-hosted cookieless analytics in one Docker binary - Stomatopod",
    description: "Self-hosted, cookieless privacy analytics. One binary, Docker on a small VPS. A Plausible self-hosted and Umami alternative you run yourself.",
};

pub const FEATURES: PageMeta = PageMeta {
    path: "/features/",
    title: "Features: tracker, funnels, API, and CLI - Stomatopod",
    description: "Cookieless tracker, dashboard, funnels, alerts, REST API, and the stoma CLI. Self-hosted privacy analytics in one process.",
};

/// Keep the compare title from the live page; unique description only (no
/// sitewide duplicate).
pub const COMPARE: PageMeta = PageMeta {
    path: "/compare/",
    title: "Plausible-class analytics on a small VPS - Stomatopod",
    description: "Operator notes for self-hosters: how Stomatopod compares to Plausible CE, Umami, and GoatCounter. Cookieless, single binary, embedded storage, MIT.",
};

pub const GET_STARTED: PageMeta = PageMeta {
    path: "/get-started/",
    title: "Get started with Docker: self-hosted analytics - Stomatopod",
    description: "Install Stomatopod locally or with Docker Compose. One binary, cookieless privacy analytics, persistent volume, first-boot admin.",
};

pub const NOT_FOUND: PageMeta = PageMeta {
    path: "/404/",
    title: "Page not found - Stomatopod",
    description: "That URL is not a page on stoma.top.",
};

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
    fn compare_keeps_live_title() {
        assert_eq!(
            COMPARE.title,
            "Plausible-class analytics on a small VPS - Stomatopod"
        );
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
