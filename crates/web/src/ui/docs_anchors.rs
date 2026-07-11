//! Docs section anchors for deep links (`/docs#…`).
//! Must match heading slugs from `assets/docs.md` (see `slugify` in routes/api.rs).

pub const INSTALLING_THE_BROWSER_TRACKER: &str = "installing-the-browser-tracker";
pub const EMITTING_CUSTOM_EVENTS: &str = "emitting-custom-events";

pub fn docs_href(slug: &str) -> String {
    if slug.is_empty() {
        "/docs".into()
    } else {
        format!("/docs#{slug}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docs_href_formats_fragment() {
        assert_eq!(
            docs_href(INSTALLING_THE_BROWSER_TRACKER),
            "/docs#installing-the-browser-tracker"
        );
        assert_eq!(
            docs_href(EMITTING_CUSTOM_EVENTS),
            "/docs#emitting-custom-events"
        );
        assert_eq!(docs_href(""), "/docs");
    }
}
