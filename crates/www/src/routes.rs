use dioxus::prelude::*;

use crate::components::layout::MarketingShell;
use crate::pages::{Compare, Features, GetStarted, Home, NotFound};

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
