use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use datafusion::{
    datasource::listing::{ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl},
    execution::context::{SessionConfig, SessionContext},
};
use parking_lot::RwLock;
use std::collections::HashSet;
use tracing::info;
use ulid::Ulid;

use stomatopod_core::{
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
};

pub struct EmbeddedReader {
    ctx: SessionContext,
    data_dir: PathBuf,
    registered: RwLock<HashSet<String>>,
}

impl EmbeddedReader {
    pub async fn new(data_dir: PathBuf) -> Result<Self> {
        // Cap parallel partitions: self-hosted boxes rarely benefit from
        // DataFusion's default (num_cpus) and it multiplies memory use.
        let config = SessionConfig::new().with_target_partitions(2);
        let ctx = SessionContext::new_with_config(config);

        // Register all existing site directories on startup
        let reader = Self {
            ctx,
            data_dir: data_dir.clone(),
            registered: RwLock::new(HashSet::new()),
        };

        if data_dir.exists() {
            let mut entries = tokio::fs::read_dir(&data_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if entry.file_type().await?.is_dir() {
                    let site_id = entry.file_name().to_string_lossy().to_string();
                    if let Err(e) = migrate_flat_date_dirs(&entry.path()).await {
                        tracing::warn!(
                            "Could not migrate flat date partitions for site {site_id}: {e}"
                        );
                    }
                    if let Err(e) = reader.register_site(&site_id).await {
                        tracing::warn!("Could not register site {site_id}: {e}");
                    }
                }
            }
        }

        Ok(reader)
    }

    /// Register a site's Parquet directory as a DataFusion ListingTable.
    /// Idempotent — safe to call for already-registered sites.
    pub async fn ensure_site_registered(&self, site_id: &str) -> Result<()> {
        if self.registered.read().contains(site_id) {
            return Ok(());
        }
        self.register_site(site_id).await
    }

    /// Ensure registration has been attempted and report whether a DataFusion
    /// table is actually available for this site.
    async fn ensure_site_table_available(&self, site_id: &str) -> Result<bool, StoreError> {
        self.ensure_site_registered(site_id)
            .await
            .map_err(StoreError::db)?;
        Ok(self.registered.read().contains(site_id))
    }

    /// Drop Hive `date=` partitions strictly older than `cutoff` and
    /// re-register affected sites so DataFusion drops the deleted files.
    /// Returns the number of partition directories removed.
    pub async fn prune_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, StoreError> {
        let cutoff_day = cutoff.date_naive();
        if !self.data_dir.exists() {
            return Ok(0);
        }
        let mut removed = 0u64;
        let mut touched_sites: Vec<String> = Vec::new();
        let mut sites = tokio::fs::read_dir(&self.data_dir)
            .await
            .map_err(StoreError::db)?;
        while let Some(site_entry) = sites.next_entry().await.map_err(StoreError::db)? {
            if !site_entry
                .file_type()
                .await
                .map_err(StoreError::db)?
                .is_dir()
            {
                continue;
            }
            let site_id = site_entry.file_name().to_string_lossy().to_string();
            let site_dir = site_entry.path();
            let mut parts = tokio::fs::read_dir(&site_dir)
                .await
                .map_err(StoreError::db)?;
            let mut site_touched = false;
            while let Some(part) = parts.next_entry().await.map_err(StoreError::db)? {
                if !part.file_type().await.map_err(StoreError::db)?.is_dir() {
                    continue;
                }
                let name = part.file_name().to_string_lossy().to_string();
                let date_str = name.strip_prefix("date=").unwrap_or(&name);
                let Ok(day) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
                    continue;
                };
                if day < cutoff_day {
                    tokio::fs::remove_dir_all(part.path())
                        .await
                        .map_err(StoreError::db)?;
                    removed += 1;
                    site_touched = true;
                }
            }
            if site_touched {
                touched_sites.push(site_id);
            }
        }
        for site_id in touched_sites {
            self.reregister_site(&site_id)
                .await
                .map_err(StoreError::db)?;
        }
        Ok(removed)
    }

    /// Drop and re-register a site's ListingTable after partition changes.
    async fn reregister_site(&self, site_id: &str) -> Result<()> {
        let name = table_name(site_id);
        let _ = self.ctx.deregister_table(&name);
        self.registered.write().remove(site_id);
        self.register_site(site_id).await
    }

    async fn register_site(&self, site_id: &str) -> Result<()> {
        let site_dir = self.data_dir.join(site_id);
        if !site_dir.exists() {
            return Ok(());
        }
        let site_dir = site_dir.canonicalize()?;

        // Trailing slash is required: without it object_store treats the
        // path as a single file rather than a directory prefix.
        let url = ListingTableUrl::parse(format!("file://{}/", site_dir.to_string_lossy()))?;

        let file_format =
            Arc::new(datafusion::datasource::file_format::parquet::ParquetFormat::default());
        let listing_options = ListingOptions::new(file_format)
            .with_file_extension(".parquet")
            .with_collect_stat(true);

        let table_name = table_name(site_id);
        let config = ListingTableConfig::new(url).with_listing_options(listing_options);
        let config = config.infer_schema(&self.ctx.state()).await?;
        let table = ListingTable::try_new(config)?;

        self.ctx.register_table(&table_name, Arc::new(table))?;
        self.registered.write().insert(site_id.to_string());
        info!("Registered DataFusion table: {table_name}");
        Ok(())
    }

    pub async fn query_pageviews(&self, q: &PageviewsQuery) -> Result<PageviewsResult, StoreError> {
        let site_id = q.site_id.to_string();
        if !self.ensure_site_table_available(&site_id).await? {
            return Ok(PageviewsResult::default());
        }

        let table = table_name(&site_id);
        let granularity_fn = granularity_trunc(&q.granularity);
        let start = q.range.start.timestamp_micros();
        let end = q.range.end.timestamp_micros();

        // `timestamp` is a reserved keyword in DataFusion's SQL parser (it
        // expects `timestamp '2024-...'` literal syntax after it); quote the
        // column to force identifier parsing. By strictly pre-filtering on
        // `kind = 'pageview'`, we can use standard `COUNT(*)` and avoid conditional
        // aggregate hacks.
        //
        // Each aggregate is wrapped in `CAST(... AS BIGINT)` so the read
        // side can rely on a single `Int64Array` downcast — DataFusion
        // promotes integer aggregates to wider types and the column types
        // would otherwise differ between SUM and COUNT.
        let filters = datafusion_filter_clause(&q.filters);
        let sql = format!(
            r#"
            SELECT
                date_trunc('{granularity_fn}', "timestamp") AS bucket,
                CAST(COUNT(*) AS BIGINT) AS pageviews,
                CAST(COUNT(DISTINCT session_id) AS BIGINT) AS sessions
            FROM {table}
            WHERE site_id = '{site_id}'
              AND "timestamp" >= to_timestamp_micros({start})
              AND "timestamp" <= to_timestamp_micros({end})
              AND CAST(kind AS VARCHAR) = 'pageview'
              {filters}
            GROUP BY 1
            ORDER BY 1
            "#
        );

        let df = self.ctx.sql(&sql).await.map_err(StoreError::query)?;
        let batches = df.collect().await.map_err(StoreError::query)?;

        let mut result = PageviewsResult::default();
        for batch in &batches {
            // `date_trunc` returns `Timestamp(Nanosecond, …)` regardless of
            // the input precision, so downcast as nanos and rescale.
            use arrow::array::{Int64Array, TimestampNanosecondArray};
            let bucket_col = batch
                .column_by_name("bucket")
                .and_then(|c| c.as_any().downcast_ref::<TimestampNanosecondArray>());
            let pv_col = batch
                .column_by_name("pageviews")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
            let sess_col = batch
                .column_by_name("sessions")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());

            if let (Some(buckets), Some(pvs), Some(sessions)) = (bucket_col, pv_col, sess_col) {
                for i in 0..batch.num_rows() {
                    let ts_ns = buckets.value(i);
                    let ts =
                        chrono::DateTime::from_timestamp_nanos(ts_ns).with_timezone(&chrono::Utc);
                    let pv = pvs.value(i) as u64;
                    let sess = sessions.value(i) as u64;
                    result.total_pageviews += pv;
                    result.total_sessions += sess;
                    result.buckets.push(TimeBucket {
                        ts,
                        pageviews: pv,
                        sessions: sess,
                    });
                }
            }
        }

        // Session-level bounce rate and average duration over the full range
        // (not per-bucket). A bounce is a session with exactly one pageview,
        // matching entry-page bounce semantics.
        if result.total_pageviews > 0 {
            let (bounce_rate, avg_duration_secs) =
                self.session_summary(&site_id, start, end, &filters).await?;
            result.bounce_rate = bounce_rate;
            result.avg_duration_secs = avg_duration_secs;
        }

        Ok(result)
    }

    /// Aggregate bounce rate and mean session duration for pageviews in range.
    async fn session_summary(
        &self,
        site_id: &str,
        start: i64,
        end: i64,
        filters: &str,
    ) -> Result<(f64, f64), StoreError> {
        let table = table_name(site_id);
        // Duration uses epoch seconds so DataFusion can AVG without depending
        // on interval arithmetic. Single-pageview sessions contribute 0s.
        let sql = format!(
            r#"
            WITH sessions AS (
                SELECT
                    session_id,
                    COUNT(*) AS pv_count,
                    date_part('epoch', MIN("timestamp")) AS started_s,
                    date_part('epoch', MAX("timestamp")) AS ended_s
                FROM {table}
                WHERE site_id = '{site_id}'
                  AND "timestamp" >= to_timestamp_micros({start})
                  AND "timestamp" <= to_timestamp_micros({end})
                  AND CAST(kind AS VARCHAR) = 'pageview'
                  {filters}
                GROUP BY session_id
            )
            SELECT
                CAST(COUNT(*) AS BIGINT) AS sessions,
                CAST(SUM(CASE WHEN pv_count = 1 THEN 1 ELSE 0 END) AS BIGINT) AS bounces,
                CAST(AVG(ended_s - started_s) AS DOUBLE) AS avg_duration_secs
            FROM sessions
            "#
        );
        let batches = self.run(&sql).await?;
        let Some(batch) = batches.first() else {
            return Ok((0.0, 0.0));
        };
        let sessions = i64_col(batch, "sessions")
            .map(|c| c.value(0) as u64)
            .unwrap_or(0);
        let bounces = i64_col(batch, "bounces")
            .map(|c| c.value(0) as u64)
            .unwrap_or(0);
        let avg_duration = f64_col(batch, "avg_duration_secs")
            .map(|c| {
                if c.is_valid(0) {
                    c.value(0).max(0.0)
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);
        let bounce_rate = if sessions > 0 {
            bounces as f64 / sessions as f64 * 100.0
        } else {
            0.0
        };
        Ok((bounce_rate, avg_duration))
    }

    pub async fn query_top_list(
        &self,
        site_id: Ulid,
        field: TopListField,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<TopList, StoreError> {
        let site_id_str = site_id.to_string();
        if !self.ensure_site_table_available(&site_id_str).await? {
            return Ok(TopList::default());
        }
        self.query_top_field(site_id, range, limit, field.column(), filters)
            .await
    }

    async fn query_top_field(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        field: &str,
        filters: &[Filter],
    ) -> Result<TopList, StoreError> {
        let site_id_str = site_id.to_string();
        self.ensure_site_registered(&site_id_str)
            .await
            .map_err(StoreError::db)?;

        let table = table_name(&site_id_str);
        let start = range.start.timestamp_micros();
        let end = range.end.timestamp_micros();
        let filter_sql = datafusion_filter_clause(filters);

        let sql = format!(
            r#"
            SELECT
                COALESCE(CAST({field} AS VARCHAR), 'Direct / None') AS value,
                CAST(COUNT(*) AS BIGINT) AS pageviews,
                CAST(COUNT(DISTINCT session_id) AS BIGINT) AS sessions
            FROM {table}
            WHERE site_id = '{site_id_str}'
              AND "timestamp" >= to_timestamp_micros({start})
              AND "timestamp" <= to_timestamp_micros({end})
              AND kind = 'pageview'
              {filter_sql}
            GROUP BY 1
            ORDER BY pageviews DESC
            LIMIT {limit}
            "#
        );

        let df = self.ctx.sql(&sql).await.map_err(StoreError::query)?;
        let batches = df.collect().await.map_err(StoreError::query)?;
        self.batches_to_top_list(batches)
    }

    pub async fn query_custom_events(&self, q: &EventQuery) -> Result<TopList, StoreError> {
        let site_id_str = q.site_id.to_string();
        if !self.ensure_site_table_available(&site_id_str).await? {
            return Ok(TopList::default());
        }

        let table = table_name(&site_id_str);
        let start = q.range.start.timestamp_micros();
        let end = q.range.end.timestamp_micros();

        let name_filter = q
            .event_name
            .as_deref()
            .map(|n| format!("AND name = '{}'", n.replace('\'', "''")))
            .unwrap_or_default();
        let filters = datafusion_filter_clause(&q.filters);

        let sql = format!(
            r#"
            SELECT
                CAST(name AS VARCHAR) AS value,
                CAST(COUNT(*) AS BIGINT) AS pageviews,
                CAST(COUNT(DISTINCT session_id) AS BIGINT) AS sessions
            FROM {table}
            WHERE site_id = '{site_id_str}'
              AND "timestamp" >= to_timestamp_micros({start})
              AND "timestamp" <= to_timestamp_micros({end})
              AND kind = 'custom'
              {name_filter}
              {filters}
            GROUP BY 1
            ORDER BY pageviews DESC
            LIMIT {}
            "#,
            q.limit
        );

        let df = self.ctx.sql(&sql).await.map_err(StoreError::query)?;
        let batches = df.collect().await.map_err(StoreError::query)?;
        self.batches_to_top_list(batches)
    }

    pub async fn query_funnel(&self, q: &FunnelQuery) -> Result<FunnelResult, StoreError> {
        if q.steps.is_empty() {
            return Ok(FunnelResult::default());
        }

        let site_id_str = q.site_id.to_string();
        if !self.ensure_site_table_available(&site_id_str).await? {
            return Ok(FunnelResult::default());
        }

        let table = table_name(&site_id_str);
        let start = q.range.start.timestamp_micros();
        let end = q.range.end.timestamp_micros();

        // Compute step counts sequentially.
        // For each step, count distinct session_ids that:
        //   1. Completed all prior steps
        //   2. Hit this step within window_secs of the previous step
        // Simplified approach: count sessions that hit each event name in order.
        let mut step_results: Vec<FunnelStepResult> = Vec::new();
        let mut prev_sessions: Option<u64> = None;

        for step in &q.steps {
            let sql = format!(
                r#"
                SELECT CAST(COUNT(DISTINCT session_id) AS BIGINT) AS sessions
                FROM {table}
                WHERE site_id = '{site_id_str}'
                  AND "timestamp" >= to_timestamp_micros({start})
                  AND "timestamp" <= to_timestamp_micros({end})
                  AND CAST(name AS VARCHAR) = '{}'
                "#,
                step.event_name.replace('\'', "''")
            );

            let df = self.ctx.sql(&sql).await.map_err(StoreError::query)?;
            let batches = df.collect().await.map_err(StoreError::query)?;
            let sessions = extract_count(&batches).unwrap_or(0);

            let (conversion_rate, drop_off_rate) = match prev_sessions {
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
                conversion_rate,
                drop_off_rate,
            });
            prev_sessions = Some(sessions);
        }

        Ok(FunnelResult {
            steps: step_results,
        })
    }

    // ---- Tier-2 analytics queries ----

    pub async fn query_entry_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<EntryPages, StoreError> {
        let site = site_id.to_string();
        if !self.ensure_site_table_available(&site).await? {
            return Ok(EntryPages::default());
        }
        let table = table_name(&site);
        let start = range.start.timestamp_micros();
        let end = range.end.timestamp_micros();
        let filter_sql = datafusion_filter_clause(filters);
        let sql = format!(
            r#"
            WITH ranked AS (
                SELECT
                    COALESCE(CAST(url AS VARCHAR), '') AS url,
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY "timestamp" ASC, id ASC) AS rn,
                    COUNT(*) OVER (PARTITION BY session_id) AS pv_count
                FROM {table}
                WHERE site_id = '{site}'
                  AND "timestamp" >= to_timestamp_micros({start})
                  AND "timestamp" <= to_timestamp_micros({end})
                  AND kind = 'pageview'
                  {filter_sql}
            )
            SELECT url AS value,
                   CAST(COUNT(*) AS BIGINT) AS sessions,
                   CAST(SUM(CASE WHEN pv_count = 1 THEN 1 ELSE 0 END) AS BIGINT) AS bounces
            FROM ranked WHERE rn = 1
            GROUP BY url ORDER BY sessions DESC LIMIT {limit}
            "#
        );
        let batches = self.run(&sql).await?;
        let mut rows = Vec::new();
        let mut total: u64 = 0;
        for b in &batches {
            let urls = str_col(b, "value");
            let sess = i64_col(b, "sessions");
            let bounce = i64_col(b, "bounces");
            if let (Some(urls), Some(sess), Some(bounce)) = (urls, sess, bounce) {
                for i in 0..b.num_rows() {
                    let sessions = sess.value(i) as u64;
                    total += sessions;
                    let bounces = bounce.value(i) as u64;
                    let bounce_rate = if sessions > 0 {
                        bounces as f64 / sessions as f64 * 100.0
                    } else {
                        0.0
                    };
                    rows.push(EntryPageRow {
                        url: urls.value(i).to_string(),
                        sessions,
                        pct: 0.0,
                        bounce_rate,
                    });
                }
            }
        }
        if total > 0 {
            for r in &mut rows {
                r.pct = r.sessions as f64 / total as f64 * 100.0;
            }
        }
        Ok(EntryPages { rows })
    }

    pub async fn query_exit_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<ExitPages, StoreError> {
        let site = site_id.to_string();
        if !self.ensure_site_table_available(&site).await? {
            return Ok(ExitPages::default());
        }
        let table = table_name(&site);
        let start = range.start.timestamp_micros();
        let end = range.end.timestamp_micros();
        let filter_sql = datafusion_filter_clause(filters);
        let sql = format!(
            r#"
            WITH ranked AS (
                SELECT
                    COALESCE(CAST(url AS VARCHAR), '') AS url,
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY "timestamp" DESC, id DESC) AS rn
                FROM {table}
                WHERE site_id = '{site}'
                  AND "timestamp" >= to_timestamp_micros({start})
                  AND "timestamp" <= to_timestamp_micros({end})
                  AND kind = 'pageview'
                  {filter_sql}
            )
            SELECT url AS value,
                   CAST(SUM(CASE WHEN rn = 1 THEN 1 ELSE 0 END) AS BIGINT) AS exits,
                   CAST(COUNT(*) AS BIGINT) AS pageviews
            FROM ranked
            GROUP BY url ORDER BY exits DESC LIMIT {limit}
            "#
        );
        let batches = self.run(&sql).await?;
        let mut rows = Vec::new();
        let mut total: u64 = 0;
        for b in &batches {
            let urls = str_col(b, "value");
            let exits = i64_col(b, "exits");
            let pvs = i64_col(b, "pageviews");
            if let (Some(urls), Some(exits), Some(pvs)) = (urls, exits, pvs) {
                for i in 0..b.num_rows() {
                    let e = exits.value(i) as u64;
                    let pv = pvs.value(i) as u64;
                    total += e;
                    let exit_rate = if pv > 0 {
                        e as f64 / pv as f64 * 100.0
                    } else {
                        0.0
                    };
                    rows.push(ExitPageRow {
                        url: urls.value(i).to_string(),
                        exits: e,
                        pct: 0.0,
                        exit_rate,
                    });
                }
            }
        }
        // Drop zero-exit rows (a page can appear in the CTE with exits=0).
        rows.retain(|r| r.exits > 0);
        if total > 0 {
            for r in &mut rows {
                r.pct = r.exits as f64 / total as f64 * 100.0;
            }
        }
        Ok(ExitPages { rows })
    }

    pub async fn query_sessions(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<SessionRow>, StoreError> {
        let site = site_id.to_string();
        if !self.ensure_site_table_available(&site).await? {
            return Ok(vec![]);
        }
        let table = table_name(&site);
        let start = range.start.timestamp_micros();
        let end = range.end.timestamp_micros();
        let sql = format!(
            r#"
            WITH s AS (
                SELECT session_id,
                    "timestamp" AS ts,
                    COALESCE(CAST(url AS VARCHAR), '') AS url,
                    CAST(referrer AS VARCHAR) AS referrer,
                    CAST(country_code AS VARCHAR) AS country,
                    COALESCE(CAST(browser AS VARCHAR), '') AS browser,
                    COALESCE(CAST(os AS VARCHAR), '') AS os,
                    COALESCE(CAST(device_type AS VARCHAR), '') AS device,
                    CAST(utm_source AS VARCHAR) AS utm_source,
                    CAST(utm_medium AS VARCHAR) AS utm_medium,
                    CAST(utm_campaign AS VARCHAR) AS utm_campaign,
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY "timestamp" ASC, id ASC) AS rn_first,
                    ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY "timestamp" DESC, id DESC) AS rn_last
                FROM {table}
                WHERE site_id = '{site}'
                  AND "timestamp" >= to_timestamp_micros({start})
                  AND "timestamp" <= to_timestamp_micros({end})
                  AND kind = 'pageview'
            )
            SELECT session_id,
                   MIN(ts) AS started,
                   MAX(ts) AS ended,
                   CAST(COUNT(*) AS BIGINT) AS pageviews,
                   MAX(CASE WHEN rn_first = 1 THEN url END) AS entry_url,
                   MAX(CASE WHEN rn_last = 1 THEN url END) AS exit_url,
                   MAX(CASE WHEN rn_first = 1 THEN referrer END) AS referrer,
                   MAX(CASE WHEN rn_first = 1 THEN country END) AS country,
                   MAX(CASE WHEN rn_first = 1 THEN browser END) AS browser,
                   MAX(CASE WHEN rn_first = 1 THEN os END) AS os,
                   MAX(CASE WHEN rn_first = 1 THEN device END) AS device,
                   MAX(CASE WHEN rn_first = 1 THEN utm_source END) AS utm_source,
                   MAX(CASE WHEN rn_first = 1 THEN utm_medium END) AS utm_medium,
                   MAX(CASE WHEN rn_first = 1 THEN utm_campaign END) AS utm_campaign
            FROM s
            GROUP BY session_id
            ORDER BY started DESC
            LIMIT {limit}
            "#
        );
        let batches = self.run(&sql).await?;
        let mut out = Vec::new();
        for b in &batches {
            let sid = fixedbin_col(b, "session_id");
            let started = ts_micros_col(b, "started");
            let ended = ts_micros_col(b, "ended");
            let pvs = i64_col(b, "pageviews");
            let entry = str_col(b, "entry_url");
            let exit = str_col(b, "exit_url");
            let referrer = str_col(b, "referrer");
            let country = str_col(b, "country");
            let browser = str_col(b, "browser");
            let os = str_col(b, "os");
            let device = str_col(b, "device");
            let utm_source = str_col(b, "utm_source");
            let utm_medium = str_col(b, "utm_medium");
            let utm_campaign = str_col(b, "utm_campaign");
            for i in 0..b.num_rows() {
                let started_at = started
                    .and_then(|c| chrono::DateTime::from_timestamp_micros(c.value(i)))
                    .unwrap_or_default();
                let ended_at = ended
                    .and_then(|c| chrono::DateTime::from_timestamp_micros(c.value(i)))
                    .unwrap_or_default();
                let pageviews = pvs.map(|c| c.value(i) as u64).unwrap_or(0);
                let duration_secs = (ended_at - started_at).num_seconds();
                let is_bounce = pageviews == 1 && duration_secs < 30;
                out.push(SessionRow {
                    session_id: sid.map(|c| hex_encode(c.value(i))).unwrap_or_default(),
                    started_at,
                    ended_at,
                    duration_secs,
                    pageviews,
                    entry_url: opt_str(entry, i).unwrap_or_default(),
                    exit_url: opt_str(exit, i).unwrap_or_default(),
                    referrer: opt_str(referrer, i),
                    country_code: opt_str(country, i),
                    browser: opt_str(browser, i).unwrap_or_default(),
                    os: opt_str(os, i).unwrap_or_default(),
                    device_type: opt_str(device, i).unwrap_or_default(),
                    utm_source: opt_str(utm_source, i),
                    utm_medium: opt_str(utm_medium, i),
                    utm_campaign: opt_str(utm_campaign, i),
                    is_bounce,
                });
            }
        }
        Ok(out)
    }

    pub async fn query_events_list(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<Vec<RawEventRow>, StoreError> {
        let site = site_id.to_string();
        if !self.ensure_site_table_available(&site).await? {
            return Ok(vec![]);
        }
        let table = table_name(&site);
        let start = range.start.timestamp_micros();
        let end = range.end.timestamp_micros();
        let sql = format!(
            r#"
            SELECT CAST(id AS VARCHAR) AS id,
                   CAST(name AS VARCHAR) AS name,
                   CAST(kind AS VARCHAR) AS kind,
                   "timestamp" AS ts,
                   COALESCE(CAST(url AS VARCHAR), '') AS url,
                   CAST(referrer AS VARCHAR) AS referrer,
                   CAST(country_code AS VARCHAR) AS country,
                   COALESCE(CAST(browser AS VARCHAR), '') AS browser,
                   COALESCE(CAST(os AS VARCHAR), '') AS os,
                   COALESCE(CAST(device_type AS VARCHAR), '') AS device,
                   CAST(properties AS VARCHAR) AS props
            FROM {table}
            WHERE site_id = '{site}'
              AND "timestamp" >= to_timestamp_micros({start})
              AND "timestamp" <= to_timestamp_micros({end})
            ORDER BY "timestamp" DESC LIMIT {limit}
            "#
        );
        let batches = self.run(&sql).await?;
        let mut out = Vec::new();
        for b in &batches {
            let id = str_col(b, "id");
            let name = str_col(b, "name");
            let kind = str_col(b, "kind");
            let ts = ts_micros_col(b, "ts");
            let url = str_col(b, "url");
            let referrer = str_col(b, "referrer");
            let country = str_col(b, "country");
            let browser = str_col(b, "browser");
            let os = str_col(b, "os");
            let device = str_col(b, "device");
            let props = str_col(b, "props");
            for i in 0..b.num_rows() {
                out.push(RawEventRow {
                    id: opt_str(id, i).unwrap_or_default(),
                    name: opt_str(name, i).unwrap_or_default(),
                    kind: opt_str(kind, i).unwrap_or_default(),
                    timestamp: ts
                        .and_then(|c| chrono::DateTime::from_timestamp_micros(c.value(i)))
                        .unwrap_or_default(),
                    url: opt_str(url, i).unwrap_or_default(),
                    referrer: opt_str(referrer, i),
                    country_code: opt_str(country, i),
                    browser: opt_str(browser, i).unwrap_or_default(),
                    os: opt_str(os, i).unwrap_or_default(),
                    device_type: opt_str(device, i).unwrap_or_default(),
                    properties: opt_str(props, i),
                });
            }
        }
        Ok(out)
    }

    // ---- Tier-3 analytics queries ----

    pub async fn query_top_sparklines(
        &self,
        site_id: Ulid,
        field: TopListField,
        range: &TimeRange,
        limit: u32,
        filters: &[Filter],
    ) -> Result<TopSparklines, StoreError> {
        let site = site_id.to_string();
        if !self.ensure_site_table_available(&site).await? {
            return Ok(TopSparklines::default());
        }
        let table = table_name(&site);
        let start = range.start.timestamp_micros();
        let end = range.end.timestamp_micros();
        let col = field.column();
        let filter_sql = datafusion_filter_clause(filters);
        let sql = format!(
            r#"
            SELECT COALESCE(CAST({col} AS VARCHAR), 'Direct / None') AS value,
                   date_trunc('day', "timestamp") AS bucket,
                   CAST(COUNT(*) AS BIGINT) AS c
            FROM {table}
            WHERE site_id = '{site}'
              AND "timestamp" >= to_timestamp_micros({start})
              AND "timestamp" <= to_timestamp_micros({end})
              AND kind = 'pageview'
              {filter_sql}
            GROUP BY 1, 2
            "#
        );
        let batches = self.run(&sql).await?;
        let mut rows = Vec::new();
        let mut days = std::collections::BTreeSet::new();
        for b in &batches {
            let vals = str_col(b, "value");
            let buckets = b
                .column_by_name("bucket")
                .and_then(|c| c.as_any().downcast_ref::<TimestampNanosecondArray>());
            let counts = i64_col(b, "c");
            if let (Some(vals), Some(buckets), Some(counts)) = (vals, buckets, counts) {
                for i in 0..b.num_rows() {
                    let day = chrono::DateTime::from_timestamp_nanos(buckets.value(i))
                        .format("%Y-%m-%d")
                        .to_string();
                    days.insert(day.clone());
                    rows.push((
                        vals.value(i).to_string(),
                        day,
                        counts.value(i).max(0) as u64,
                    ));
                }
            }
        }
        Ok(TopSparklines::from_counts(
            rows,
            days.into_iter().collect(),
            limit as usize,
        ))
    }

    /// Run a SQL string and collect the result batches.
    async fn run(&self, sql: &str) -> Result<Vec<arrow::record_batch::RecordBatch>, StoreError> {
        let df = self.ctx.sql(sql).await.map_err(StoreError::query)?;
        df.collect().await.map_err(StoreError::query)
    }

    fn batches_to_top_list(
        &self,
        batches: Vec<arrow::record_batch::RecordBatch>,
    ) -> Result<TopList, StoreError> {
        let mut rows = Vec::new();
        let mut total_pv = 0u64;

        for batch in &batches {
            use arrow::array::{Int64Array, StringArray};
            let val_col = batch
                .column_by_name("value")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let pv_col = batch
                .column_by_name("pageviews")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
            let sess_col = batch
                .column_by_name("sessions")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());

            if let (Some(vals), Some(pvs), Some(sessions)) = (val_col, pv_col, sess_col) {
                for i in 0..batch.num_rows() {
                    let pv = pvs.value(i) as u64;
                    total_pv += pv;
                    rows.push(TopRow {
                        value: vals.value(i).to_string(),
                        pageviews: pv,
                        sessions: sessions.value(i) as u64,
                        pct: 0.0,
                    });
                }
            }
        }

        if total_pv > 0 {
            for row in &mut rows {
                row.pct = (row.pageviews as f64 / total_pv as f64) * 100.0;
            }
        }

        Ok(TopList { rows })
    }
}

fn table_name(site_id: &str) -> String {
    format!("events_{}", site_id.replace('-', "_"))
}

// ---- Arrow column extraction helpers ----

use arrow::array::{
    Array, FixedSizeBinaryArray, Int64Array, StringArray, TimestampMicrosecondArray,
    TimestampNanosecondArray,
};
use arrow::record_batch::RecordBatch;

/// Lowercase hex encoding without an extra hex dependency.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn str_col<'a>(b: &'a RecordBatch, name: &str) -> Option<&'a StringArray> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
}

fn i64_col<'a>(b: &'a RecordBatch, name: &str) -> Option<&'a Int64Array> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
}

fn f64_col<'a>(b: &'a RecordBatch, name: &str) -> Option<&'a arrow::array::Float64Array> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<arrow::array::Float64Array>())
}

fn fixedbin_col<'a>(b: &'a RecordBatch, name: &str) -> Option<&'a FixedSizeBinaryArray> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<FixedSizeBinaryArray>())
}

/// Read a microsecond-precision timestamp column. The raw `timestamp`
/// column is `Timestamp(Microsecond, UTC)`; `MIN`/`MAX` preserve that.
fn ts_micros_col<'a>(b: &'a RecordBatch, name: &str) -> Option<&'a TimestampMicrosecondArray> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<TimestampMicrosecondArray>())
}

/// Read a non-null string at row `i`, returning None when the column is
/// absent or the cell is SQL NULL.
fn opt_str(col: Option<&StringArray>, i: usize) -> Option<String> {
    col.filter(|c| c.is_valid(i))
        .map(|c| c.value(i).to_string())
}

/// Build the `AND <col> <op> '<value>'` fragment for a set of analytics
/// filters, escaped for DataFusion's SQL dialect. Columns come from the
/// `FilterField` enum (never user input); values are single-quote-escaped.
///
/// DataFusion's `LIKE` has no `ESCAPE` support, so `Contains`/`StartsWith`
/// build the wildcard pattern from the raw value here rather than reusing
/// `Filter::sql_value` (whose backslash escapes would be taken literally).
/// The trade-off: a `%`/`_` inside a contains/starts-with value acts as a
/// wildcard on this backend.
fn datafusion_filter_clause(filters: &[Filter]) -> String {
    let mut out = String::new();
    for f in filters {
        let col = f.field.column();
        match f.op {
            FilterOp::Contains => {
                let val = f.value.replace('\'', "''");
                out.push_str(&format!(" AND CAST({col} AS VARCHAR) LIKE '%{val}%'"));
            }
            FilterOp::StartsWith => {
                let val = f.value.replace('\'', "''");
                out.push_str(&format!(" AND CAST({col} AS VARCHAR) LIKE '{val}%'"));
            }
            _ => {
                let val = f.value.replace('\'', "''");
                out.push_str(&format!(
                    " AND CAST({col} AS VARCHAR) {} '{val}'",
                    f.op.sql_operator()
                ));
            }
        }
    }
    out
}

/// Rename any pre-existing flat `<date>/` partitions under a site directory
/// to Hive-style `date=<date>/` partitions. Earlier versions wrote the flat
/// layout, but DataFusion's `ListingTable` defaults to
/// `listing_table_ignore_subdirectory=true` and so skipped those files at
/// query time. Idempotent — directories already named `date=...` are left
/// alone, and a target collision falls back to merging files into the
/// existing Hive directory.
async fn migrate_flat_date_dirs(site_dir: &std::path::Path) -> Result<()> {
    let mut entries = tokio::fs::read_dir(site_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str.contains('=') {
            continue;
        }
        if !looks_like_iso_date(name_str) {
            continue;
        }
        let from = entry.path();
        let to = site_dir.join(format!("date={name_str}"));
        if tokio::fs::metadata(&to).await.is_ok() {
            // Target exists: move individual files in and drop the empty source.
            let mut files = tokio::fs::read_dir(&from).await?;
            while let Some(f) = files.next_entry().await? {
                let dest = to.join(f.file_name());
                if let Err(e) = tokio::fs::rename(f.path(), &dest).await {
                    tracing::warn!("Could not merge {:?} into {:?}: {e}", f.path(), dest);
                }
            }
            let _ = tokio::fs::remove_dir(&from).await;
        } else {
            tokio::fs::rename(&from, &to).await?;
        }
        info!(
            "Migrated flat date partition {:?} → {:?}",
            from.file_name().unwrap_or_default(),
            to.file_name().unwrap_or_default()
        );
    }
    Ok(())
}

fn looks_like_iso_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[0..4].iter().all(|b| b.is_ascii_digit())
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
}

fn granularity_trunc(g: &Granularity) -> &'static str {
    match g {
        Granularity::Hour => "hour",
        Granularity::Day => "day",
        Granularity::Week => "week",
        Granularity::Month => "month",
    }
}

fn extract_count(batches: &[arrow::record_batch::RecordBatch]) -> Option<u64> {
    use arrow::array::Int64Array;
    batches.first().and_then(|b| {
        b.column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .map(|a| a.value(0) as u64)
    })
}

#[cfg(test)]
mod tests {
    use super::EmbeddedReader;
    use chrono::Utc;
    use stomatopod_core::query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelStep},
        pageviews::{Granularity, PageviewsQuery, TimeRange, TopListField},
    };
    use tempfile::tempdir;
    use ulid::Ulid;

    #[test]
    fn empty_site_queries_return_defaults_without_table_errors() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        rt.block_on(async {
            let temp = tempdir().expect("tempdir");
            let reader = EmbeddedReader::new(temp.path().to_path_buf())
                .await
                .expect("reader init");
            let site_id = Ulid::new();
            let range = TimeRange {
                start: Utc::now() - chrono::Duration::days(7),
                end: Utc::now(),
            };

            let pageviews = reader
                .query_pageviews(&PageviewsQuery {
                    site_id,
                    range: range.clone(),
                    granularity: Granularity::Day,
                    filters: vec![],
                })
                .await
                .expect("pageviews query");
            assert_eq!(pageviews.total_pageviews, 0);
            assert_eq!(pageviews.total_sessions, 0);
            assert!(pageviews.buckets.is_empty());

            let top = reader
                .query_top_list(site_id, TopListField::Page, &range, 10, &[])
                .await
                .expect("top list query");
            assert!(top.rows.is_empty());

            let events = reader
                .query_custom_events(&EventQuery {
                    site_id,
                    range: range.clone(),
                    event_name: None,
                    filters: vec![],
                    limit: 10,
                })
                .await
                .expect("custom events query");
            assert!(events.rows.is_empty());

            let funnel = reader
                .query_funnel(&FunnelQuery {
                    site_id,
                    range,
                    steps: vec![FunnelStep {
                        name: "Signup".to_string(),
                        event_name: "signup".to_string(),
                        filters: vec![],
                    }],
                    window_secs: 86_400,
                })
                .await
                .expect("funnel query");
            assert!(funnel.steps.is_empty());
        });
    }
}
