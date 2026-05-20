use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use datafusion::{
    datasource::listing::{ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl},
    execution::context::SessionContext,
};
use parking_lot::RwLock;
use std::collections::HashSet;
use tracing::info;
use ulid::Ulid;

use stomatopod_core::{
    error::StoreError,
    query::{
        events::EventQuery,
        funnel::{FunnelQuery, FunnelResult, FunnelStepResult},
        pageviews::{
            Granularity, PageviewsQuery, PageviewsResult, TimeBucket, TimeRange, TopList, TopRow,
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
        let ctx = SessionContext::new();

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

    async fn register_site(&self, site_id: &str) -> Result<()> {
        let site_dir = self.data_dir.join(site_id);
        if !site_dir.exists() {
            return Ok(());
        }

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
        self.ensure_site_registered(&site_id)
            .await
            .map_err(StoreError::db)?;

        let table = table_name(&site_id);
        let granularity_fn = granularity_trunc(&q.granularity);
        let start = q.range.start.timestamp_micros();
        let end = q.range.end.timestamp_micros();

        // `timestamp` is a reserved keyword in DataFusion's SQL parser (it
        // expects `timestamp '2024-...'` literal syntax after it); quote the
        // column to force identifier parsing. We also avoid the `FILTER
        // (WHERE ...)` aggregate clause, which the parser rejects — `CASE
        // WHEN ... THEN 1 END` is equivalent.
        //
        // Each aggregate is wrapped in `CAST(... AS BIGINT)` so the read
        // side can rely on a single `Int64Array` downcast — DataFusion
        // promotes integer aggregates to wider types and the column types
        // would otherwise differ between SUM and COUNT.
        let sql = format!(
            r#"
            SELECT
                date_trunc('{granularity_fn}', "timestamp") AS bucket,
                CAST(SUM(CASE WHEN CAST(kind AS VARCHAR) = 'pageview' THEN 1 ELSE 0 END) AS BIGINT) AS pageviews,
                CAST(COUNT(DISTINCT session_id) AS BIGINT) AS sessions
            FROM {table}
            WHERE site_id = '{site_id}'
              AND "timestamp" >= to_timestamp_micros({start})
              AND "timestamp" <= to_timestamp_micros({end})
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
                    let ts = chrono::DateTime::from_timestamp_nanos(ts_ns)
                        .with_timezone(&chrono::Utc);
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

        Ok(result)
    }

    pub async fn query_top_pages(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "url").await
    }

    pub async fn query_top_referrers(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "referrer")
            .await
    }

    pub async fn query_top_countries(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "country_code")
            .await
    }

    pub async fn query_top_browsers(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "browser").await
    }

    pub async fn query_top_devices(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
    ) -> Result<TopList, StoreError> {
        self.query_top_field(site_id, range, limit, "device_type")
            .await
    }

    async fn query_top_field(
        &self,
        site_id: Ulid,
        range: &TimeRange,
        limit: u32,
        field: &str,
    ) -> Result<TopList, StoreError> {
        let site_id_str = site_id.to_string();
        self.ensure_site_registered(&site_id_str)
            .await
            .map_err(StoreError::db)?;

        let table = table_name(&site_id_str);
        let start = range.start.timestamp_micros();
        let end = range.end.timestamp_micros();

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
              AND CAST(kind AS VARCHAR) = 'pageview'
            GROUP BY 1
            ORDER BY pageviews DESC
            LIMIT {limit}
            "#
        );

        let df = self.ctx.sql(&sql).await.map_err(StoreError::query)?;
        let batches = df.collect().await.map_err(StoreError::query)?;

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

        // Compute percentages
        if total_pv > 0 {
            for row in &mut rows {
                row.pct = (row.pageviews as f64 / total_pv as f64) * 100.0;
            }
        }

        Ok(TopList { rows })
    }

    pub async fn query_custom_events(&self, q: &EventQuery) -> Result<TopList, StoreError> {
        let site_id_str = q.site_id.to_string();
        self.ensure_site_registered(&site_id_str)
            .await
            .map_err(StoreError::db)?;

        let table = table_name(&site_id_str);
        let start = q.range.start.timestamp_micros();
        let end = q.range.end.timestamp_micros();

        let name_filter = q
            .event_name
            .as_deref()
            .map(|n| format!("AND name = '{}'", n.replace('\'', "''")))
            .unwrap_or_default();

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
              AND CAST(kind AS VARCHAR) = 'custom'
              {name_filter}
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
        self.ensure_site_registered(&site_id_str)
            .await
            .map_err(StoreError::db)?;

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
