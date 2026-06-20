//! ClickHouse-backed analytics storage for high-scale SaaS deployments.
//!
//! Uses the HTTP interface for portability across ClickHouse versions:
//! - ingest goes through `INSERT ... FORMAT JSONEachRow` with rows hand-
//!   serialized so column types match the schema (chrono ISO8601 strings,
//!   hex-encoded session ids, json-stringified properties).
//! - reads go through `FORMAT JSON` and are decoded with `serde_json`.
//!
//! In a SaaS deployment ClickHouse holds analytics rows only; metadata
//! (sites, orgs, users, funnels, agents, …) lives in Postgres. So this
//! backend implements `StorageBackend` + `AgentStore` but not `MetaStore`.

use async_trait::async_trait;
use serde::Deserialize;
use ulid::Ulid;

use stomatopod_core::{
    config::ClickhouseConfig,
    domain::{agent_span::AgentSpan, event::Event},
    error::StoreError,
    query::{
        analytics::{
            EntryPageRow, EntryPages, ExitPageRow, ExitPages, GoalBucket, GoalQuery, GoalStats,
            RawEventRow, RealtimeEvent, RealtimeSnapshot, RealtimeTopPage, SessionRow,
        },
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult, FunnelStepResult},
        pageviews::{
            Filter, PageviewsQuery, PageviewsResult, TimeBucket, TimeRange, TopList, TopListField,
        },
        spans::{AgentSummary, SpanQuery, SpanRow},
    },
    traits::{AgentStore, StorageBackend},
};

pub struct ClickhouseBackend {
    client: reqwest::Client,
    url: String,
    database: String,
    username: String,
    password: String,
}

impl ClickhouseBackend {
    pub fn new(cfg: &ClickhouseConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: cfg.url.clone(),
            database: cfg.database.clone(),
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        }
    }

    /// Create the `events` and `agent_spans` tables if they don't exist.
    /// Idempotent — safe to call on every boot.
    pub async fn bootstrap(&self) -> Result<(), StoreError> {
        self.execute(&format!("CREATE DATABASE IF NOT EXISTS {}", self.database))
            .await?;
        for stmt in ddl::all_statements() {
            self.execute(stmt).await?;
        }
        Ok(())
    }

    async fn execute(&self, query: &str) -> Result<String, StoreError> {
        let response = self
            .client
            .post(&self.url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("database", &self.database)])
            .body(query.to_string())
            .send()
            .await
            .map_err(StoreError::db)?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(StoreError::db(format!("ClickHouse error: {text}")));
        }

        response.text().await.map_err(StoreError::db)
    }

    async fn query_json<T: for<'de> Deserialize<'de>>(
        &self,
        sql: &str,
    ) -> Result<Vec<T>, StoreError> {
        let body = self.execute(&format!("{sql} FORMAT JSON")).await?;
        let parsed: JsonResponse<T> = serde_json::from_str(&body).map_err(StoreError::query)?;
        Ok(parsed.data)
    }
}

#[derive(Deserialize)]
struct JsonResponse<T> {
    data: Vec<T>,
}

#[async_trait]
impl StorageBackend for ClickhouseBackend {
    async fn ingest_events(&self, events: Vec<Event>) -> Result<(), StoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let rows: Vec<String> = events
            .iter()
            .map(|e| {
                serde_json::to_string(&row::EventRow::from(e))
                    .expect("EventRow serialization is infallible")
            })
            .collect();
        let body = format!("INSERT INTO events FORMAT JSONEachRow\n{}", rows.join("\n"));
        self.execute(&body).await?;
        Ok(())
    }

    async fn query_pageviews(&self, q: &PageviewsQuery) -> Result<PageviewsResult, StoreError> {
        let sql = sql::pageviews(q);
        #[derive(Deserialize)]
        struct Row {
            bucket: String,
            pageviews: String,
            sessions: String,
        }
        let rows: Vec<Row> = self.query_json(&sql).await?;
        let mut result = PageviewsResult::default();
        for r in rows {
            let pv = r.pageviews.parse::<u64>().unwrap_or(0);
            let sess = r.sessions.parse::<u64>().unwrap_or(0);
            result.total_pageviews += pv;
            result.total_sessions += sess;
            result.buckets.push(TimeBucket {
                ts: chrono::DateTime::parse_from_rfc3339(&r.bucket)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_default(),
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
        let sql = sql::custom_events(q);
        let rows: Vec<row::TopRowJson> = self.query_json(&sql).await?;
        Ok(row::top_list_from_rows(rows))
    }

    async fn query_funnel(&self, q: &FunnelQuery) -> Result<FunnelResult, StoreError> {
        if q.steps.is_empty() {
            return Ok(FunnelResult::default());
        }
        let mut steps_out = Vec::with_capacity(q.steps.len());
        let mut prev_sessions: Option<u64> = None;
        for step in &q.steps {
            let sql = sql::funnel_step(q, &step.event_name);
            #[derive(Deserialize)]
            struct Row {
                sessions: String,
            }
            let rows: Vec<Row> = self.query_json(&sql).await?;
            let sessions = rows
                .first()
                .and_then(|r| r.sessions.parse::<u64>().ok())
                .unwrap_or(0);
            let (cr, drop) = match prev_sessions {
                None => (1.0, 0.0),
                Some(prev) if prev > 0 => {
                    let cr = sessions as f64 / prev as f64;
                    (cr, 1.0 - cr)
                }
                _ => (0.0, 1.0),
            };
            steps_out.push(FunnelStepResult {
                name: step.name.clone(),
                sessions,
                conversion_rate: cr,
                drop_off_rate: drop,
            });
            prev_sessions = Some(sessions);
        }
        Ok(FunnelResult { steps: steps_out })
    }

    async fn query_entry_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<EntryPages, StoreError> {
        let sql = sql::entry_pages(site_id, range, limit, filters);
        #[derive(Deserialize)]
        struct Row {
            value: Option<String>,
            sessions: String,
            bounces: String,
        }
        let rows: Vec<Row> = self.query_json(&sql).await?;
        let mut out = Vec::with_capacity(rows.len());
        let mut total = 0u64;
        for r in rows {
            let sessions = r.sessions.parse::<u64>().unwrap_or(0);
            let bounces = r.bounces.parse::<u64>().unwrap_or(0);
            total += sessions;
            let bounce_rate = if sessions > 0 {
                bounces as f64 / sessions as f64 * 100.0
            } else {
                0.0
            };
            out.push(EntryPageRow {
                url: r.value.unwrap_or_default(),
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
        let sql = sql::exit_pages(site_id, range, limit, filters);
        #[derive(Deserialize)]
        struct Row {
            value: Option<String>,
            exits: String,
            pageviews: String,
        }
        let rows: Vec<Row> = self.query_json(&sql).await?;
        let mut out = Vec::with_capacity(rows.len());
        let mut total = 0u64;
        for r in rows {
            let exits = r.exits.parse::<u64>().unwrap_or(0);
            let pv = r.pageviews.parse::<u64>().unwrap_or(0);
            if exits == 0 {
                continue;
            }
            total += exits;
            let exit_rate = if pv > 0 {
                exits as f64 / pv as f64 * 100.0
            } else {
                0.0
            };
            out.push(ExitPageRow {
                url: r.value.unwrap_or_default(),
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
        #[derive(Deserialize)]
        struct Head {
            active: String,
            pvs: String,
        }
        let head: Vec<Head> = self
            .query_json(&sql::realtime_head(site_id, window))
            .await?;
        let (active_sessions, pageviews) = head
            .first()
            .map(|h| {
                (
                    h.active.parse::<u64>().unwrap_or(0),
                    h.pvs.parse::<u64>().unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        let pageviews_per_minute = pageviews as f64 / window as f64;

        #[derive(Deserialize)]
        struct Page {
            value: Option<String>,
            active: String,
        }
        let pages: Vec<Page> = self
            .query_json(&sql::realtime_pages(site_id, window))
            .await?;
        let top_pages = pages
            .into_iter()
            .map(|p| {
                let a = p.active.parse::<u64>().unwrap_or(0);
                let pct = if active_sessions > 0 {
                    a as f64 / active_sessions as f64 * 100.0
                } else {
                    0.0
                };
                RealtimeTopPage {
                    url: p.value.unwrap_or_default(),
                    active_sessions: a,
                    pct,
                }
            })
            .collect();

        #[derive(Deserialize)]
        struct Ev {
            name: String,
            url: Option<String>,
            seconds_ago: String,
            props: Option<String>,
        }
        let evs: Vec<Ev> = self
            .query_json(&sql::realtime_events(site_id, window))
            .await?;
        let recent_events = evs
            .into_iter()
            .map(|e| RealtimeEvent {
                name: e.name,
                url: e.url.unwrap_or_default(),
                seconds_ago: e.seconds_ago.parse::<i64>().unwrap_or(0),
                properties: e
                    .props
                    .and_then(|p| serde_json::from_str(&p).ok())
                    .unwrap_or(serde_json::Value::Null),
            })
            .collect();

        Ok(RealtimeSnapshot {
            active_sessions,
            pageviews_per_minute,
            top_pages,
            recent_events,
        })
    }

    async fn query_goal(&self, q: &GoalQuery) -> Result<GoalStats, StoreError> {
        #[derive(Deserialize)]
        struct Totals {
            completions: String,
            uniq: String,
        }
        let totals: Vec<Totals> = self.query_json(&sql::goal_totals(q)).await?;
        let (completions, unique_completions) = totals
            .first()
            .map(|t| {
                (
                    t.completions.parse::<u64>().unwrap_or(0),
                    t.uniq.parse::<u64>().unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));

        #[derive(Deserialize)]
        struct Sessions {
            sessions: String,
        }
        let sess: Vec<Sessions> = self
            .query_json(&sql::range_sessions(q.site_id, &q.range))
            .await?;
        let total_sessions = sess
            .first()
            .and_then(|s| s.sessions.parse::<u64>().ok())
            .unwrap_or(0);
        let conversion_rate = if total_sessions > 0 {
            unique_completions as f64 / total_sessions as f64 * 100.0
        } else {
            0.0
        };

        #[derive(Deserialize)]
        struct Bucket {
            date: String,
            uniq: String,
            sessions: String,
        }
        let buckets: Vec<Bucket> = self.query_json(&sql::goal_timeseries(q)).await?;
        let timeseries = buckets
            .into_iter()
            .map(|b| {
                let c = b.uniq.parse::<u64>().unwrap_or(0);
                let s = b.sessions.parse::<u64>().unwrap_or(0);
                let cr = if s > 0 {
                    c as f64 / s as f64 * 100.0
                } else {
                    0.0
                };
                GoalBucket {
                    date: b.date,
                    completions: c,
                    conversion_rate: cr,
                }
            })
            .collect();

        Ok(GoalStats {
            completions,
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
        #[derive(Deserialize)]
        struct Row {
            session_id: String,
            started: String,
            ended: String,
            pageviews: String,
            entry_url: Option<String>,
            exit_url: Option<String>,
            referrer: Option<String>,
            country_code: Option<String>,
            browser: Option<String>,
            os: Option<String>,
            device_type: Option<String>,
            utm_source: Option<String>,
            utm_medium: Option<String>,
            utm_campaign: Option<String>,
        }
        let rows: Vec<Row> = self
            .query_json(&sql::sessions(site_id, range, limit))
            .await?;
        let parse_ts = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| d.with_timezone(&chrono::Utc))
                .unwrap_or_default()
        };
        Ok(rows
            .into_iter()
            .map(|r| {
                let started_at = parse_ts(&r.started);
                let ended_at = parse_ts(&r.ended);
                let pageviews = r.pageviews.parse::<u64>().unwrap_or(0);
                let duration_secs = (ended_at - started_at).num_seconds();
                SessionRow {
                    session_id: r.session_id,
                    started_at,
                    ended_at,
                    duration_secs,
                    pageviews,
                    entry_url: r.entry_url.unwrap_or_default(),
                    exit_url: r.exit_url.unwrap_or_default(),
                    referrer: r.referrer,
                    country_code: r.country_code,
                    browser: r.browser.unwrap_or_default(),
                    os: r.os.unwrap_or_default(),
                    device_type: r.device_type.unwrap_or_default(),
                    utm_source: r.utm_source,
                    utm_medium: r.utm_medium,
                    utm_campaign: r.utm_campaign,
                    is_bounce: pageviews == 1 && duration_secs < 30,
                }
            })
            .collect())
    }

    async fn query_events_list(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<RawEventRow>, StoreError> {
        #[derive(Deserialize)]
        struct Row {
            id: String,
            name: String,
            kind: String,
            ts: String,
            url: Option<String>,
            referrer: Option<String>,
            country_code: Option<String>,
            browser: Option<String>,
            os: Option<String>,
            device_type: Option<String>,
            props: Option<String>,
        }
        let rows: Vec<Row> = self
            .query_json(&sql::events_list(site_id, range, limit))
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| RawEventRow {
                id: r.id,
                name: r.name,
                kind: r.kind,
                timestamp: chrono::DateTime::parse_from_rfc3339(&r.ts)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_default(),
                url: r.url.unwrap_or_default(),
                referrer: r.referrer,
                country_code: r.country_code,
                browser: r.browser.unwrap_or_default(),
                os: r.os.unwrap_or_default(),
                device_type: r.device_type.unwrap_or_default(),
                properties: r.props,
            })
            .collect())
    }
}

impl ClickhouseBackend {
    async fn query_top_field(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        field: &str,
        filters: &[Filter],
    ) -> Result<TopList, StoreError> {
        let sql = sql::top_field(site_id, range, limit, field, filters);
        let rows: Vec<row::TopRowJson> = self.query_json(&sql).await?;
        Ok(row::top_list_from_rows(rows))
    }
}

#[async_trait]
impl AgentStore for ClickhouseBackend {
    async fn ingest_spans(&self, spans: Vec<AgentSpan>) -> Result<(), StoreError> {
        if spans.is_empty() {
            return Ok(());
        }
        let rows: Vec<String> = spans
            .iter()
            .map(|s| {
                serde_json::to_string(&row::SpanRowIn::from(s))
                    .expect("SpanRowIn serialization is infallible")
            })
            .collect();
        let body = format!(
            "INSERT INTO agent_spans FORMAT JSONEachRow\n{}",
            rows.join("\n")
        );
        self.execute(&body).await?;
        Ok(())
    }

    async fn query_spans(&self, q: &SpanQuery) -> Result<Vec<SpanRow>, StoreError> {
        let sql = sql::query_spans(q);
        let rows: Vec<row::SpanRowOut> = self.query_json(&sql).await?;
        Ok(rows.into_iter().map(SpanRow::from).collect())
    }

    async fn summarize_agents(
        &self,
        site_id: Ulid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<AgentSummary>, StoreError> {
        let sql = sql::summarize_agents(site_id, since);
        #[derive(Deserialize)]
        struct Row {
            agent_id: String,
            last_seen: String,
            total_spans: String,
            in_tok: String,
            out_tok: String,
            cost: f64,
        }
        let rows: Vec<Row> = self.query_json(&sql).await?;
        Ok(rows
            .into_iter()
            .map(|r| AgentSummary {
                agent_id: r.agent_id,
                last_seen_at: chrono::DateTime::parse_from_rfc3339(&r.last_seen)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_default(),
                total_spans: r.total_spans.parse().unwrap_or(0),
                total_input_tokens: r.in_tok.parse().unwrap_or(0),
                total_output_tokens: r.out_tok.parse().unwrap_or(0),
                total_cost_usd: r.cost,
            })
            .collect())
    }

    async fn session_cost_usd(
        &self,
        site_id: Ulid,
        agent_session_id: &str,
    ) -> Result<f64, StoreError> {
        let sql = sql::session_cost_usd(site_id, agent_session_id);
        #[derive(Deserialize)]
        struct Row {
            cost: f64,
        }
        let rows: Vec<Row> = self.query_json(&sql).await?;
        Ok(rows.first().map(|r| r.cost).unwrap_or(0.0))
    }
}

// ---------------------------------------------------------------------------
// SQL builders and row types — kept module-private and pure so they can be
// unit-tested without standing up a ClickHouse server.
// ---------------------------------------------------------------------------

mod ddl {
    pub fn all_statements() -> [&'static str; 2] {
        [EVENTS_DDL, SPANS_DDL]
    }

    const EVENTS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS events (
            id String,
            site_id String,
            name LowCardinality(String),
            kind LowCardinality(String),
            timestamp DateTime64(6, 'UTC'),
            received_at DateTime64(6, 'UTC'),
            url String,
            referrer Nullable(String),
            utm_source LowCardinality(Nullable(String)),
            utm_medium LowCardinality(Nullable(String)),
            utm_campaign LowCardinality(Nullable(String)),
            utm_term Nullable(String),
            utm_content Nullable(String),
            browser LowCardinality(String),
            browser_version LowCardinality(String),
            os LowCardinality(String),
            os_version LowCardinality(String),
            device_type LowCardinality(String),
            screen_width Nullable(UInt16),
            screen_height Nullable(UInt16),
            language LowCardinality(Nullable(String)),
            ip_anonymized String,
            country_code LowCardinality(Nullable(String)),
            region LowCardinality(Nullable(String)),
            city Nullable(String),
            session_id String,
            properties Nullable(String)
        ) ENGINE = MergeTree
        PARTITION BY toYYYYMM(timestamp)
        ORDER BY (site_id, timestamp)
    "#;

    const SPANS_DDL: &str = r#"
        CREATE TABLE IF NOT EXISTS agent_spans (
            id String,
            site_id String,
            agent_id LowCardinality(String),
            agent_session_id String,
            parent_span_id Nullable(String),
            kind LowCardinality(String),
            model LowCardinality(String),
            started_at DateTime64(6, 'UTC'),
            ended_at DateTime64(6, 'UTC'),
            input_tokens UInt32,
            output_tokens UInt32,
            cache_read_tokens UInt32,
            cache_creation_tokens UInt32,
            cost_usd Float64,
            tool_name LowCardinality(Nullable(String)),
            tool_input_hash Nullable(String),
            stop_reason LowCardinality(Nullable(String)),
            properties Nullable(String)
        ) ENGINE = MergeTree
        PARTITION BY toYYYYMM(started_at)
        ORDER BY (site_id, started_at, agent_id)
    "#;
}

mod sql {
    use chrono::{DateTime, Utc};
    use ulid::Ulid;

    use stomatopod_core::query::{
        analytics::GoalQuery,
        events::EventQuery,
        funnel::FunnelQuery,
        pageviews::{Filter, FilterOp, Granularity, PageviewsQuery, TimeRange},
        spans::SpanQuery,
    };

    /// Escape a single-quoted SQL literal. ClickHouse uses backslash
    /// escapes; we double the slashes/quotes to avoid SQL injection.
    pub(super) fn quote(s: &str) -> String {
        let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
        format!("'{escaped}'")
    }

    /// Build the `AND <col> <op> '<value>'` fragment for analytics filters.
    /// Columns come from the `FilterField` enum (never user input); values
    /// are quote-escaped. `LIKE` patterns rely on ClickHouse's default
    /// backslash escape character, matching [`Filter::sql_value`].
    pub(super) fn filter_clause(filters: &[Filter]) -> String {
        let mut out = String::new();
        for f in filters {
            // For LIKE the pattern already carries backslash escapes; for the
            // others the raw value is escaped by `quote`.
            let lit = match f.op {
                FilterOp::Contains | FilterOp::StartsWith => quote(&f.sql_value()),
                _ => quote(&f.value),
            };
            out.push_str(&format!(
                " AND {} {} {lit}",
                f.field.column(),
                f.op.sql_operator()
            ));
        }
        out
    }

    fn date_trunc(g: &Granularity) -> &'static str {
        match g {
            Granularity::Hour => "toStartOfHour",
            Granularity::Day => "toStartOfDay",
            Granularity::Week => "toStartOfWeek",
            Granularity::Month => "toStartOfMonth",
        }
    }

    fn micros(dt: DateTime<Utc>) -> String {
        format!("fromUnixTimestamp64Micro({})", dt.timestamp_micros())
    }

    pub fn pageviews(q: &PageviewsQuery) -> String {
        format!(
            "SELECT \
                formatDateTime({fn}(timestamp), '%Y-%m-%dT%H:%M:%S.000000Z') AS bucket, \
                toString(countIf(kind = 'pageview')) AS pageviews, \
                toString(uniqExact(session_id)) AS sessions \
             FROM events \
             WHERE site_id = {site} \
               AND timestamp >= {start} \
               AND timestamp <= {end} \
               {filters} \
             GROUP BY bucket \
             ORDER BY bucket",
            fn = date_trunc(&q.granularity),
            site = quote(&q.site_id.to_string()),
            start = micros(q.range.start),
            end = micros(q.range.end),
            filters = filter_clause(&q.filters),
        )
    }

    pub fn top_field(
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        field: &str,
        filters: &[Filter],
    ) -> String {
        // `field` is a static identifier from the public API surface (url,
        // referrer, country_code, browser, device_type, os, region); never
        // user input.
        format!(
            "SELECT \
                coalesce({field}, 'Direct / None') AS value, \
                toString(count()) AS pageviews, \
                toString(uniqExact(session_id)) AS sessions \
             FROM events \
             WHERE site_id = {site} \
               AND timestamp >= {start} \
               AND timestamp <= {end} \
               AND kind = 'pageview' \
               {filters} \
             GROUP BY value \
             ORDER BY count() DESC \
             LIMIT {limit}",
            site = quote(&site_id.to_string()),
            start = micros(range.start),
            end = micros(range.end),
            filters = filter_clause(filters),
        )
    }

    pub fn custom_events(q: &EventQuery) -> String {
        let name_filter = q
            .event_name
            .as_deref()
            .map(|n| format!("AND name = {}", quote(n)))
            .unwrap_or_default();
        format!(
            "SELECT \
                name AS value, \
                toString(count()) AS pageviews, \
                toString(uniqExact(session_id)) AS sessions \
             FROM events \
             WHERE site_id = {site} \
               AND timestamp >= {start} \
               AND timestamp <= {end} \
               AND kind = 'custom' \
               {name_filter} \
             GROUP BY value \
             ORDER BY count() DESC \
             LIMIT {limit}",
            site = quote(&q.site_id.to_string()),
            start = micros(q.range.start),
            end = micros(q.range.end),
            limit = q.limit,
        )
    }

    pub fn funnel_step(q: &FunnelQuery, event_name: &str) -> String {
        format!(
            "SELECT toString(uniqExact(session_id)) AS sessions \
             FROM events \
             WHERE site_id = {site} \
               AND timestamp >= {start} \
               AND timestamp <= {end} \
               AND name = {name}",
            site = quote(&q.site_id.to_string()),
            start = micros(q.range.start),
            end = micros(q.range.end),
            name = quote(event_name),
        )
    }

    pub fn entry_pages(site_id: Ulid, range: &TimeRange, limit: u32, filters: &[Filter]) -> String {
        format!(
            "SELECT value, toString(count()) AS sessions, toString(countIf(pv_count = 1)) AS bounces \
             FROM (\
                SELECT coalesce(url, '') AS value, session_id, \
                    row_number() OVER (PARTITION BY session_id ORDER BY timestamp ASC) AS rn, \
                    count() OVER (PARTITION BY session_id) AS pv_count \
                FROM events \
                WHERE site_id = {site} AND timestamp >= {start} AND timestamp <= {end} \
                  AND kind = 'pageview' {filters}) \
             WHERE rn = 1 GROUP BY value ORDER BY count() DESC LIMIT {limit}",
            site = quote(&site_id.to_string()),
            start = micros(range.start),
            end = micros(range.end),
            filters = filter_clause(filters),
        )
    }

    pub fn exit_pages(site_id: Ulid, range: &TimeRange, limit: u32, filters: &[Filter]) -> String {
        format!(
            "SELECT value, toString(countIf(rn = 1)) AS exits, toString(count()) AS pageviews \
             FROM (\
                SELECT coalesce(url, '') AS value, \
                    row_number() OVER (PARTITION BY session_id ORDER BY timestamp DESC) AS rn \
                FROM events \
                WHERE site_id = {site} AND timestamp >= {start} AND timestamp <= {end} \
                  AND kind = 'pageview' {filters}) \
             GROUP BY value ORDER BY exits DESC LIMIT {limit}",
            site = quote(&site_id.to_string()),
            start = micros(range.start),
            end = micros(range.end),
            filters = filter_clause(filters),
        )
    }

    pub fn realtime_head(site_id: Ulid, window_minutes: u32) -> String {
        format!(
            "SELECT toString(uniqExact(session_id)) AS active, \
                    toString(countIf(kind = 'pageview')) AS pvs \
             FROM events \
             WHERE site_id = {site} AND timestamp >= now() - INTERVAL {w} MINUTE",
            site = quote(&site_id.to_string()),
            w = window_minutes,
        )
    }

    pub fn realtime_pages(site_id: Ulid, window_minutes: u32) -> String {
        format!(
            "SELECT value, toString(uniqExact(session_id)) AS active FROM (\
                SELECT coalesce(url, '') AS value, session_id, \
                    row_number() OVER (PARTITION BY session_id ORDER BY timestamp DESC) AS rn \
                FROM events \
                WHERE site_id = {site} AND timestamp >= now() - INTERVAL {w} MINUTE \
                  AND kind = 'pageview') \
             WHERE rn = 1 GROUP BY value ORDER BY active DESC LIMIT 10",
            site = quote(&site_id.to_string()),
            w = window_minutes,
        )
    }

    pub fn realtime_events(site_id: Ulid, window_minutes: u32) -> String {
        format!(
            "SELECT name, coalesce(url, '') AS url, \
                    toString(toInt64(now() - timestamp)) AS seconds_ago, properties AS props \
             FROM events \
             WHERE site_id = {site} AND timestamp >= now() - INTERVAL {w} MINUTE AND kind = 'custom' \
             ORDER BY timestamp DESC LIMIT 50",
            site = quote(&site_id.to_string()),
            w = window_minutes,
        )
    }

    pub fn goal_totals(q: &GoalQuery) -> String {
        format!(
            "SELECT toString(count()) AS completions, toString(uniqExact(session_id)) AS uniq \
             FROM events \
             WHERE site_id = {site} AND timestamp >= {start} AND timestamp <= {end} \
               AND kind = 'custom' AND name = {name} {filters}",
            site = quote(&q.site_id.to_string()),
            start = micros(q.range.start),
            end = micros(q.range.end),
            name = quote(&q.event_name),
            filters = filter_clause(&q.filters),
        )
    }

    pub fn range_sessions(site_id: Ulid, range: &TimeRange) -> String {
        format!(
            "SELECT toString(uniqExact(session_id)) AS sessions FROM events \
             WHERE site_id = {site} AND timestamp >= {start} AND timestamp <= {end}",
            site = quote(&site_id.to_string()),
            start = micros(range.start),
            end = micros(range.end),
        )
    }

    pub fn goal_timeseries(q: &GoalQuery) -> String {
        let trunc = date_trunc(&q.granularity);
        format!(
            "SELECT comp.date AS date, toString(comp.uniq) AS uniq, \
                    toString(coalesce(sess.sessions, 0)) AS sessions FROM (\
                SELECT formatDateTime({trunc}(timestamp), '%Y-%m-%d') AS date, \
                    uniqExact(session_id) AS uniq \
                FROM events WHERE site_id = {site} AND timestamp >= {start} AND timestamp <= {end} \
                  AND kind = 'custom' AND name = {name} {filters} GROUP BY date) AS comp \
             LEFT JOIN (\
                SELECT formatDateTime({trunc}(timestamp), '%Y-%m-%d') AS date, \
                    uniqExact(session_id) AS sessions \
                FROM events WHERE site_id = {site} AND timestamp >= {start} AND timestamp <= {end} \
                GROUP BY date) AS sess USING (date) \
             ORDER BY date",
            site = quote(&q.site_id.to_string()),
            start = micros(q.range.start),
            end = micros(q.range.end),
            name = quote(&q.event_name),
            filters = filter_clause(&q.filters),
        )
    }

    pub fn sessions(site_id: Ulid, range: &TimeRange, limit: u32) -> String {
        format!(
            "SELECT lower(hex(session_id)) AS session_id, \
                    formatDateTime(min(timestamp), '%Y-%m-%dT%H:%M:%S.000000Z') AS started, \
                    formatDateTime(max(timestamp), '%Y-%m-%dT%H:%M:%S.000000Z') AS ended, \
                    toString(count()) AS pageviews, \
                    argMin(url, timestamp) AS entry_url, \
                    argMax(url, timestamp) AS exit_url, \
                    argMin(referrer, timestamp) AS referrer, \
                    argMin(country_code, timestamp) AS country_code, \
                    argMin(browser, timestamp) AS browser, \
                    argMin(os, timestamp) AS os, \
                    argMin(device_type, timestamp) AS device_type, \
                    argMin(utm_source, timestamp) AS utm_source, \
                    argMin(utm_medium, timestamp) AS utm_medium, \
                    argMin(utm_campaign, timestamp) AS utm_campaign \
             FROM events \
             WHERE site_id = {site} AND timestamp >= {start} AND timestamp <= {end} AND kind = 'pageview' \
             GROUP BY session_id ORDER BY started DESC LIMIT {limit}",
            site = quote(&site_id.to_string()),
            start = micros(range.start),
            end = micros(range.end),
        )
    }

    pub fn events_list(site_id: Ulid, range: &TimeRange, limit: u32) -> String {
        format!(
            "SELECT id, name, kind, \
                    formatDateTime(timestamp, '%Y-%m-%dT%H:%M:%S.000000Z') AS ts, \
                    coalesce(url, '') AS url, referrer, country_code, browser, os, device_type, \
                    properties AS props \
             FROM events \
             WHERE site_id = {site} AND timestamp >= {start} AND timestamp <= {end} \
             ORDER BY timestamp DESC LIMIT {limit}",
            site = quote(&site_id.to_string()),
            start = micros(range.start),
            end = micros(range.end),
        )
    }

    pub fn query_spans(q: &SpanQuery) -> String {
        let agent_filter = q
            .agent_id
            .as_deref()
            .map(|a| format!("AND agent_id = {}", quote(a)))
            .unwrap_or_default();
        let session_filter = q
            .session_id
            .as_deref()
            .map(|s| format!("AND agent_session_id = {}", quote(s)))
            .unwrap_or_default();
        format!(
            "SELECT id, agent_id, agent_session_id, kind, model, \
                    formatDateTime(started_at, '%Y-%m-%dT%H:%M:%S.000000Z') AS started_at, \
                    formatDateTime(ended_at, '%Y-%m-%dT%H:%M:%S.000000Z') AS ended_at, \
                    input_tokens, output_tokens, cost_usd, tool_name, stop_reason \
             FROM agent_spans \
             WHERE site_id = {site} \
               AND started_at >= {start} \
               AND started_at <= {end} \
               {agent_filter} \
               {session_filter} \
             ORDER BY started_at DESC \
             LIMIT {limit}",
            site = quote(&q.site_id.to_string()),
            start = micros(q.since),
            end = micros(q.until),
            limit = q.limit,
        )
    }

    pub fn summarize_agents(site_id: Ulid, since: DateTime<Utc>) -> String {
        format!(
            "SELECT \
                agent_id, \
                formatDateTime(max(started_at), '%Y-%m-%dT%H:%M:%S.000000Z') AS last_seen, \
                toString(count()) AS total_spans, \
                toString(sum(input_tokens)) AS in_tok, \
                toString(sum(output_tokens)) AS out_tok, \
                sum(cost_usd) AS cost \
             FROM agent_spans \
             WHERE site_id = {site} \
               AND started_at >= {start} \
             GROUP BY agent_id \
             ORDER BY last_seen DESC",
            site = quote(&site_id.to_string()),
            start = micros(since),
        )
    }

    pub fn session_cost_usd(site_id: Ulid, agent_session_id: &str) -> String {
        format!(
            "SELECT sum(cost_usd) AS cost \
             FROM agent_spans \
             WHERE site_id = {site} \
               AND agent_session_id = {session}",
            site = quote(&site_id.to_string()),
            session = quote(agent_session_id),
        )
    }
}

mod row {
    //! Serde adaptors between the domain types and ClickHouse's JSONEachRow
    //! wire format.

    use chrono::SecondsFormat;
    use serde::{Deserialize, Serialize};

    use stomatopod_core::{
        domain::{
            agent_span::AgentSpan,
            event::{DeviceType, Event, EventKind},
        },
        query::{pageviews::TopList, spans::SpanRow},
    };

    #[derive(Serialize)]
    pub struct EventRow<'a> {
        pub id: String,
        pub site_id: String,
        pub name: &'a str,
        pub kind: &'static str,
        pub timestamp: String,
        pub received_at: String,
        pub url: &'a str,
        pub referrer: Option<&'a str>,
        pub utm_source: Option<&'a str>,
        pub utm_medium: Option<&'a str>,
        pub utm_campaign: Option<&'a str>,
        pub utm_term: Option<&'a str>,
        pub utm_content: Option<&'a str>,
        pub browser: &'a str,
        pub browser_version: &'a str,
        pub os: &'a str,
        pub os_version: &'a str,
        pub device_type: &'static str,
        pub screen_width: Option<u16>,
        pub screen_height: Option<u16>,
        pub language: Option<&'a str>,
        pub ip_anonymized: &'a str,
        pub country_code: Option<&'a str>,
        pub region: Option<&'a str>,
        pub city: Option<&'a str>,
        /// Hex-encoded so ClickHouse's `String` column accepts the bytes
        /// without any escape ambiguity.
        pub session_id: String,
        /// `properties` rendered as a JSON string (the column is
        /// `Nullable(String)`, so we serialize it ourselves rather than
        /// embedding raw JSON).
        pub properties: Option<String>,
    }

    impl<'a> From<&'a Event> for EventRow<'a> {
        fn from(e: &'a Event) -> Self {
            Self {
                id: e.id.to_string(),
                site_id: e.site_id.to_string(),
                name: &e.name,
                kind: kind_str(e.kind),
                timestamp: rfc3339_micros(e.timestamp),
                received_at: rfc3339_micros(e.received_at),
                url: &e.url,
                referrer: e.referrer.as_deref(),
                utm_source: e.utm_source.as_deref(),
                utm_medium: e.utm_medium.as_deref(),
                utm_campaign: e.utm_campaign.as_deref(),
                utm_term: e.utm_term.as_deref(),
                utm_content: e.utm_content.as_deref(),
                browser: &e.browser,
                browser_version: &e.browser_version,
                os: &e.os,
                os_version: &e.os_version,
                device_type: device_type_str(e.device_type),
                screen_width: e.screen_width,
                screen_height: e.screen_height,
                language: e.language.as_deref(),
                ip_anonymized: &e.ip_anonymized,
                country_code: e.country_code.as_deref(),
                region: e.region.as_deref(),
                city: e.city.as_deref(),
                session_id: hex::encode(e.session_id),
                properties: e.properties.as_ref().map(|p| p.to_string()),
            }
        }
    }

    fn kind_str(k: EventKind) -> &'static str {
        match k {
            EventKind::Pageview => "pageview",
            EventKind::Custom => "custom",
        }
    }

    fn device_type_str(d: DeviceType) -> &'static str {
        match d {
            DeviceType::Desktop => "desktop",
            DeviceType::Mobile => "mobile",
            DeviceType::Tablet => "tablet",
            DeviceType::Unknown => "unknown",
        }
    }

    #[derive(Serialize)]
    pub struct SpanRowIn<'a> {
        pub id: String,
        pub site_id: String,
        pub agent_id: &'a str,
        pub agent_session_id: &'a str,
        pub parent_span_id: Option<String>,
        pub kind: &'static str,
        pub model: &'a str,
        pub started_at: String,
        pub ended_at: String,
        pub input_tokens: u32,
        pub output_tokens: u32,
        pub cache_read_tokens: u32,
        pub cache_creation_tokens: u32,
        pub cost_usd: f64,
        pub tool_name: Option<&'a str>,
        pub tool_input_hash: Option<&'a str>,
        pub stop_reason: Option<&'a str>,
        pub properties: Option<String>,
    }

    impl<'a> From<&'a AgentSpan> for SpanRowIn<'a> {
        fn from(s: &'a AgentSpan) -> Self {
            Self {
                id: s.id.to_string(),
                site_id: s.site_id.to_string(),
                agent_id: &s.agent_id,
                agent_session_id: &s.agent_session_id,
                parent_span_id: s.parent_span_id.map(|p| p.to_string()),
                kind: s.kind.as_str(),
                model: &s.model,
                started_at: rfc3339_micros(s.started_at),
                ended_at: rfc3339_micros(s.ended_at),
                input_tokens: s.input_tokens,
                output_tokens: s.output_tokens,
                cache_read_tokens: s.cache_read_tokens,
                cache_creation_tokens: s.cache_creation_tokens,
                cost_usd: s.cost_usd,
                tool_name: s.tool_name.as_deref(),
                tool_input_hash: s.tool_input_hash.as_deref(),
                stop_reason: s.stop_reason.as_deref(),
                properties: s.properties.as_ref().map(|p| p.to_string()),
            }
        }
    }

    #[derive(Deserialize)]
    pub struct SpanRowOut {
        pub id: String,
        pub agent_id: String,
        pub agent_session_id: String,
        pub kind: String,
        pub model: String,
        pub started_at: String,
        pub ended_at: String,
        pub input_tokens: u32,
        pub output_tokens: u32,
        pub cost_usd: f64,
        pub tool_name: Option<String>,
        pub stop_reason: Option<String>,
    }

    impl From<SpanRowOut> for SpanRow {
        fn from(r: SpanRowOut) -> Self {
            SpanRow {
                id: r.id,
                agent_id: r.agent_id,
                agent_session_id: r.agent_session_id,
                kind: r.kind,
                model: r.model,
                started_at: chrono::DateTime::parse_from_rfc3339(&r.started_at)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_default(),
                ended_at: chrono::DateTime::parse_from_rfc3339(&r.ended_at)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_default(),
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cost_usd: r.cost_usd,
                tool_name: r.tool_name,
                stop_reason: r.stop_reason,
            }
        }
    }

    #[derive(Deserialize)]
    pub struct TopRowJson {
        pub value: Option<String>,
        pub pageviews: String,
        pub sessions: String,
    }

    pub fn top_list_from_rows(rows: Vec<TopRowJson>) -> TopList {
        use stomatopod_core::query::pageviews::TopRow;
        let mut out: Vec<TopRow> = rows
            .into_iter()
            .map(|r| TopRow {
                value: r.value.unwrap_or_else(|| "Direct / None".into()),
                pageviews: r.pageviews.parse().unwrap_or(0),
                sessions: r.sessions.parse().unwrap_or(0),
                pct: 0.0,
            })
            .collect();
        let total: u64 = out.iter().map(|r| r.pageviews).sum();
        if total > 0 {
            for row in &mut out {
                row.pct = (row.pageviews as f64 / total as f64) * 100.0;
            }
        }
        TopList { rows: out }
    }

    fn rfc3339_micros(dt: chrono::DateTime<chrono::Utc>) -> String {
        // ClickHouse `parseDateTime64BestEffort` and the implicit cast on
        // `DateTime64(6, 'UTC')` accept RFC3339 with microsecond precision.
        dt.to_rfc3339_opts(SecondsFormat::Micros, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use stomatopod_core::query::pageviews::Granularity;

    fn fixed_ts() -> chrono::DateTime<chrono::Utc> {
        Utc.with_ymd_and_hms(2026, 5, 20, 12, 30, 45).unwrap()
    }

    #[test]
    fn pageviews_sql_uses_hour_bucket_and_parameter_binding() {
        let site = Ulid::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let q = PageviewsQuery {
            site_id: site,
            range: TimeRange {
                start: fixed_ts(),
                end: fixed_ts() + chrono::Duration::hours(1),
            },
            granularity: Granularity::Hour,
            filters: vec![],
        };
        let sql = sql::pageviews(&q);
        assert!(sql.contains("toStartOfHour"));
        assert!(sql.contains("countIf(kind = 'pageview')"));
        assert!(sql.contains("uniqExact(session_id)"));
        assert!(sql.contains("site_id = '01ARZ3NDEKTSV4RRFFQ69G5FAV'"));
        assert!(sql.contains("fromUnixTimestamp64Micro"));
    }

    #[test]
    fn sql_quote_escapes_single_quote_and_backslash() {
        assert_eq!(sql::quote("a'b"), "'a\\'b'");
        assert_eq!(sql::quote("a\\b"), "'a\\\\b'");
        assert_eq!(
            sql::quote("'; DROP TABLE events; --"),
            "'\\'; DROP TABLE events; --'"
        );
    }

    #[test]
    fn top_field_sql_pins_field_name() {
        let site = Ulid::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let range = TimeRange {
            start: fixed_ts(),
            end: fixed_ts() + chrono::Duration::hours(1),
        };
        let sql = sql::top_field(site, &range, 5, "url", &[]);
        assert!(sql.contains("coalesce(url, 'Direct / None')"));
        assert!(sql.contains("LIMIT 5"));
        assert!(sql.contains("kind = 'pageview'"));
    }

    #[test]
    fn filter_clause_builds_and_fragments() {
        use stomatopod_core::query::pageviews::Filter;
        let filters = vec![
            Filter::parse("country:eq:US").unwrap(),
            Filter::parse("url:contains:/blog").unwrap(),
        ];
        let frag = sql::filter_clause(&filters);
        assert!(frag.contains("AND country_code = 'US'"));
        assert!(frag.contains("AND url LIKE '%/blog%'"));
    }

    #[test]
    fn query_spans_sql_applies_filters_when_present() {
        let site = Ulid::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let q = SpanQuery {
            site_id: site,
            agent_id: Some("agent-a".into()),
            session_id: Some("sess-1".into()),
            since: fixed_ts(),
            until: fixed_ts() + chrono::Duration::hours(1),
            limit: 100,
        };
        let sql = sql::query_spans(&q);
        assert!(sql.contains("agent_id = 'agent-a'"));
        assert!(sql.contains("agent_session_id = 'sess-1'"));
        assert!(sql.contains("LIMIT 100"));
        assert!(sql.contains("ORDER BY started_at DESC"));
    }

    #[test]
    fn query_spans_sql_skips_optional_filters_when_absent() {
        let site = Ulid::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let q = SpanQuery {
            site_id: site,
            agent_id: None,
            session_id: None,
            since: fixed_ts(),
            until: fixed_ts() + chrono::Duration::hours(1),
            limit: 100,
        };
        let sql = sql::query_spans(&q);
        assert!(!sql.contains("agent_id ="));
        assert!(!sql.contains("agent_session_id ="));
    }

    #[test]
    fn ddl_contains_both_tables_with_partition_keys() {
        let stmts = ddl::all_statements().join("\n");
        assert!(stmts.contains("events"));
        assert!(stmts.contains("agent_spans"));
        assert!(stmts.contains("PARTITION BY toYYYYMM(timestamp)"));
        assert!(stmts.contains("PARTITION BY toYYYYMM(started_at)"));
    }

    #[test]
    fn event_row_serializes_session_id_as_hex_and_props_as_string() {
        let event = Event {
            id: Ulid::new(),
            site_id: Ulid::new(),
            name: "purchase".into(),
            kind: stomatopod_core::domain::event::EventKind::Custom,
            timestamp: fixed_ts(),
            received_at: fixed_ts(),
            url: "/checkout".into(),
            referrer: None,
            utm_source: None,
            utm_medium: None,
            utm_campaign: None,
            utm_term: None,
            utm_content: None,
            browser: "Firefox".into(),
            browser_version: "120".into(),
            os: "Linux".into(),
            os_version: "6.18".into(),
            device_type: stomatopod_core::domain::event::DeviceType::Desktop,
            screen_width: None,
            screen_height: None,
            language: None,
            ip_anonymized: "127.0.0.0".into(),
            country_code: None,
            region: None,
            city: None,
            session_id: [0xAB; 16],
            properties: Some(serde_json::json!({"sku": "x"})),
        };
        let json = serde_json::to_string(&row::EventRow::from(&event)).unwrap();
        assert!(
            json.contains("\"session_id\":\"abababababababababababababababab\""),
            "session_id should be hex-encoded: {json}"
        );
        assert!(json.contains("\"properties\":\"{\\\"sku\\\":\\\"x\\\"}\""));
        assert!(json.contains("\"kind\":\"custom\""));
        assert!(json.contains("\"device_type\":\"desktop\""));
    }
}
