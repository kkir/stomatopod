//! Postgres-backed storage for SaaS deployments.
//!
//! Holds everything in one database - analytics events and metadata
//! (sites, orgs, users, funnels). Uses
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
        alert_channel::{AlertChannel, AlertChannelKind},
        analytics_alert::{
            default_analytics_alerts, AnalyticsAlert, AnalyticsAlertFire, AnalyticsAlertKind,
        },
        api_key::{ApiKey, ApiKeyScope},
        digest::{DigestFrequency, DigestSubscription},
        event::Event,
        org::{Funnel, Organization, Plan, User, UserRole},
        site::Site,
    },
    error::StoreError,
    query::{
        analytics::{
            EntryPageRow, EntryPages, ExitPageRow, ExitPages, RawEventRow, SessionRow,
            TopSparklines,
        },
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult, FunnelStepResult},
        pageviews::{
            Filter, FilterOp, Granularity, PageviewsQuery, PageviewsResult, TimeBucket, TimeRange,
            TopList, TopListField, TopRow,
        },
    },
    traits::{MetaStore, StorageBackend},
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
                    COUNT(*)::BIGINT AS pageviews, \
                    COUNT(DISTINCT session_id)::BIGINT AS sessions \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
               AND kind = 'pageview' {filter_sql} \
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

        // Session-level bounce + avg duration over the full range. A bounce is
        // a session with exactly one pageview (same as entry-page bounce).
        if result.total_pageviews > 0 {
            let (filter_sql, filter_vals) = pg_filter_clause(&q.filters, 4);
            let summary_sql = format!(
                "WITH sessions AS (\
                    SELECT session_id, COUNT(*) AS pv_count, \
                           MIN(timestamp) AS started, MAX(timestamp) AS ended \
                    FROM events \
                    WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
                      AND kind = 'pageview' {filter_sql} \
                    GROUP BY session_id) \
                 SELECT COUNT(*)::BIGINT AS sessions, \
                        SUM(CASE WHEN pv_count = 1 THEN 1 ELSE 0 END)::BIGINT AS bounces, \
                        COALESCE(AVG(EXTRACT(EPOCH FROM (ended - started))), 0)::FLOAT8 \
                            AS avg_duration_secs \
                 FROM sessions"
            );
            let mut summary = sqlx::query(&summary_sql)
                .bind(q.site_id.to_string())
                .bind(q.range.start)
                .bind(q.range.end);
            for val in &filter_vals {
                summary = summary.bind(val.clone());
            }
            if let Some(row) = summary
                .fetch_optional(&self.pool)
                .await
                .map_err(StoreError::query)?
            {
                let sessions: i64 = row.try_get("sessions").map_err(StoreError::query)?;
                let bounces: i64 = row.try_get("bounces").map_err(StoreError::query)?;
                let avg: f64 = row
                    .try_get("avg_duration_secs")
                    .map_err(StoreError::query)?;
                let sessions = sessions.max(0) as u64;
                let bounces = bounces.max(0) as u64;
                result.bounce_rate = if sessions > 0 {
                    bounces as f64 / sessions as f64 * 100.0
                } else {
                    0.0
                };
                result.avg_duration_secs = avg.max(0.0);
            }
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
        // $1 site, $2 start, $3 end, optional $4 name, then filter values.
        let mut next_idx = 4;
        let name_filter = if q.event_name.is_some() {
            let clause = format!("AND name = ${next_idx}");
            next_idx += 1;
            clause
        } else {
            String::new()
        };
        let (filter_sql, filter_vals) = pg_filter_clause(&q.filters, next_idx);
        let sql = format!(
            "SELECT name AS value, \
                    COUNT(*)::BIGINT AS pageviews, \
                    COUNT(DISTINCT session_id)::BIGINT AS sessions \
             FROM events \
             WHERE site_id = $1 AND timestamp >= $2 AND timestamp <= $3 \
               AND kind = 'custom' {name_filter} {filter_sql} \
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
        for val in &filter_vals {
            query = query.bind(val.clone());
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

    async fn prune_events_before(&self, cutoff: chrono::DateTime<Utc>) -> Result<u64, StoreError> {
        let result = sqlx::query("DELETE FROM events WHERE timestamp < $1")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        Ok(result.rows_affected())
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
        // Real starter rows so the Alerts UI can list/disable/delete them.
        for alert in default_analytics_alerts(site.id) {
            self.create_analytics_alert(&alert).await?;
        }
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
        let id_str = id.to_string();
        // Child tables reference sites (and analytics_alert_fires refs
        // analytics_alerts). Clear them before the site row itself.
        // create_site inserts default analytics_alerts, so a bare DELETE
        // on sites always fails with a foreign key constraint.
        sqlx::query(
            "DELETE FROM analytics_alert_fires WHERE alert_id IN (
                 SELECT id FROM analytics_alerts WHERE site_id = $1
             )",
        )
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        sqlx::query("DELETE FROM analytics_alerts WHERE site_id = $1")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        sqlx::query("DELETE FROM funnels WHERE site_id = $1")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        sqlx::query("DELETE FROM alert_channels WHERE site_id = $1")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        sqlx::query("DELETE FROM digest_subscriptions WHERE site_id = $1")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        sqlx::query("DELETE FROM api_keys WHERE site_id = $1")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        sqlx::query("DELETE FROM sites WHERE id = $1")
            .bind(&id_str)
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

    async fn update_user_password(&self, id: Ulid, password_hash: &str) -> Result<(), StoreError> {
        let result = sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
            .bind(password_hash)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(StoreError::db)?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
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

    // ---- Analytics alerts ----
    async fn create_analytics_alert(&self, alert: &AnalyticsAlert) -> Result<(), StoreError> {
        let config = serde_json::to_string(&alert.config)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO analytics_alerts (id, site_id, type, config, enabled, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(alert.id.to_string())
        .bind(alert.site_id.to_string())
        .bind(alert.kind.as_str())
        .bind(config)
        .bind(alert.enabled)
        .bind(alert.created_at)
        .execute(&self.pool)
        .await
        .map_err(StoreError::db)?;
        Ok(())
    }

    async fn get_analytics_alert(&self, id: Ulid) -> Result<Option<AnalyticsAlert>, StoreError> {
        let row = sqlx::query(
            "SELECT id, site_id, type, config, enabled, created_at \
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
            "SELECT id, site_id, type, config, enabled, created_at \
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
            "SELECT id, site_id, type, config, enabled, created_at \
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

    // ---- Analytics digest subscriptions ----
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

fn row_to_analytics_alert(row: sqlx::postgres::PgRow) -> Result<AnalyticsAlert, StoreError> {
    let id: String = row.try_get("id").map_err(StoreError::db)?;
    let site_id: String = row.try_get("site_id").map_err(StoreError::db)?;
    let type_str: String = row.try_get("type").map_err(StoreError::db)?;
    let config_str: String = row.try_get("config").map_err(StoreError::db)?;
    Ok(AnalyticsAlert {
        id: parse_ulid(&id),
        site_id: parse_ulid(&site_id),
        kind: AnalyticsAlertKind::from_str(&type_str).unwrap_or(AnalyticsAlertKind::TrafficSpike),
        config: serde_json::from_str(&config_str).unwrap_or_default(),
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

fn plan_str(_p: Plan) -> &'static str {
    "self_hosted"
}

fn parse_plan(_s: &str) -> Plan {
    // Map legacy free/pro/enterprise rows to SelfHosted.
    Plan::SelfHosted
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

mod ddl {
    pub fn all_statements() -> [&'static str; 9] {
        [
            ORGS_DDL,
            SITES_DDL,
            USERS_DDL,
            FUNNELS_DDL,
            API_KEYS_DDL,
            ALERT_CHANNELS_DDL,
            ANALYTICS_ALERTS_DDL,
            ANALYTICS_ALERT_FIRES_DDL,
            DIGEST_SUBSCRIPTIONS_DDL,
        ]
    }

    pub fn analytics_statements() -> [&'static str; 2] {
        [EVENTS_DDL, EVENTS_INDEX_TIMESTAMP]
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

    const ANALYTICS_ALERTS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS analytics_alerts (
            id          TEXT PRIMARY KEY,
            site_id     TEXT NOT NULL REFERENCES sites(id),
            type        TEXT NOT NULL,
            config      TEXT NOT NULL,
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
            "api_keys",
            "alert_channels",
            "analytics_alerts",
            "digest_subscriptions",
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
        assert!(stmts.contains("idx_events_site_timestamp"));
    }

    #[test]
    fn plan_round_trip() {
        assert_eq!(parse_plan(plan_str(Plan::SelfHosted)), Plan::SelfHosted);
        // Legacy plan strings all map to SelfHosted.
        for s in ["free", "pro", "enterprise", "self_hosted", "unknown"] {
            assert_eq!(parse_plan(s), Plan::SelfHosted);
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
    fn ulid_parse_falls_back_to_default_on_garbage() {
        let bad = parse_ulid("not-a-ulid");
        assert_eq!(bad, Ulid::default());
    }
}
