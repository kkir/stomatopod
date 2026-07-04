use dioxus::prelude::*;

use crate::components::layout::Shell;
use crate::pages::{
    Alerts, Campaigns, Compare, Docs, Events, FunnelDetail, Funnels, GlobalAlerts, GlobalGoals,
    GlobalRealtime, Goals, Keys, NotFound, Paths, Realtime, Retention, SiteKeys, SiteOverview,
    SiteSettings, SitesIndex,
};
use crate::query::DashQuery;

/// Mirrors `dashboard_routes` in crates/web/src/router.rs one-to-one,
/// minus the `/app` prefix (the SPA's `base_path`, `ui` during migration
/// and `app` after cutover). Global insight pages (`GlobalRealtime`,
/// `GlobalGoals`) carry an optional `site` selector, matching
/// `resolve_scope`'s fallback to the first site in `insights.rs`.
#[derive(Routable, Clone, PartialEq)]
pub enum Route {
    #[layout(Shell)]
    #[route("/")]
    SitesIndex {},

    #[route("/sites/:site_id?:..q")]
    SiteOverview { site_id: String, q: DashQuery },

    #[route("/sites/:site_id/realtime")]
    Realtime { site_id: String },

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

    #[route("/sites/:site_id/keys")]
    SiteKeys { site_id: String },

    #[route("/sites/:site_id/settings")]
    SiteSettings { site_id: String },

    #[route("/realtime?:site")]
    GlobalRealtime { site: Option<String> },

    #[route("/goals?:site")]
    GlobalGoals { site: Option<String> },

    #[route("/campaigns?:..q")]
    Campaigns { q: DashQuery },

    #[route("/retention?:..q")]
    Retention { q: DashQuery },

    #[route("/paths?:..q")]
    Paths { q: DashQuery },

    #[route("/compare?:..q")]
    Compare { q: DashQuery },

    #[route("/alerts")]
    GlobalAlerts {},

    #[route("/keys")]
    Keys {},

    #[route("/docs")]
    Docs {},

    // Catch-all for unknown paths (e.g. stale legacy `/app/sites` bookmarks):
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
            | Route::Campaigns { q }
            | Route::Retention { q }
            | Route::Paths { q }
            | Route::Compare { q } => Some(q),
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
            Route::Campaigns { .. } => Route::Campaigns { q },
            Route::Retention { .. } => Route::Retention { q },
            Route::Paths { .. } => Route::Paths { q },
            Route::Compare { .. } => Route::Compare { q },
            other => other.clone(),
        }
    }
}
