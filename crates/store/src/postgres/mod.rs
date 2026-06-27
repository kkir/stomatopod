//! Postgres-backed storage for SaaS deployments.
//!
//! Holds everything in one database — analytics events, agent spans,
//! and metadata (sites, orgs, users, funnels, AI-firewall config). Uses
//! `sqlx` with a connection pool and `INSERT ... ON CONFLICT` for
//! upserts.
//!
//! Schema is created lazily via `bootstrap()`; idempotent and safe to
//! re-run on every boot. Migrations are kept inline so they stay close
//! to the impl — when this grows beyond a couple of tables we should
//! switch to `sqlx::migrate!`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool, QueryBuilder, Row};
use ulid::Ulid;

use stomatopod_core::{
    config::PostgresConfig,
    domain::{
        agent::{Agent, AlertChannel, AlertChannelKind, SentinelToken},
        agent_span::AgentSpan,
        analytics_alert::{AnalyticsAlert, AnalyticsAlertFire, AnalyticsAlertKind},
        annotation::Annotation,
        api_key::{ApiKey, ApiKeyScope},
        digest::{DigestFrequency, DigestSubscription},
        event::Event,
        goal::Goal,
        incident::{Incident, IncidentStatus, IncidentTrigger},
        org::{Funnel, Organization, Plan, User, UserRole},
        policy::Policy,
        share_link::ShareLink,
        site::Site,
    },
    error::StoreError,
    query::{
        analytics::{
            EntryPageRow, EntryPages, ExitPageRow, ExitPages, GoalBucket, GoalQuery, GoalStats,
            PathReport, RawEventRow, RealtimeEvent, RealtimeSnapshot, RealtimeTopPage,
            RetentionGrid, SessionRow, TopSparklines,
        },
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult, FunnelStepResult},
        pageviews::{
            Filter, FilterOp, Granularity, PageviewsQuery, PageviewsResult, TimeBucket, TimeRange,
            TopList, TopListField, TopRow,
        },
        spans::{AgentSummary, SpanQuery, SpanRow},
    },
    traits::{AgentStore, MetaStore, StorageBackend},
};

pub struct PostgresBackend {
    pool: PgPool,
}

impl PostgresBackend {
    pub async fn connect(cfg: &PostgresConfig) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .connect(&cfg.url)
            .await?;
        Ok(Self { pool })
    }

    /// Create every table this backend uses if it does not exist —
    /// metadata tables first (some have FKs into each other), then
    /// the analytics tables. Idempotent; safe to call on every boot.
    pub async fn bootstrap(&self) -> Result<(), StoreError> {
        for stmt in ddl::all_statements()
            .iter()
            .chain(ddl::analytics_statements().iter())
        {
            sqlx::query(stmt)
                .execute(&self.pool)
                .await
                .map_err(StoreError::db)?;
        }
        Ok(())
    }
}

fn parse_ulid(s: &str) -> Ulid {
    Ulid::from_string(s).unwrap_or_default()
}

#[async_trait]
impl StorageBackend for PostgresBackend {
    async fn ingest_events(&self, events: Vec<Event>) -> Result<(), StoreError> {
        if events.is_empty() {
            return Ok(());
        }
        // Bulk INSERT in chunks that respect Postgres's 32 767 parameter
        // limit per statement. With 27 columns we cap each chunk at
        // ~1 200 rows — well under the limit while still collapsing
        // thousands of round-trips into a handful.
        //
        // All chunks run inside a single transaction so a multi-chunk
        // batch is atomic: a failure on the second chunk doesn't leave
        // the first one durably committed, which would otherwise force
        // callers to dedupe on retry.
        const COLUMNS: usize = 27;
        const MAX_PARAMS: usize = 32_767;
        let chunk_size = MAX_PARAMS / COLUMNS;

        let mut tx = self.pool.begin().await.map_err(StoreError::db)?;
        for chunk in events.chunks(chunk_size) {
            let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
                "INSERT INTO events (\
                    id, site_id, name, kind, timestamp, received_at, url, referrer, \
                    utm_source, utm_medium, utm_campaign, utm_term, utm_content, \
                    browser, browser_version, os, os_version, device_type, \
                    screen_width, screen_height, language, \
                    ip_anonymized, country_code, region, city, \
                    session_id, properties) ",
            );
            qb.push_values(chunk, |mut b, e| {
                b.push_bind(e.id.to_string())
                    .push_bind(e.site_id.to_string())
                    .push_bind(&e.name)
                    .push_bind(e.kind.as_str())
                    .push_bind(e.timestamp)
                    .push_bind(e.received_at)
                    .push_bind(&e.url)
                    .push_bind(&e.referrer)
                    .push_bind(&e.utm_source)
                    .push_bind(&e.utm_medium)
                    .push_bind(&e.utm_campaign)
                    .push_bind(&e.utm_term)
                    .push_bind(&e.utm_content)
                    .push_bind(&e.browser)
                    .push_bind(&e.browser_version)
                    .push_bind(&e.os)
                    .push_bind(&e.os_version)
                    .push_bind(e.device_type.as_str())
                    .push_bind(e.screen_width.map(|v| v as i32))
                    .push_bind(e.screen_height.map(|v| v as i32))
                    .push_bind(&e.language)
                    .push_bind(&e.ip_anonymized)
                    .push_bind(&e.country_code)
                    .push_bind(&e.region)
                    .push_bind(&e.city)
                    .push_bind(&e.session_id[..])
                    .push_bind(e.properties.as_ref().map(|p| p.to_string()));
            });
            qb.build().execute(&mut *tx).await.map_err(StoreError::db)?;
        }
        tx.commit().await.map_err(StoreError::db)?;
        Ok(())
    }

    async fn query_pageviews(&self, q: &PageviewsQuery) -> Result<PageviewsResult, StoreError> {
        let bucket = pg_date_trunc(&q.granularity);
        let (filter_sql, filter_vals) = pg_filter_clause(&q.filters, 4);
        let sql = format!(
            "SELECT date_trunc('{bucket}', timestamp) AS bucket, \
                    COUNT(*) FILTER (WHERE kind = 'pageview')::BIGINT AS pageviews, \
                    COUNT(DISTINCT session_id)::BIGINT AS sessions \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 {filter_sql} \
             GROUP BY 1 ORDER BY 1"
        );
        let mut query = sqlx::query(&sql)
            .bind(q.site_id.to_string())
            .bind(q.range.start)
            .bind(q.range.end);
        for val in &filter_vals {
            query = query.bind(val.clone());
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        let mut result = PageviewsResult::default();
        for row in rows {
            let ts: DateTime<Utc> = row.try_get("bucket").map_err(StoreError::query)?;
            let pv: i64 = row.try_get("pageviews").map_err(StoreError::query)?;
            let sess: i64 = row.try_get("sessions").map_err(StoreError::query)?;
            let pv = pv as u64;
            let sess = sess as u64;
            result.total_pageviews += pv;
            result.total_sessions += sess;
            result.buckets.push(TimeBucket {
                ts,
                pageviews: pv,
                sessions: sess,
            });
        }
        Ok(result)
    }

    async fn query_top_list(
        &self,
        site_id: Ulid,
        field: TopListField,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, field.column(), filters)
            .await
    }

    async fn query_custom_events(&self, q: &EventQuery) -> Result<TopList, StoreError> {
        let name_filter = if q.event_name.is_some() {
            "AND name = $4"
        } else {
            ""
        };
        let sql = format!(
            "SELECT name AS value, \
                    COUNT(*)::BIGINT AS pageviews, \
                    COUNT(DISTINCT session_id)::BIGINT AS sessions \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
               AND kind = 'custom' {name_filter} \
             GROUP BY 1 ORDER BY pageviews DESC \
             LIMIT {}",
            q.limit,
        );
        let mut query = sqlx::query(&sql)
            .bind(q.site_id.to_string())
            .bind(q.range.start)
            .bind(q.range.end);
        if let Some(name) = q.event_name.as_deref() {
            query = query.bind(name.to_string());
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        Ok(top_list_from_rows(rows)?)
    }

    async fn query_funnel(&self, q: &FunnelQuery) -> Result<FunnelResult, StoreError> {
        if q.steps.is_empty() {
            return Ok(FunnelResult::default());
        }
        let mut step_results = Vec::with_capacity(q.steps.len());
        let mut prev_sessions: Option<u64> = None;
        for step in &q.steps {
            let row = sqlx::query(
                "SELECT COUNT(DISTINCT session_id)::BIGINT AS sessions \
                 FROM events \
                 WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
                   AND name = $4",
            )
            .bind(q.site_id.to_string())
            .bind(q.range.start)
            .bind(q.range.end)
            .bind(&step.event_name)
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::query)?;
            let sessions: i64 = row.try_get("sessions").map_err(StoreError::query)?;
            let sessions = sessions as u64;
            let (cr, drop) = match prev_sessions {
                None => (1.0, 0.0),
                Some(prev) if prev > 0 => {
                    let cr = sessions as f64 / prev as f64;
                    (cr, 1.0 - cr)
                }
                _ => (0.0, 1.0),
            };
            step_results.push(FunnelStepResult {
                name: step.name.clone(),
                sessions,
                conversion_rate: cr,
                drop_off_rate: drop,
            });
            prev_sessions = Some(sessions);
        }
        Ok(FunnelResult {
            steps: step_results,
        })
    }

    async fn query_entry_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<EntryPages, StoreError> {
        let (filter_sql, filter_vals) = pg_filter_clause(filters, 4);
        let sql = format!(
            "WITH ranked AS (\
                SELECT url, \
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY timestamp ASC, id ASC) AS rn, \
                    COUNT(*) OVER (PARTITION BY session_id) AS pv_count \
                FROM events \
                WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
                  AND kind = 'pageview' {filter_sql}) \
             SELECT COALESCE(url, '') AS value, COUNT(*)::BIGINT AS sessions, \
                    SUM(CASE WHEN pv_count = 1 THEN 1 ELSE 0 END)::BIGINT AS bounces \
             FROM ranked WHERE rn = 1 GROUP BY url ORDER BY sessions DESC LIMIT {limit}"
        );
        let mut query = sqlx::query(&sql)
            .bind(site_id.to_string())
            .bind(range.start)
            .bind(range.end);
        for v in &filter_vals {
            query = query.bind(v.clone());
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        let mut out = Vec::with_capacity(rows.len());
        let mut total: u64 = 0;
        for row in rows {
            let url: String = row.try_get("value").map_err(StoreError::query)?;
            let sessions: i64 = row.try_get("sessions").map_err(StoreError::query)?;
            let bounces: i64 = row.try_get("bounces").map_err(StoreError::query)?;
            let sessions = sessions as u64;
            total += sessions;
            let bounce_rate = if sessions > 0 {
                bounces as f64 / sessions as f64 * 100.0
            } else {
                0.0
            };
            out.push(EntryPageRow {
                url,
                sessions,
                pct: 0.0,
                bounce_rate,
            });
        }
        if total > 0 {
            for r in &mut out {
                r.pct = r.sessions as f64 / total as f64 * 100.0;
            }
        }
        Ok(EntryPages { rows: out })
    }

    async fn query_exit_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<ExitPages, StoreError> {
        let (filter_sql, filter_vals) = pg_filter_clause(filters, 4);
        let sql = format!(
            "WITH ranked AS (\
                SELECT url, \
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY timestamp DESC, id DESC) AS rn \
                FROM events \
                WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
                  AND kind = 'pageview' {filter_sql}) \
             SELECT COALESCE(url, '') AS value, \
                    SUM(CASE WHEN rn = 1 THEN 1 ELSE 0 END)::BIGINT AS exits, \
                    COUNT(*)::BIGINT AS pageviews \
             FROM ranked GROUP BY url HAVING SUM(CASE WHEN rn = 1 THEN 1 ELSE 0 END) > 0 \
             ORDER BY exits DESC LIMIT {limit}"
        );
        let mut query = sqlx::query(&sql)
            .bind(site_id.to_string())
            .bind(range.start)
            .bind(range.end);
        for v in &filter_vals {
            query = query.bind(v.clone());
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        let mut out = Vec::with_capacity(rows.len());
        let mut total: u64 = 0;
        for row in rows {
            let url: String = row.try_get("value").map_err(StoreError::query)?;
            let exits: i64 = row.try_get("exits").map_err(StoreError::query)?;
            let pageviews: i64 = row.try_get("pageviews").map_err(StoreError::query)?;
            let exits = exits as u64;
            total += exits;
            let exit_rate = if pageviews > 0 {
                exits as f64 / pageviews as f64 * 100.0
            } else {
                0.0
            };
            out.push(ExitPageRow {
                url,
                exits,
                pct: 0.0,
                exit_rate,
            });
        }
        if total > 0 {
            for r in &mut out {
                r.pct = r.exits as f64 / total as f64 * 100.0;
            }
        }
        Ok(ExitPages { rows: out })
    }

    async fn query_realtime(
        &self,
        site_id: Ulid,
        window_minutes: u32,
    ) -> Result<RealtimeSnapshot, StoreError> {
        let window = window_minutes.max(1);
        let since = format!("NOW() - INTERVAL '{window} minutes'");

        let head = sqlx::query(&format!(
            "SELECT COUNT(DISTINCT session_id)::BIGINT AS active, \
                    COUNT(*) FILTER (WHERE kind = 'pageview')::BIGINT AS pvs \
             FROM events WHERE site_id = $1 AND timestamp >= {since}"
        ))
        .bind(site_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::query)?;
        let active: i64 = head.try_get("active").map_err(StoreError::query)?;
        let pvs: i64 = head.try_get("pvs").map_err(StoreError::query)?;
        let active_sessions = active as u64;
        let pageviews_per_minute = pvs as f64 / window as f64;

        let page_rows = sqlx::query(&format!(
            "WITH ranked AS (\
                SELECT url, session_id, \
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY timestamp DESC, id DESC) AS rn \
                FROM events WHERE site_id = $1 AND timestamp >= {since} AND kind = 'pageview') \
             SELECT COALESCE(url, '') AS value, COUNT(DISTINCT session_id)::BIGINT AS active \
             FROM ranked WHERE rn = 1 GROUP BY url ORDER BY active DESC LIMIT 10"
        ))
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::query)?;
        let mut top_pages = Vec::new();
        for row in page_rows {
            let url: String = row.try_get("value").map_err(StoreError::query)?;
            let a: i64 = row.try_get("active").map_err(StoreError::query)?;
            let a = a as u64;
            let pct = if active_sessions > 0 {
                a as f64 / active_sessions as f64 * 100.0
            } else {
                0.0
            };
            top_pages.push(RealtimeTopPage {
                url,
                active_sessions: a,
                pct,
            });
        }

        let ev_rows = sqlx::query(&format!(
            "SELECT name, COALESCE(url, '') AS url, timestamp, properties \
             FROM events WHERE site_id = $1 AND timestamp >= {since} AND kind = 'custom' \
             ORDER BY timestamp DESC LIMIT 50"
        ))
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::query)?;
        let now = Utc::now();
        let mut recent_events = Vec::new();
        for row in ev_rows {
            let name: String = row.try_get("name").map_err(StoreError::query)?;
            let url: String = row.try_get("url").map_err(StoreError::query)?;
            let ts: DateTime<Utc> = row.try_get("timestamp").map_err(StoreError::query)?;
            let props: Option<String> = row.try_get("properties").map_err(StoreError::query)?;
            recent_events.push(RealtimeEvent {
                name,
                url,
                seconds_ago: (now - ts).num_seconds().max(0),
                properties: props
                    .and_then(|p| serde_json::from_str(&p).ok())
                    .unwrap_or(serde_json::Value::Null),
            });
        }

        Ok(RealtimeSnapshot {
            active_sessions,
            pageviews_per_minute,
            top_pages,
            recent_events,
        })
    }

    async fn query_goal(&self, q: &GoalQuery) -> Result<GoalStats, StoreError> {
        let (filter_sql, filter_vals) = pg_filter_clause(&q.filters, 5);
        let bucket = pg_date_trunc(&q.granularity);

        // Headline completions.
        let totals_sql = format!(
            "SELECT COUNT(*)::BIGINT AS completions, COUNT(DISTINCT session_id)::BIGINT AS uniq \
             FROM events WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
               AND kind = 'custom' AND name = $4 {filter_sql}"
        );
        let mut tq = sqlx::query(&totals_sql)
            .bind(q.site_id.to_string())
            .bind(q.range.start)
            .bind(q.range.end)
            .bind(&q.event_name);
        for v in &filter_vals {
            tq = tq.bind(v.clone());
        }
        let trow = tq.fetch_one(&self.pool).await.map_err(StoreError::query)?;
        let completions: i64 = trow.try_get("completions").map_err(StoreError::query)?;
        let uniq: i64 = trow.try_get("uniq").map_err(StoreError::query)?;
        let unique_completions = uniq as u64;

        let srow = sqlx::query(
            "SELECT COUNT(DISTINCT session_id)::BIGINT AS sessions FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3",
        )
        .bind(q.site_id.to_string())
        .bind(q.range.start)
        .bind(q.range.end)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::query)?;
        let total_sessions: i64 = srow.try_get("sessions").map_err(StoreError::query)?;
        let conversion_rate = if total_sessions > 0 {
            unique_completions as f64 / total_sessions as f64 * 100.0
        } else {
            0.0
        };

        let ts_sql = format!(
            "WITH comp AS (\
                SELECT date_trunc('{bucket}', timestamp) AS b, COUNT(DISTINCT session_id) AS uniq \
                FROM events WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
                  AND kind = 'custom' AND name = $4 {filter_sql} GROUP BY 1), \
             sess AS (\
                SELECT date_trunc('{bucket}', timestamp) AS b, COUNT(DISTINCT session_id) AS sessions \
                FROM events WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 GROUP BY 1) \
             SELECT comp.b AS bucket, comp.uniq::BIGINT AS uniq, \
                    COALESCE(sess.sessions, 0)::BIGINT AS sessions \
             FROM comp LEFT JOIN sess ON comp.b = sess.b ORDER BY comp.b"
        );
        let mut tsq = sqlx::query(&ts_sql)
            .bind(q.site_id.to_string())
            .bind(q.range.start)
            .bind(q.range.end)
            .bind(&q.event_name);
        for v in &filter_vals {
            tsq = tsq.bind(v.clone());
        }
        let ts_rows = tsq.fetch_all(&self.pool).await.map_err(StoreError::query)?;
        let mut timeseries = Vec::new();
        for row in ts_rows {
            let b: DateTime<Utc> = row.try_get("bucket").map_err(StoreError::query)?;
            let c: i64 = row.try_get("uniq").map_err(StoreError::query)?;
            let s: i64 = row.try_get("sessions").map_err(StoreError::query)?;
            let cr = if s > 0 {
                c as f64 / s as f64 * 100.0
            } else {
                0.0
            };
            timeseries.push(GoalBucket {
                date: b.format("%Y-%m-%d").to_string(),
                completions: c as u64,
                conversion_rate: cr,
            });
        }

        Ok(GoalStats {
            completions: completions as u64,
            unique_completions,
            conversion_rate,
            timeseries,
        })
    }

    async fn query_sessions(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<SessionRow>, StoreError> {
        let sql = format!(
            "WITH s AS (\
                SELECT session_id, timestamp AS ts, url, referrer, country_code, browser, os, \
                    device_type, utm_source, utm_medium, utm_campaign, \
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY timestamp ASC, id ASC) AS rn_first, \
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY timestamp DESC, id DESC) AS rn_last \
                FROM events WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 AND kind = 'pageview') \
             SELECT encode(session_id, 'hex') AS session_id, MIN(ts) AS started, MAX(ts) AS ended, \
                    COUNT(*)::BIGINT AS pageviews, \
                    MAX(url) FILTER (WHERE rn_first = 1) AS entry_url, \
                    MAX(url) FILTER (WHERE rn_last = 1) AS exit_url, \
                    MAX(referrer) FILTER (WHERE rn_first = 1) AS referrer, \
                    MAX(country_code) FILTER (WHERE rn_first = 1) AS country_code, \
                    MAX(browser) FILTER (WHERE rn_first = 1) AS browser, \
                    MAX(os) FILTER (WHERE rn_first = 1) AS os, \
                    MAX(device_type) FILTER (WHERE rn_first = 1) AS device_type, \
                    MAX(utm_source) FILTER (WHERE rn_first = 1) AS utm_source, \
                    MAX(utm_medium) FILTER (WHERE rn_first = 1) AS utm_medium, \
                    MAX(utm_campaign) FILTER (WHERE rn_first = 1) AS utm_campaign \
             FROM s GROUP BY session_id ORDER BY started DESC LIMIT {limit}"
        );
        let rows = sqlx::query(&sql)
            .bind(site_id.to_string())
            .bind(range.start)
            .bind(range.end)
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let started: DateTime<Utc> = row.try_get("started").map_err(StoreError::query)?;
            let ended: DateTime<Utc> = row.try_get("ended").map_err(StoreError::query)?;
            let pageviews: i64 = row.try_get("pageviews").map_err(StoreError::query)?;
            let pageviews = pageviews as u64;
            let duration_secs = (ended - started).num_seconds();
            out.push(SessionRow {
                session_id: row.try_get("session_id").map_err(StoreError::query)?,
                started_at: started,
                ended_at: ended,
                duration_secs,
                pageviews,
                entry_url: row
                    .try_get::<Option<String>, _>("entry_url")
                    .map_err(StoreError::query)?
                    .unwrap_or_default(),
                exit_url: row
                    .try_get::<Option<String>, _>("exit_url")
                    .map_err(StoreError::query)?
                    .unwrap_or_default(),
                referrer: row.try_get("referrer").map_err(StoreError::query)?,
                country_code: row.try_get("country_code").map_err(StoreError::query)?,
                browser: row
                    .try_get::<Option<String>, _>("browser")
                    .map_err(StoreError::query)?
                    .unwrap_or_default(),
                os: row
                    .try_get::<Option<String>, _>("os")
                    .map_err(StoreError::query)?
                    .unwrap_or_default(),
                device_type: row
                    .try_get::<Option<String>, _>("device_type")
                    .map_err(StoreError::query)?
                    .unwrap_or_default(),
                utm_source: row.try_get("utm_source").map_err(StoreError::query)?,
                utm_medium: row.try_get("utm_medium").map_err(StoreError::query)?,
                utm_campaign: row.try_get("utm_campaign").map_err(StoreError::query)?,
                is_bounce: pageviews == 1 && duration_secs < 30,
            });
        }
        Ok(out)
    }

    async fn query_events_list(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<RawEventRow>, StoreError> {
        let sql = format!(
            "SELECT id, name, kind, timestamp, COALESCE(url, '') AS url, referrer, country_code, \
                    browser, os, device_type, properties \
             FROM events WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
             ORDER BY timestamp DESC LIMIT {limit}"
        );
        let rows = sqlx::query(&sql)
            .bind(site_id.to_string())
            .bind(range.start)
            .bind(range.end)
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(RawEventRow {
                id: row.try_get("id").map_err(StoreError::query)?,
                name: row.try_get("name").map_err(StoreError::query)?,
                kind: row.try_get("kind").map_err(StoreError::query)?,
                timestamp: row.try_get("timestamp").map_err(StoreError::query)?,
                url: row.try_get("url").map_err(StoreError::query)?,
                referrer: row.try_get("referrer").map_err(StoreError::query)?,
                country_code: row.try_get("country_code").map_err(StoreError::query)?,
                browser: row
                    .try_get::<Option<String>, _>("browser")
                    .map_err(StoreError::query)?
                    .unwrap_or_default(),
                os: row
                    .try_get::<Option<String>, _>("os")
                    .map_err(StoreError::query)?
                    .unwrap_or_default(),
                device_type: row
                    .try_get::<Option<String>, _>("device_type")
                    .map_err(StoreError::query)?
                    .unwrap_or_default(),
                properties: row.try_get("properties").map_err(StoreError::query)?,
            });
        }
        Ok(out)
    }

    async fn query_top_sparklines(
        &self,
        site_id: Ulid,
        field: TopListField,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<TopSparklines, StoreError> {
        let (filter_sql, filter_vals) = pg_filter_clause(filters, 4);
        let col = field.column();
        let sql = format!(
            "SELECT COALESCE({col}, 'Direct / None') AS value, \
                    to_char(date_trunc('day', timestamp), 'YYYY-MM-DD') AS day, \
                    COUNT(*)::BIGINT AS c \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
               AND kind = 'pageview' {filter_sql} \
             GROUP BY 1, 2"
        );
        let mut q = sqlx::query(&sql)
            .bind(site_id.to_string())
            .bind(range.start)
            .bind(range.end);
        for v in &filter_vals {
            q = q.bind(v.clone());
        }
        let rows = q.fetch_all(&self.pool).await.map_err(StoreError::query)?;
        let mut out = Vec::with_capacity(rows.len());
        let mut days = std::collections::BTreeSet::new();
        for row in rows {
            let value: String = row.try_get("value").map_err(StoreError::query)?;
            let day: String = row.try_get("day").map_err(StoreError::query)?;
            let c: i64 = row.try_get("c").map_err(StoreError::query)?;
            days.insert(day.clone());
            out.push((value, day, c.max(0) as u64));
        }
        Ok(TopSparklines::from_counts(
            out,
            days.into_iter().collect(),
            limit as usize,
        ))
    }

    async fn query_retention(
        &self,
        site_id: Ulid,
        range: &TimeRange,
    ) -> Result<RetentionGrid, StoreError> {
        let rows = sqlx::query(
            "SELECT DISTINCT encode(session_id, 'hex') AS sid, \
                    to_char(date_trunc('week', timestamp), 'YYYY-MM-DD') AS week \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 AND kind = 'pageview'",
        )
        .bind(site_id.to_string())
        .bind(range.start)
        .bind(range.end)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::query)?;
        let mut pairs = Vec::with_capacity(rows.len());
        for row in rows {
            let sid: String = row.try_get("sid").map_err(StoreError::query)?;
            let week: String = row.try_get("week").map_err(StoreError::query)?;
            pairs.push((sid, week));
        }
        Ok(RetentionGrid::from_session_weeks(pairs))
    }

    async fn query_paths(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        depth: u32,
        limit: u32,
    ) -> Result<PathReport, StoreError> {
        let depth = depth.clamp(2, 10);
        let sql = format!(
            "WITH ranked AS (\
                SELECT encode(session_id, 'hex') AS sid, COALESCE(url, '') AS url, \
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY timestamp ASC, id ASC) AS rn \
                FROM events \
                WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 AND kind = 'pageview') \
             SELECT sid, rn::BIGINT AS seq, url FROM ranked WHERE rn <= {depth}"
        );
        let rows = sqlx::query(&sql)
            .bind(site_id.to_string())
            .bind(range.start)
            .bind(range.end)
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        let mut steps = Vec::with_capacity(rows.len());
        for row in rows {
            let sid: String = row.try_get("sid").map_err(StoreError::query)?;
            let seq: i64 = row.try_get("seq").map_err(StoreError::query)?;
            let url: String = row.try_get("url").map_err(StoreError::query)?;
            steps.push((sid, seq.max(0) as u32, url));
        }
        Ok(PathReport::from_steps(steps, limit as usize))
    }
}

impl PostgresBackend {
    async fn query_top_field(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        field: &str,
        filters: &[Filter],
    ) -> Result<TopList, StoreError> {
        // `field` is a static identifier from the trait surface, never
        // user input — see top-level trait docs. Filter values are bound as
        // parameters ($4+), so they're injection-safe too.
        let (filter_sql, filter_vals) = pg_filter_clause(filters, 4);
        let sql = format!(
            "SELECT COALESCE({field}, 'Direct / None') AS value, \
                    COUNT(*)::BIGINT AS pageviews, \
                    COUNT(DISTINCT session_id)::BIGINT AS sessions \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
               AND kind = 'pageview' {filter_sql} \
             GROUP BY 1 ORDER BY pageviews DESC \
             LIMIT {limit}"
        );
        let mut query = sqlx::query(&sql)
            .bind(site_id.to_string())
            .bind(range.start)
            .bind(range.end);
        for val in &filter_vals {
            query = query.bind(val.clone());
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::query)?;
        top_list_from_rows(rows)
    }
}

/// Build the `AND <col> <op> $N` fragment for analytics filters, plus the
/// ordered list of values to bind. `start_idx` is the first placeholder
/// number (after the fixed site/start/end binds). Columns are static enum
/// values; values are always bound, never interpolated.
fn pg_filter_clause(filters: &[Filter], start_idx: usize) -> (String, Vec<String>) {
    let mut frag = String::new();
    let mut vals = Vec::with_capacity(filters.len());
    for (i, f) in filters.iter().enumerate() {
        let idx = start_idx + i;
        frag.push_str(&format!(
            " AND {} {} ${idx}",
            f.field.column(),
            f.op.sql_operator()
        ));
        if matches!(f.op, FilterOp::Contains | FilterOp::StartsWith) {
            frag.push_str(" ESCAPE '\\'");
        }
        vals.push(f.sql_value());
    }
    (frag, vals)
}

fn pg_date_trunc(g: &Granularity) -> &'static str {
    match g {
        Granularity::Hour => "hour",
        Granularity::Day => "day",
        Granularity::Week => "week",
        Granularity::Month => "month",
    }
}

fn top_list_from_rows(rows: Vec<sqlx::postgres::PgRow>) -> Result<TopList, StoreError> {
    let mut out: Vec<TopRow> = Vec::with_capacity(rows.len());
    for row in rows {
        let value: Option<String> = row.try_get("value").map_err(StoreError::query)?;
        let pv: i64 = row.try_get("pageviews").map_err(StoreError::query)?;
        let sess: i64 = row.try_get("sessions").map_err(StoreError::query)?;
        out.push(TopRow {
            value: value.unwrap_or_else(|| "Direct / None".into()),
            pageviews: pv as u64,
            sessions: sess as u64,
            pct: 0.0,
        });
    }
    let total: u64 = out.iter().map(|r| r.pageviews).sum();
    if total > 0 {
        for row in &mut out {
            row.pct = (row.pageviews as f64 / total as f64) * 100.0;
        }
    }
    Ok(TopList { rows: out })
}

#[async_trait]
impl MetaStore for PostgresBackend {
    async fn create_site(&self, site: &Site) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sites (id, org_id, domain, name, timezone, public_key, created_at, is_active) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(site.id.to_string())
        .bind(site.org_id.to_string())
        .bind(&site.domain)
        .bind(&site.name)
        .bind(&site.timezone)
        .bind(&site.public_key)
        .bind(site.created_at)
        .bind(site.is_active)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn update_site(&self, site: &Site) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE sites
             SET org_id = $2, domain = $3, name = $4, timezone = $5, public_key = $6, created_at = $7, is_active = $8
             WHERE id = $1",
        )
        .bind(site.id.to_string())
        .bind(site.org_id.to_string())
        .bind(&site.domain)
        .bind(&site.name)
        .bind(&site.timezone)
        .bind(&site.public_key)
        .bind(site.created_at)
        .bind(site.is_active)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_site(&self, id: Ulid) -> Result<Option<Site>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active \
             FROM sites WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_site).transpose()
    }

    async fn get_site_by_key(&self, public_key: &str) -> Result<Option<Site>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active \
             FROM sites WHERE public_key = $1 AND is_active = TRUE",
        )
        .bind(public_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_site).transpose()
    }

    async fn get_site_by_domain(&self, domain: &str) -> Result<Option<Site>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active \
             FROM sites WHERE domain = $1 AND is_active = TRUE LIMIT 1",
        )
        .bind(domain)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_site).transpose()
    }

    async fn list_sites(&self, org_id: Ulid) -> Result<Vec<Site>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, org_id, domain, name, timezone, public_key, created_at, is_active \
             FROM sites WHERE org_id = $1 ORDER BY created_at ASC",
        )
        .bind(org_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_site).collect()
    }

    async fn delete_site(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sites WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn create_org(&self, org: &Organization) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO orgs (id, name, slug, plan, created_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(org.id.to_string())
        .bind(&org.name)
        .bind(&org.slug)
        .bind(plan_str(org.plan))
        .bind(org.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_org(&self, id: Ulid) -> Result<Option<Organization>, StoreError> {
        let row = sqlx::query("SELECT id, name, slug, plan, created_at FROM orgs WHERE id = $1")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::db)?;
        row.map(row_to_org).transpose()
    }

    async fn list_orgs(&self) -> Result<Vec<Organization>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, name, slug, plan, created_at FROM orgs ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_org).collect()
    }

    async fn create_user(&self, user: &User) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO users (id, org_id, email, password_hash, role, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(user.id.to_string())
        .bind(user.org_id.to_string())
        .bind(&user.email)
        .bind(&user.password_hash)
        .bind(role_str(user.role))
        .bind(user.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, email, password_hash, role, created_at FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_user).transpose()
    }

    async fn get_user(&self, id: Ulid) -> Result<Option<User>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, email, password_hash, role, created_at FROM users WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_user).transpose()
    }

    async fn create_funnel(&self, funnel: &Funnel) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO funnels (id, site_id, name, definition, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(funnel.id.to_string())
        .bind(funnel.site_id.to_string())
        .bind(&funnel.name)
        .bind(&funnel.definition)
        .bind(funnel.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_funnel(&self, id: Ulid) -> Result<Option<Funnel>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, name, definition, created_at FROM funnels WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_funnel).transpose()
    }

    async fn list_funnels(&self, site_id: Ulid) -> Result<Vec<Funnel>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, name, definition, created_at FROM funnels \
             WHERE site_id = $1 ORDER BY created_at ASC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_funnel).collect()
    }

    async fn delete_funnel(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM funnels WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    // ---- Goals ----
    async fn create_goal(&self, goal: &Goal) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO goals (id, site_id, name, event_name, filters, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(goal.id.to_string())
        .bind(goal.site_id.to_string())
        .bind(&goal.name)
        .bind(&goal.event_name)
        .bind(&goal.filters)
        .bind(goal.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_goal(&self, id: Ulid) -> Result<Option<Goal>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, name, event_name, filters, created_at FROM goals WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_goal).transpose()
    }

    async fn list_goals(&self, site_id: Ulid) -> Result<Vec<Goal>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, name, event_name, filters, created_at FROM goals \
             WHERE site_id = $1 ORDER BY created_at ASC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_goal).collect()
    }

    async fn delete_goal(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM goals WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    // ---- Annotations ----
    async fn create_annotation(&self, annotation: &Annotation) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO annotations (id, site_id, date, text, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(annotation.id.to_string())
        .bind(annotation.site_id.to_string())
        .bind(annotation.date.format("%Y-%m-%d").to_string())
        .bind(&annotation.text)
        .bind(annotation.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_annotation(&self, id: Ulid) -> Result<Option<Annotation>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, date, text, created_at FROM annotations WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_annotation).transpose()
    }

    async fn list_annotations(
        &self,
        site_id: Ulid,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Result<Vec<Annotation>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, date, text, created_at FROM annotations \
             WHERE site_id = $1 AND date >= $2 AND date <= $3 ORDER BY date DESC",
        )
        .bind(site_id.to_string())
        .bind(start.format("%Y-%m-%d").to_string())
        .bind(end.format("%Y-%m-%d").to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_annotation).collect()
    }

    async fn delete_annotation(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM annotations WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    // ---- Analytics alerts ----
    async fn create_analytics_alert(&self, alert: &AnalyticsAlert) -> Result<(), StoreError> {
        let config = serde_json::to_string(&alert.config)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO analytics_alerts (id, site_id, type, config, channel_id, enabled, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(alert.id.to_string())
        .bind(alert.site_id.to_string())
        .bind(alert.kind.as_str())
        .bind(config)
        .bind(alert.channel_id.to_string())
        .bind(alert.enabled)
        .bind(alert.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_analytics_alert(&self, id: Ulid) -> Result<Option<AnalyticsAlert>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, type, config, channel_id, enabled, created_at \
             FROM analytics_alerts WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_analytics_alert).transpose()
    }

    async fn list_analytics_alerts(
        &self,
        site_id: Ulid,
    ) -> Result<Vec<AnalyticsAlert>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, type, config, channel_id, enabled, created_at \
             FROM analytics_alerts WHERE site_id = $1 ORDER BY created_at DESC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_analytics_alert).collect()
    }

    async fn list_enabled_analytics_alerts(&self) -> Result<Vec<AnalyticsAlert>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, type, config, channel_id, enabled, created_at \
             FROM analytics_alerts WHERE enabled = TRUE ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_analytics_alert).collect()
    }

    async fn set_analytics_alert_enabled(&self, id: Ulid, enabled: bool) -> Result<(), StoreError> {
        sqlx::query("UPDATE analytics_alerts SET enabled = $1 WHERE id = $2")
            .bind(enabled)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn delete_analytics_alert(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM analytics_alerts WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn record_analytics_alert_fire(
        &self,
        fire: &AnalyticsAlertFire,
    ) -> Result<(), StoreError> {
        let payload = serde_json::to_string(&fire.payload)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO analytics_alert_fires (id, alert_id, fired_at, payload) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(fire.id.to_string())
        .bind(fire.alert_id.to_string())
        .bind(fire.fired_at)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn last_analytics_alert_fire(
        &self,
        alert_id: Ulid,
    ) -> Result<Option<AnalyticsAlertFire>, StoreError> {
        let row = sqlx::query(
            "SELECT id, alert_id, fired_at, payload FROM analytics_alert_fires \
             WHERE alert_id = $1 ORDER BY fired_at DESC LIMIT 1",
        )
        .bind(alert_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_alert_fire).transpose()
    }

    // ---- Share links ----
    async fn create_share_link(&self, link: &ShareLink) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO share_links (id, site_id, token, label, expires_at, created_by, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(link.id.to_string())
        .bind(link.site_id.to_string())
        .bind(&link.token)
        .bind(&link.label)
        .bind(link.expires_at)
        .bind(&link.created_by)
        .bind(link.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_share_links(&self, site_id: Ulid) -> Result<Vec<ShareLink>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, token, label, expires_at, created_by, created_at FROM share_links \
             WHERE site_id = $1 ORDER BY created_at DESC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_share_link).collect()
    }

    async fn get_share_link(&self, id: Ulid) -> Result<Option<ShareLink>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, token, label, expires_at, created_by, created_at \
             FROM share_links WHERE id = $1",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_share_link).transpose()
    }

    async fn get_share_link_by_token(&self, token: &str) -> Result<Option<ShareLink>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, token, label, expires_at, created_by, created_at \
             FROM share_links WHERE token = $1",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_share_link).transpose()
    }

    async fn update_share_link(
        &self,
        id: Ulid,
        label: Option<String>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE share_links SET label = $2, expires_at = $3 WHERE id = $1")
            .bind(id.to_string())
            .bind(label)
            .bind(expires_at)
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn delete_share_link(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM share_links WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    // ---- Email digest subscriptions ----
    async fn upsert_digest_subscription(&self, sub: &DigestSubscription) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO digest_subscriptions \
                (id, user_id, site_id, frequency, enabled, bounce_count, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (user_id, site_id) DO UPDATE SET \
                frequency = EXCLUDED.frequency, \
                enabled = EXCLUDED.enabled, \
                bounce_count = EXCLUDED.bounce_count",
        )
        .bind(sub.id.to_string())
        .bind(sub.user_id.to_string())
        .bind(sub.site_id.to_string())
        .bind(sub.frequency.as_str())
        .bind(sub.enabled)
        .bind(sub.bounce_count as i64)
        .bind(sub.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_digest_subscription(
        &self,
        user_id: Ulid,
        site_id: Ulid,
    ) -> Result<Option<DigestSubscription>, StoreError> {
        let row = sqlx::query(
            "SELECT id, user_id, site_id, frequency, enabled, bounce_count, created_at \
             FROM digest_subscriptions WHERE user_id = $1 AND site_id = $2",
        )
        .bind(user_id.to_string())
        .bind(site_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_digest_sub).transpose()
    }

    async fn delete_digest_subscription(
        &self,
        user_id: Ulid,
        site_id: Ulid,
    ) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM digest_subscriptions WHERE user_id = $1 AND site_id = $2")
            .bind(user_id.to_string())
            .bind(site_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_enabled_digest_subscriptions(
        &self,
    ) -> Result<Vec<DigestSubscription>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, user_id, site_id, frequency, enabled, bounce_count, created_at \
             FROM digest_subscriptions WHERE enabled = TRUE ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_digest_sub).collect()
    }

    async fn record_digest_bounce(&self, id: Ulid, disable_at: u32) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE digest_subscriptions \
             SET bounce_count = bounce_count + 1, \
                 enabled = CASE WHEN bounce_count + 1 >= $2 THEN FALSE ELSE enabled END \
             WHERE id = $1",
        )
        .bind(id.to_string())
        .bind(disable_at as i64)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn upsert_agent(&self, agent: &Agent) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO agents (id, site_id, agent_id, name, policy_id, created_at, last_seen_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (site_id, agent_id) DO UPDATE SET \
                name = EXCLUDED.name, \
                policy_id = EXCLUDED.policy_id, \
                last_seen_at = EXCLUDED.last_seen_at",
        )
        .bind(agent.id.to_string())
        .bind(agent.site_id.to_string())
        .bind(&agent.agent_id)
        .bind(&agent.name)
        .bind(agent.policy_id.map(|p| p.to_string()))
        .bind(agent.created_at)
        .bind(agent.last_seen_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_agents(&self, site_id: Ulid) -> Result<Vec<Agent>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, agent_id, name, policy_id, created_at, last_seen_at \
             FROM agents WHERE site_id = $1 ORDER BY last_seen_at DESC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_agent).collect()
    }

    async fn get_agent(&self, site_id: Ulid, agent_id: &str) -> Result<Option<Agent>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, agent_id, name, policy_id, created_at, last_seen_at \
             FROM agents WHERE site_id = $1 AND agent_id = $2",
        )
        .bind(site_id.to_string())
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_agent).transpose()
    }

    async fn create_sentinel_token(&self, token: &SentinelToken) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sentinel_tokens (id, site_id, name, token_hash, created_at, last_used_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(token.id.to_string())
        .bind(token.site_id.to_string())
        .bind(&token.name)
        .bind(&token.token_hash)
        .bind(token.created_at)
        .bind(token.last_used_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_sentinel_tokens(&self, site_id: Ulid) -> Result<Vec<SentinelToken>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, name, token_hash, created_at, last_used_at \
             FROM sentinel_tokens WHERE site_id = $1 ORDER BY created_at DESC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_sentinel_token).collect()
    }

    async fn get_sentinel_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<SentinelToken>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, name, token_hash, created_at, last_used_at \
             FROM sentinel_tokens WHERE token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_sentinel_token).transpose()
    }

    async fn touch_sentinel_token(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("UPDATE sentinel_tokens SET last_used_at = NOW() WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn delete_sentinel_token(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sentinel_tokens WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    // ---- API keys ----
    async fn create_api_key(&self, key: &ApiKey) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO api_keys \
                (id, org_id, site_id, name, scope, key_hash, display_prefix, created_at, last_used_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(key.id.to_string())
        .bind(key.org_id.to_string())
        .bind(key.site_id.map(|s| s.to_string()))
        .bind(&key.name)
        .bind(key.scope.as_str())
        .bind(&key.key_hash)
        .bind(&key.display_prefix)
        .bind(key.created_at)
        .bind(key.last_used_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_api_keys(&self, org_id: Ulid) -> Result<Vec<ApiKey>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, org_id, site_id, name, scope, key_hash, display_prefix, created_at, last_used_at \
             FROM api_keys WHERE org_id = $1 ORDER BY created_at DESC",
        )
        .bind(org_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_api_key).collect()
    }

    async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKey>, StoreError> {
        let row = sqlx::query(
            "SELECT id, org_id, site_id, name, scope, key_hash, display_prefix, created_at, last_used_at \
             FROM api_keys WHERE key_hash = $1",
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_api_key).transpose()
    }

    async fn touch_api_key(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn delete_api_key(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM api_keys WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn create_alert_channel(&self, channel: &AlertChannel) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO alert_channels (id, site_id, kind, url, secret, created_at, last_error_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(channel.id.to_string())
        .bind(channel.site_id.to_string())
        .bind(channel.kind.as_str())
        .bind(&channel.url)
        .bind(&channel.secret)
        .bind(channel.created_at)
        .bind(channel.last_error_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_alert_channels(&self, site_id: Ulid) -> Result<Vec<AlertChannel>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, kind, url, secret, created_at, last_error_at \
             FROM alert_channels WHERE site_id = $1 ORDER BY created_at DESC",
        )
        .bind(site_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_alert_channel).collect()
    }

    async fn delete_alert_channel(&self, id: Ulid) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM alert_channels WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(())
    }

    async fn upsert_policy(&self, policy: &Policy) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO policies (id, site_id, repetition_max, velocity_max_tps, cost_cap_usd, hint_template, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (site_id) DO UPDATE SET \
                repetition_max = EXCLUDED.repetition_max, \
                velocity_max_tps = EXCLUDED.velocity_max_tps, \
                cost_cap_usd = EXCLUDED.cost_cap_usd, \
                hint_template = EXCLUDED.hint_template",
        )
        .bind(policy.id.to_string())
        .bind(policy.site_id.to_string())
        .bind(policy.repetition_max.map(|n| n as i64))
        .bind(policy.velocity_max_tps)
        .bind(policy.cost_cap_usd)
        .bind(&policy.hint_template)
        .bind(policy.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_policy(&self, site_id: Ulid) -> Result<Option<Policy>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, repetition_max, velocity_max_tps, cost_cap_usd, hint_template, created_at \
             FROM policies WHERE site_id = $1",
        )
        .bind(site_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(StoreError::db)?;
        row.map(row_to_policy).transpose()
    }

    async fn record_incident(&self, incident: &Incident) -> Result<(), StoreError> {
        let trigger_json = serde_json::to_string(&incident.trigger)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO incidents (id, site_id, agent_id, trigger_json, status, opened_at, closed_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(incident.id.to_string())
        .bind(incident.site_id.to_string())
        .bind(&incident.agent_id)
        .bind(trigger_json)
        .bind(incident.status.as_str())
        .bind(incident.opened_at)
        .bind(incident.closed_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn list_incidents(&self, site_id: Ulid, limit: u32) -> Result<Vec<Incident>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, site_id, agent_id, trigger_json, status, opened_at, closed_at \
             FROM incidents WHERE site_id = $1 ORDER BY opened_at DESC LIMIT $2",
        )
        .bind(site_id.to_string())
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::db)?;
        rows.into_iter().map(row_to_incident).collect()
    }

    async fn update_incident_status(
        &self,
        id: Ulid,
        status: IncidentStatus,
    ) -> Result<(), StoreError> {
        let closed_at = matches!(status, IncidentStatus::Resolved).then(Utc::now);
        sqlx::query(
            "UPDATE incidents SET status = $1, closed_at = COALESCE($2, closed_at) WHERE id = $3",
        )
        .bind(status.as_str())
        .bind(closed_at)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }
}

#[async_trait]
impl AgentStore for PostgresBackend {
    async fn ingest_spans(&self, spans: Vec<AgentSpan>) -> Result<(), StoreError> {
        if spans.is_empty() {
            return Ok(());
        }
        // Same chunking + atomicity rationale as `ingest_events`: all
        // chunks run inside one transaction so a partial failure rolls
        // the whole batch back rather than half-committing.
        const COLUMNS: usize = 18;
        const MAX_PARAMS: usize = 32_767;
        let chunk_size = MAX_PARAMS / COLUMNS;

        let mut tx = self.pool.begin().await.map_err(StoreError::db)?;
        for chunk in spans.chunks(chunk_size) {
            let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
                "INSERT INTO agent_spans (\
                    id, site_id, agent_id, agent_session_id, parent_span_id, \
                    kind, model, started_at, ended_at, \
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, \
                    cost_usd, tool_name, tool_input_hash, stop_reason, properties) ",
            );
            qb.push_values(chunk, |mut b, s| {
                b.push_bind(s.id.to_string())
                    .push_bind(s.site_id.to_string())
                    .push_bind(&s.agent_id)
                    .push_bind(&s.agent_session_id)
                    .push_bind(s.parent_span_id.map(|p| p.to_string()))
                    .push_bind(s.kind.as_str())
                    .push_bind(&s.model)
                    .push_bind(s.started_at)
                    .push_bind(s.ended_at)
                    .push_bind(s.input_tokens as i64)
                    .push_bind(s.output_tokens as i64)
                    .push_bind(s.cache_read_tokens as i64)
                    .push_bind(s.cache_creation_tokens as i64)
                    .push_bind(s.cost_usd)
                    .push_bind(&s.tool_name)
                    .push_bind(&s.tool_input_hash)
                    .push_bind(&s.stop_reason)
                    .push_bind(s.properties.as_ref().map(|p| p.to_string()));
            });
            qb.build().execute(&mut *tx).await.map_err(StoreError::db)?;
        }
        tx.commit().await.map_err(StoreError::db)?;
        Ok(())
    }

    async fn query_spans(&self, q: &SpanQuery) -> Result<Vec<SpanRow>, StoreError> {
        // Build with a fixed parameter shape so the prepared-statement
        // cache hits across calls regardless of which optional filters
        // are present. NULL acts as a wildcard.
        let rows = sqlx::query(
            "SELECT id, agent_id, agent_session_id, kind, model, started_at, ended_at, \
                    input_tokens, output_tokens, cost_usd, tool_name, stop_reason \
             FROM agent_spans \
             WHERE site_id = $1 \
               AND started_at >= $2 AND started_at <= $3 \
               AND ($4::TEXT IS NULL OR agent_id = $4) \
               AND ($5::TEXT IS NULL OR agent_session_id = $5) \
             ORDER BY started_at DESC \
             LIMIT $6",
        )
        .bind(q.site_id.to_string())
        .bind(q.since)
        .bind(q.until)
        .bind(&q.agent_id)
        .bind(&q.session_id)
        .bind(q.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::query)?;

        rows.into_iter()
            .map(|row| {
                let in_tok: i64 = row.try_get("input_tokens").map_err(StoreError::query)?;
                let out_tok: i64 = row.try_get("output_tokens").map_err(StoreError::query)?;
                Ok(SpanRow {
                    id: row.try_get("id").map_err(StoreError::query)?,
                    agent_id: row.try_get("agent_id").map_err(StoreError::query)?,
                    agent_session_id: row.try_get("agent_session_id").map_err(StoreError::query)?,
                    kind: row.try_get("kind").map_err(StoreError::query)?,
                    model: row.try_get("model").map_err(StoreError::query)?,
                    started_at: row.try_get("started_at").map_err(StoreError::query)?,
                    ended_at: row.try_get("ended_at").map_err(StoreError::query)?,
                    input_tokens: in_tok as u32,
                    output_tokens: out_tok as u32,
                    cost_usd: row.try_get("cost_usd").map_err(StoreError::query)?,
                    tool_name: row.try_get("tool_name").map_err(StoreError::query)?,
                    stop_reason: row.try_get("stop_reason").map_err(StoreError::query)?,
                })
            })
            .collect()
    }

    async fn summarize_agents(
        &self,
        site_id: Ulid,
        since: DateTime<Utc>,
    ) -> Result<Vec<AgentSummary>, StoreError> {
        let rows = sqlx::query(
            "SELECT agent_id, \
                    MAX(started_at) AS last_seen, \
                    COUNT(*)::BIGINT AS total_spans, \
                    SUM(input_tokens)::BIGINT AS in_tok, \
                    SUM(output_tokens)::BIGINT AS out_tok, \
                    SUM(cost_usd)::DOUBLE PRECISION AS cost \
             FROM agent_spans \
             WHERE site_id = $1 AND started_at >= $2 \
             GROUP BY agent_id \
             ORDER BY last_seen DESC",
        )
        .bind(site_id.to_string())
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::query)?;

        rows.into_iter()
            .map(|row| {
                let total_spans: i64 = row.try_get("total_spans").map_err(StoreError::query)?;
                let in_tok: i64 = row.try_get("in_tok").map_err(StoreError::query)?;
                let out_tok: i64 = row.try_get("out_tok").map_err(StoreError::query)?;
                Ok(AgentSummary {
                    agent_id: row.try_get("agent_id").map_err(StoreError::query)?,
                    last_seen_at: row.try_get("last_seen").map_err(StoreError::query)?,
                    total_spans: total_spans as u64,
                    total_input_tokens: in_tok as u64,
                    total_output_tokens: out_tok as u64,
                    total_cost_usd: row.try_get("cost").map_err(StoreError::query)?,
                })
            })
            .collect()
    }

    async fn session_cost_usd(
        &self,
        site_id: Ulid,
        agent_session_id: &str,
    ) -> Result<f64, StoreError> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(cost_usd), 0.0)::DOUBLE PRECISION AS cost \
             FROM agent_spans \
             WHERE site_id = $1 AND agent_session_id = $2",
        )
        .bind(site_id.to_string())
        .bind(agent_session_id)
        .fetch_one(&self.pool)
        .await
        .map_err(StoreError::query)?;
        row.try_get("cost").map_err(StoreError::query)
    }
}

// ---------------------------------------------------------------------------
// Row decoders.
// ---------------------------------------------------------------------------

fn row_to_site(row: sqlx::postgres::PgRow) -> Result<Site, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let org_id: String = row.try_get("org_id").map_err(StoreError::db)?;
    Ok(Site {
        id: parse_ulid(&id),
        org_id: parse_ulid(&org_id),
        domain: row.try_get("domain").map_err(StoreError::db)?,
        name: row.try_get("name").map_err(StoreError::db)?,
        timezone: row.try_get("timezone").map_err(StoreError::db)?,
        public_key: row.try_get("public_key").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
        is_active: row.try_get("is_active").map_err(StoreError::db)?,
    })
}

fn row_to_org(row: sqlx::postgres::PgRow) -> Result<Organization, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let plan: String = row.try_get("plan").map_err(StoreError::db)?;
    Ok(Organization {
        id: parse_ulid(&id),
        name: row.try_get("name").map_err(StoreError::db)?,
        slug: row.try_get("slug").map_err(StoreError::db)?,
        plan: parse_plan(&plan),
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_user(row: sqlx::postgres::PgRow) -> Result<User, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let org_id: String = row.try_get("org_id").map_err(StoreError::db)?;
    let role: String = row.try_get("role").map_err(StoreError::db)?;
    Ok(User {
        id: parse_ulid(&id),
        org_id: parse_ulid(&org_id),
        email: row.try_get("email").map_err(StoreError::db)?,
        password_hash: row.try_get("password_hash").map_err(StoreError::db)?,
        role: parse_role(&role),
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_funnel(row: sqlx::postgres::PgRow) -> Result<Funnel, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    Ok(Funnel {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        name: row.try_get("name").map_err(StoreError::db)?,
        definition: row.try_get("definition").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_goal(row: sqlx::postgres::PgRow) -> Result<Goal, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    Ok(Goal {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        name: row.try_get("name").map_err(StoreError::db)?,
        event_name: row.try_get("event_name").map_err(StoreError::db)?,
        filters: row.try_get("filters").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_annotation(row: sqlx::postgres::PgRow) -> Result<Annotation, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let date: String = row.try_get("date").map_err(StoreError::db)?;
    Ok(Annotation {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        date: chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").unwrap_or_default(),
        text: row.try_get("text").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_analytics_alert(row: sqlx::postgres::PgRow) -> Result<AnalyticsAlert, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let type_str: String = row.try_get("type").map_err(StoreError::db)?;
    let config_str: String = row.try_get("config").map_err(StoreError::db)?;
    let channel_id: String = row.try_get("channel_id").map_err(StoreError::db)?;
    Ok(AnalyticsAlert {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        kind: AnalyticsAlertKind::from_str(&type_str).unwrap_or(AnalyticsAlertKind::TrafficSpike),
        config: serde_json::from_str(&config_str).unwrap_or_default(),
        channel_id: parse_ulid(&channel_id),
        enabled: row.try_get("enabled").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_alert_fire(row: sqlx::postgres::PgRow) -> Result<AnalyticsAlertFire, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let alert_id: String = row.try_get("alert_id").map_err(StoreError::db)?;
    let payload_str: String = row.try_get("payload").map_err(StoreError::db)?;
    Ok(AnalyticsAlertFire {
        id: parse_ulid(&id),
        alert_id: parse_ulid(&alert_id),
        fired_at: row.try_get("fired_at").map_err(StoreError::db)?,
        payload: serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null),
    })
}

fn row_to_share_link(row: sqlx::postgres::PgRow) -> Result<ShareLink, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    Ok(ShareLink {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        token: row.try_get("token").map_err(StoreError::db)?,
        label: row.try_get("label").map_err(StoreError::db)?,
        expires_at: row.try_get("expires_at").map_err(StoreError::db)?,
        created_by: row.try_get("created_by").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_digest_sub(row: sqlx::postgres::PgRow) -> Result<DigestSubscription, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let user_id: String = row.try_get("user_id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let freq: String = row.try_get("frequency").map_err(StoreError::db)?;
    let bounce_count: i64 = row.try_get("bounce_count").map_err(StoreError::db)?;
    Ok(DigestSubscription {
        id: parse_ulid(&id),
        user_id: parse_ulid(&user_id),
        site_id: parse_ulid(&site_id),
        frequency: DigestFrequency::from_str(&freq).unwrap_or(DigestFrequency::Weekly),
        enabled: row.try_get("enabled").map_err(StoreError::db)?,
        bounce_count: bounce_count.max(0) as u32,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_agent(row: sqlx::postgres::PgRow) -> Result<Agent, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let policy_id: Option<String> = row.try_get("policy_id").map_err(StoreError::db)?;
    Ok(Agent {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        agent_id: row.try_get("agent_id").map_err(StoreError::db)?,
        name: row.try_get("name").map_err(StoreError::db)?,
        policy_id: policy_id.as_deref().and_then(|s| Ulid::from_string(s).ok()),
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
        last_seen_at: row.try_get("last_seen_at").map_err(StoreError::db)?,
    })
}

fn row_to_sentinel_token(row: sqlx::postgres::PgRow) -> Result<SentinelToken, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    Ok(SentinelToken {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        name: row.try_get("name").map_err(StoreError::db)?,
        token_hash: row.try_get("token_hash").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
        last_used_at: row.try_get("last_used_at").map_err(StoreError::db)?,
    })
}

fn row_to_api_key(row: sqlx::postgres::PgRow) -> Result<ApiKey, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let org_id: String = row.try_get("org_id").map_err(StoreError::db)?;
    let site_id: Option<String> = row.try_get("site_id").map_err(StoreError::db)?;
    let scope: String = row.try_get("scope").map_err(StoreError::db)?;
    Ok(ApiKey {
        id: parse_ulid(&id),
        org_id: parse_ulid(&org_id),
        site_id: site_id.as_deref().and_then(|s| Ulid::from_string(s).ok()),
        name: row.try_get("name").map_err(StoreError::db)?,
        scope: ApiKeyScope::parse(&scope).unwrap_or(ApiKeyScope::Read),
        key_hash: row.try_get("key_hash").map_err(StoreError::db)?,
        display_prefix: row.try_get("display_prefix").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
        last_used_at: row.try_get("last_used_at").map_err(StoreError::db)?,
    })
}

fn row_to_alert_channel(row: sqlx::postgres::PgRow) -> Result<AlertChannel, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let kind: String = row.try_get("kind").map_err(StoreError::db)?;
    Ok(AlertChannel {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        kind: parse_alert_kind(&kind),
        url: row.try_get("url").map_err(StoreError::db)?,
        secret: row.try_get("secret").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
        last_error_at: row.try_get("last_error_at").map_err(StoreError::db)?,
    })
}

fn row_to_policy(row: sqlx::postgres::PgRow) -> Result<Policy, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let repetition_max: Option<i64> = row.try_get("repetition_max").map_err(StoreError::db)?;
    Ok(Policy {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        repetition_max: repetition_max.map(|n| n as u32),
        velocity_max_tps: row.try_get("velocity_max_tps").map_err(StoreError::db)?,
        cost_cap_usd: row.try_get("cost_cap_usd").map_err(StoreError::db)?,
        hint_template: row.try_get("hint_template").map_err(StoreError::db)?,
        created_at: row.try_get("created_at").map_err(StoreError::db)?,
    })
}

fn row_to_incident(row: sqlx::postgres::PgRow) -> Result<Incident, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let trigger_json: String = row.try_get("trigger_json").map_err(StoreError::db)?;
    let status: String = row.try_get("status").map_err(StoreError::db)?;
    let trigger: IncidentTrigger =
        serde_json::from_str(&trigger_json).unwrap_or(IncidentTrigger::Manual);
    Ok(Incident {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        agent_id: row.try_get("agent_id").map_err(StoreError::db)?,
        trigger,
        status: parse_incident_status(&status),
        opened_at: row.try_get("opened_at").map_err(StoreError::db)?,
        closed_at: row.try_get("closed_at").map_err(StoreError::db)?,
    })
}

fn plan_str(p: Plan) -> &'static str {
    match p {
        Plan::SelfHosted => "self_hosted",
        Plan::Free => "free",
        Plan::Pro => "pro",
        Plan::Enterprise => "enterprise",
    }
}

fn parse_plan(s: &str) -> Plan {
    match s {
        "free" => Plan::Free,
        "pro" => Plan::Pro,
        "enterprise" => Plan::Enterprise,
        _ => Plan::SelfHosted,
    }
}

fn role_str(r: UserRole) -> &'static str {
    match r {
        UserRole::Owner => "owner",
        UserRole::Admin => "admin",
        UserRole::Viewer => "viewer",
    }
}

fn parse_role(s: &str) -> UserRole {
    match s {
        "admin" => UserRole::Admin,
        "viewer" => UserRole::Viewer,
        _ => UserRole::Owner,
    }
}

fn parse_alert_kind(s: &str) -> AlertChannelKind {
    AlertChannelKind::from_str(s)
}

fn parse_incident_status(s: &str) -> IncidentStatus {
    match s {
        "acknowledged" => IncidentStatus::Acknowledged,
        "resolved" => IncidentStatus::Resolved,
        _ => IncidentStatus::Open,
    }
}

mod ddl {
    pub fn all_statements() -> [&'static str; 16] {
        [
            ORGS_DDL,
            SITES_DDL,
            USERS_DDL,
            FUNNELS_DDL,
            POLICIES_DDL,
            AGENTS_DDL,
            SENTINEL_TOKENS_DDL,
            API_KEYS_DDL,
            ALERT_CHANNELS_DDL,
            INCIDENTS_DDL,
            GOALS_DDL,
            ANNOTATIONS_DDL,
            ANALYTICS_ALERTS_DDL,
            ANALYTICS_ALERT_FIRES_DDL,
            SHARE_LINKS_DDL,
            DIGEST_SUBSCRIPTIONS_DDL,
        ]
    }

    pub fn analytics_statements() -> [&'static str; 4] {
        [EVENTS_DDL, EVENTS_INDEX_TIMESTAMP, SPANS_DDL, SPANS_INDEX]
    }

    const ORGS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS orgs (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            slug       TEXT UNIQUE NOT NULL,
            plan       TEXT NOT NULL DEFAULT 'self_hosted',
            created_at TIMESTAMPTZ NOT NULL
        )
    "#;

    const SITES_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS sites (
            id         TEXT PRIMARY KEY,
            org_id     TEXT NOT NULL REFERENCES orgs(id),
            domain     TEXT NOT NULL,
            name       TEXT NOT NULL,
            timezone   TEXT NOT NULL DEFAULT 'UTC',
            public_key TEXT UNIQUE NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            is_active  BOOLEAN NOT NULL DEFAULT TRUE
        )
    "#;

    const USERS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS users (
            id            TEXT PRIMARY KEY,
            org_id        TEXT NOT NULL REFERENCES orgs(id),
            email         TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            role          TEXT NOT NULL DEFAULT 'owner',
            created_at    TIMESTAMPTZ NOT NULL
        )
    "#;

    const FUNNELS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS funnels (
            id         TEXT PRIMARY KEY,
            site_id    TEXT NOT NULL REFERENCES sites(id),
            name       TEXT NOT NULL,
            definition TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL
        )
    "#;

    const POLICIES_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS policies (
            id                TEXT PRIMARY KEY,
            site_id           TEXT NOT NULL REFERENCES sites(id) UNIQUE,
            repetition_max    BIGINT,
            velocity_max_tps  DOUBLE PRECISION,
            cost_cap_usd      DOUBLE PRECISION,
            hint_template     TEXT,
            created_at        TIMESTAMPTZ NOT NULL
        )
    "#;

    const AGENTS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS agents (
            id            TEXT PRIMARY KEY,
            site_id       TEXT NOT NULL REFERENCES sites(id),
            agent_id      TEXT NOT NULL,
            name          TEXT NOT NULL,
            policy_id     TEXT REFERENCES policies(id),
            created_at    TIMESTAMPTZ NOT NULL,
            last_seen_at  TIMESTAMPTZ NOT NULL,
            UNIQUE (site_id, agent_id)
        )
    "#;

    const SENTINEL_TOKENS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS sentinel_tokens (
            id            TEXT PRIMARY KEY,
            site_id       TEXT NOT NULL REFERENCES sites(id),
            name          TEXT NOT NULL,
            token_hash    TEXT UNIQUE NOT NULL,
            created_at    TIMESTAMPTZ NOT NULL,
            last_used_at  TIMESTAMPTZ
        )
    "#;

    const API_KEYS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS api_keys (
            id             TEXT PRIMARY KEY,
            org_id         TEXT NOT NULL REFERENCES orgs(id),
            site_id        TEXT REFERENCES sites(id),
            name           TEXT NOT NULL,
            scope          TEXT NOT NULL,
            key_hash       TEXT UNIQUE NOT NULL,
            display_prefix TEXT NOT NULL,
            created_at     TIMESTAMPTZ NOT NULL,
            last_used_at   TIMESTAMPTZ
        )
    "#;

    const ALERT_CHANNELS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS alert_channels (
            id              TEXT PRIMARY KEY,
            site_id         TEXT NOT NULL REFERENCES sites(id),
            kind            TEXT NOT NULL,
            url             TEXT NOT NULL,
            secret          TEXT,
            created_at      TIMESTAMPTZ NOT NULL,
            last_error_at   TIMESTAMPTZ
        )
    "#;

    const INCIDENTS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS incidents (
            id            TEXT PRIMARY KEY,
            site_id       TEXT NOT NULL REFERENCES sites(id),
            agent_id      TEXT NOT NULL,
            trigger_json  TEXT NOT NULL,
            status        TEXT NOT NULL DEFAULT 'open',
            opened_at     TIMESTAMPTZ NOT NULL,
            closed_at     TIMESTAMPTZ
        )
    "#;

    const GOALS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS goals (
            id          TEXT PRIMARY KEY,
            site_id     TEXT NOT NULL REFERENCES sites(id),
            name        TEXT NOT NULL,
            event_name  TEXT NOT NULL,
            filters     TEXT,
            created_at  TIMESTAMPTZ NOT NULL
        )
    "#;

    const ANNOTATIONS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS annotations (
            id          TEXT PRIMARY KEY,
            site_id     TEXT NOT NULL REFERENCES sites(id),
            date        TEXT NOT NULL,
            text        TEXT NOT NULL,
            created_at  TIMESTAMPTZ NOT NULL
        )
    "#;

    const ANALYTICS_ALERTS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS analytics_alerts (
            id          TEXT PRIMARY KEY,
            site_id     TEXT NOT NULL REFERENCES sites(id),
            type        TEXT NOT NULL,
            config      TEXT NOT NULL,
            channel_id  TEXT NOT NULL REFERENCES alert_channels(id),
            enabled     BOOLEAN NOT NULL DEFAULT TRUE,
            created_at  TIMESTAMPTZ NOT NULL
        )
    "#;

    const ANALYTICS_ALERT_FIRES_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS analytics_alert_fires (
            id          TEXT PRIMARY KEY,
            alert_id    TEXT NOT NULL REFERENCES analytics_alerts(id),
            fired_at    TIMESTAMPTZ NOT NULL,
            payload     TEXT NOT NULL
        )
    "#;

    const SHARE_LINKS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS share_links (
            id          TEXT PRIMARY KEY,
            site_id     TEXT NOT NULL REFERENCES sites(id),
            token       TEXT UNIQUE NOT NULL,
            label       TEXT,
            expires_at  TIMESTAMPTZ,
            created_by  TEXT NOT NULL,
            created_at  TIMESTAMPTZ NOT NULL
        )
    "#;

    const DIGEST_SUBSCRIPTIONS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS digest_subscriptions (
            id           TEXT PRIMARY KEY,
            user_id      TEXT NOT NULL REFERENCES users(id),
            site_id      TEXT NOT NULL REFERENCES sites(id),
            frequency    TEXT NOT NULL,
            enabled      BOOLEAN NOT NULL DEFAULT TRUE,
            bounce_count BIGINT NOT NULL DEFAULT 0,
            created_at   TIMESTAMPTZ NOT NULL,
            UNIQUE (user_id, site_id)
        )
    "#;

    const EVENTS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS events (
            id              TEXT PRIMARY KEY,
            site_id         TEXT NOT NULL,
            name            TEXT NOT NULL,
            kind            TEXT NOT NULL,
            timestamp       TIMESTAMPTZ NOT NULL,
            received_at     TIMESTAMPTZ NOT NULL,
            url             TEXT NOT NULL,
            referrer        TEXT,
            utm_source      TEXT,
            utm_medium      TEXT,
            utm_campaign    TEXT,
            utm_term        TEXT,
            utm_content     TEXT,
            browser         TEXT NOT NULL,
            browser_version TEXT NOT NULL,
            os              TEXT NOT NULL,
            os_version      TEXT NOT NULL,
            device_type     TEXT NOT NULL,
            screen_width    INTEGER,
            screen_height   INTEGER,
            language        TEXT,
            ip_anonymized   TEXT NOT NULL,
            country_code    TEXT,
            region          TEXT,
            city            TEXT,
            session_id      BYTEA NOT NULL,
            properties      TEXT
        )
    "#;

    const EVENTS_INDEX_TIMESTAMP: &str =
        "CREATE INDEX IF NOT EXISTS idx_events_site_timestamp ON events(site_id, timestamp DESC)";

    const SPANS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS agent_spans (
            id                    TEXT PRIMARY KEY,
            site_id               TEXT NOT NULL,
            agent_id              TEXT NOT NULL,
            agent_session_id      TEXT NOT NULL,
            parent_span_id        TEXT,
            kind                  TEXT NOT NULL,
            model                 TEXT NOT NULL,
            started_at            TIMESTAMPTZ NOT NULL,
            ended_at              TIMESTAMPTZ NOT NULL,
            input_tokens          BIGINT NOT NULL,
            output_tokens         BIGINT NOT NULL,
            cache_read_tokens     BIGINT NOT NULL,
            cache_creation_tokens BIGINT NOT NULL,
            cost_usd              DOUBLE PRECISION NOT NULL,
            tool_name             TEXT,
            tool_input_hash       TEXT,
            stop_reason           TEXT,
            properties            TEXT
        )
    "#;

    const SPANS_INDEX: &str =
        "CREATE INDEX IF NOT EXISTS idx_spans_site_started ON agent_spans(site_id, started_at DESC)";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_metadata_tables_present() {
        let stmts = ddl::all_statements().join("\n");
        for table in [
            "orgs",
            "sites",
            "users",
            "funnels",
            "policies",
            "agents",
            "sentinel_tokens",
            "api_keys",
            "alert_channels",
            "incidents",
        ] {
            assert!(
                stmts.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
                "missing table {table} in DDL"
            );
        }
    }

    #[test]
    fn ddl_analytics_tables_present_with_indexes() {
        let stmts = ddl::analytics_statements().join("\n");
        assert!(stmts.contains("CREATE TABLE IF NOT EXISTS events"));
        assert!(stmts.contains("CREATE TABLE IF NOT EXISTS agent_spans"));
        assert!(stmts.contains("idx_events_site_timestamp"));
        assert!(stmts.contains("idx_spans_site_started"));
    }

    #[test]
    fn plan_round_trip() {
        for p in [Plan::SelfHosted, Plan::Free, Plan::Pro, Plan::Enterprise] {
            assert_eq!(parse_plan(plan_str(p)), p);
        }
    }

    #[test]
    fn role_round_trip() {
        for r in [UserRole::Owner, UserRole::Admin, UserRole::Viewer] {
            assert_eq!(parse_role(role_str(r)), r);
        }
    }

    #[test]
    fn alert_kind_parses_known_values() {
        assert!(matches!(parse_alert_kind("slack"), AlertChannelKind::Slack));
        assert!(matches!(
            parse_alert_kind("webhook"),
            AlertChannelKind::Webhook
        ));
        // Unknown values default to Webhook (most permissive sink).
        assert!(matches!(
            parse_alert_kind("teams"),
            AlertChannelKind::Webhook
        ));
    }

    #[test]
    fn incident_status_parses_known_values() {
        assert!(matches!(
            parse_incident_status("open"),
            IncidentStatus::Open
        ));
        assert!(matches!(
            parse_incident_status("acknowledged"),
            IncidentStatus::Acknowledged
        ));
        assert!(matches!(
            parse_incident_status("resolved"),
            IncidentStatus::Resolved
        ));
    }

    #[test]
    fn ulid_parse_falls_back_to_default_on_garbage() {
        let bad = parse_ulid("not-a-ulid");
        assert_eq!(bad, Ulid::default());
    }
}
