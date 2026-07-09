use dioxus::prelude::*;

use crate::ui::components::layout::Shell;
use crate::ui::pages::{
    Alerts, Campaigns, Docs, Events, FunnelDetail, Funnels, Goals, Keys, NotFound, Paths, SiteKeys,
    SiteOverview, SiteSettings, SitesIndex,
};
use crate::ui::query::DashQuery;

/// Client-side routes for the Dioxus SPA. Site-scoped analytics pages
/// live under `/sites/:site_id/...`; global nav is Sites / API Keys / Docs.
#[derive(Routable, Clone, PartialEq)]
pub enum Route {
    #[layout(Shell)]
    #[route("/")]
    SitesIndex {},

    #[route("/sites/:site_id?:..q")]
    SiteOverview { site_id: String, q: DashQuery },

    #[route("/sites/:site_id/events?:..q")]
    Events { site_id: String, q: DashQuery },

    #[route("/sites/:site_id/goals?:..q")]
    Goals { site_id: String, q: DashQuery },

    #[route("/sites/:site_id/funnels?:..q")]
    Funnels { site_id: String, q: DashQuery },

    #[route("/sites/:site_id/funnels/:funnel_id?:..q")]
    FunnelDetail {
        site_id: String,
        funnel_id: String,
        q: DashQuery,
    },

    #[route("/sites/:site_id/alerts")]
    Alerts { site_id: String },

    #[route("/sites/:site_id/campaigns?:..q")]
    Campaigns { site_id: String, q: DashQuery },

    #[route("/sites/:site_id/paths?:..q")]
    Paths { site_id: String, q: DashQuery },

    #[route("/sites/:site_id/keys")]
    SiteKeys { site_id: String },

    #[route("/sites/:site_id/settings")]
    SiteSettings { site_id: String },

    #[route("/keys")]
    Keys {},

    #[route("/docs")]
    Docs {},

    // Catch-all for unknown paths (e.g. stale legacy bookmarks):
    // render a not-found page inside the shell rather than a blank screen.
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

impl Route {
    /// The [`DashQuery`] carried by this route, for routes that have one.
    /// Used by `RangeTabs` to read the currently active range/filters.
    pub fn query(&self) -> Option<&DashQuery> {
        match self {
            Route::SiteOverview { q, .. }
            | Route::Events { q, .. }
            | Route::Goals { q, .. }
            | Route::Funnels { q, .. }
            | Route::FunnelDetail { q, .. }
            | Route::Campaigns { q, .. }
            | Route::Paths { q, .. } => Some(q),
            _ => None,
        }
    }

    /// A copy of this route with its query replaced. Routes without a
    /// [`DashQuery`] are returned unchanged.
    pub fn with_query(&self, q: DashQuery) -> Route {
        match self {
            Route::SiteOverview { site_id, .. } => Route::SiteOverview {
                site_id: site_id.clone(),
                q,
            },
            Route::Events { site_id, .. } => Route::Events {
                site_id: site_id.clone(),
                q,
            },
            Route::Goals { site_id, .. } => Route::Goals {
                site_id: site_id.clone(),
                q,
            },
            Route::Funnels { site_id, .. } => Route::Funnels {
                site_id: site_id.clone(),
                q,
            },
            Route::FunnelDetail {
                site_id, funnel_id, ..
            } => Route::FunnelDetail {
                site_id: site_id.clone(),
                funnel_id: funnel_id.clone(),
                q,
            },
            Route::Campaigns { site_id, .. } => Route::Campaigns {
                site_id: site_id.clone(),
                q,
            },
            Route::Paths { site_id, .. } => Route::Paths {
                site_id: site_id.clone(),
                q,
            },
            other => other.clone(),
        }
    }
}
