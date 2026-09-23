use dioxus::prelude::*;

use crate::components::layout::MarketingShell;
use crate::pages::{Compare, Features, GetStarted, Home, NotFound, NotFoundPage};

/// Marketing site routes. All static segments are pre-rendered by SSG.
#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(MarketingShell)]
    #[route("/")]
    Home {},

    #[route("/features")]
    Features {},

    #[route("/get-started")]
    GetStarted {},

    #[route("/compare")]
    Compare {},

    /// Pre-rendered so the Pages artifact can ship a real `404.html`
    /// instead of copying the homepage. The artifact step then removes
    /// `404/` so `/404/` is a missing path (HTTP 404). GitHub Pages still
    /// serves the remaining `404.html` as HTTP 200 at `/404.html` and,
    /// via its clean-URL map, at `/404`. That 200 is a host limitation,
    /// not a second published page; see `scripts/prepare-www-artifact.sh`.
    #[route("/404")]
    NotFoundPage {},

    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

/// Called by `dx bundle --ssg` to discover routes to pre-render.
#[server(endpoint = "static_routes", output = server_fn::codec::Json)]
pub async fn static_routes() -> Result<Vec<String>, ServerFnError> {
    Ok(Route::static_routes()
        .iter()
        .map(ToString::to_string)
        .collect())
}
