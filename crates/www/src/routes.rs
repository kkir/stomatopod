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
    /// `404/` — leaving that directory would make `/404/` a 200 page.
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
