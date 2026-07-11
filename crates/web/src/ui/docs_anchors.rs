//! Docs section anchors for deep links (`/docs#…`).
//! Must match heading slugs from `assets/docs.md` (see `slugify` in routes/api.rs).

pub const INSTALLING_THE_BROWSER_TRACKER: &str = "installing-the-browser-tracker";
pub const EMITTING_CUSTOM_EVENTS: &str = "emitting-custom-events";

pub fn docs_href(slug: &str) -> String {
    format!("/docs#{slug}")
}
