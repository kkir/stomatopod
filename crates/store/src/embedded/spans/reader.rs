use std::{collections::HashSet, path::PathBuf, sync::Arc};

use anyhow::Result;
use datafusion::{
    datasource::listing::{ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl},
    execution::context::SessionContext,
};
use parking_lot::RwLock;
use tracing::info;

use stomatopod_core::{
    error::StoreError,
    query::spans::{AgentSummary, SpanQuery, SpanRow},
};

pub struct SpanReader {
    ctx: SessionContext,
    data_dir: PathBuf,
    registered: RwLock<HashSet<String>>,
}

impl SpanReader {
    pub async fn new(data_dir: PathBuf) -> Result<Self> {
        let ctx = SessionContext::new();
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
                    if let Err(e) = reader.register_site(&site_id).await {
                        tracing::warn!("Could not register spans for site {site_id}: {e}");
                    }
                }
            }
        }

        Ok(reader)
    }

    pub async fn ensure_site_registered(&self, site_id: &str) -> Result<()> {
        if self.registered.read().contains(site_id) {
            return Ok(());
        }
        self.register_site(site_id).await
    }

    async fn ensure_site_table_available(&self, site_id: &str) -> Result<bool, StoreError> {
        self.ensure_site_registered(site_id)
            .await
            .map_err(StoreError::db)?;
        Ok(self.registered.read().contains(site_id))
    }

    async fn register_site(&self, site_id: &str) -> Result<()> {
        let site_dir = self.data_dir.join(site_id);
        if !site_dir.exists() {
            return Ok(());
        }
        let site_dir = site_dir.canonicalize()?;

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
        info!("Registered DataFusion span table: {table_name}");
        Ok(())
    }

    pub async fn query_spans(&self, q: &SpanQuery) -> Result<Vec<SpanRow>, StoreError> {
        let site_id_str = q.site_id.to_string();
        if !self.ensure_site_table_available(&site_id_str).await? {
            return Ok(Vec::new());
        }

        let table = table_name(&site_id_str);
        let start = q.since.timestamp_micros();
        let end = q.until.timestamp_micros();
        let limit = q.limit;

        let agent_filter = q
            .agent_id
            .as_deref()
            .map(|a| format!("AND agent_id = '{}'", a.replace('\'', "''")))
            .unwrap_or_default();
        let session_filter = q
            .session_id
            .as_deref()
            .map(|s| format!("AND agent_session_id = '{}'", s.replace('\'', "''")))
            .unwrap_or_default();

        // Cast Dict/Utf8View columns to plain VARCHAR so the read side can
        // rely on `StringArray` downcasts. DataFusion 43 silently promotes
        // parquet `Utf8` to `Utf8View` and keeps Dict columns as Dict;
        // matching every downcast site would be brittle.
        let sql = format!(
            r#"
            SELECT
                CAST(id AS VARCHAR) AS id,
                CAST(agent_id AS VARCHAR) AS agent_id,
                CAST(agent_session_id AS VARCHAR) AS agent_session_id,
                CAST(kind AS VARCHAR) AS kind,
                CAST(model AS VARCHAR) AS model,
                started_at, ended_at,
                input_tokens, output_tokens, cost_usd,
                CAST(tool_name AS VARCHAR) AS tool_name,
                CAST(stop_reason AS VARCHAR) AS stop_reason
            FROM {table}
            WHERE site_id = '{site_id_str}'
              AND started_at >= to_timestamp_micros({start})
              AND started_at <= to_timestamp_micros({end})
              {agent_filter}
              {session_filter}
            ORDER BY started_at DESC
            LIMIT {limit}
            "#
        );

        let df = self.ctx.sql(&sql).await.map_err(StoreError::query)?;
        let batches = df.collect().await.map_err(StoreError::query)?;
        Ok(rows_from_batches(&batches))
    }

    pub async fn summarize_agents(
        &self,
        site_id: ulid::Ulid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<AgentSummary>, StoreError> {
        let site_id_str = site_id.to_string();
        if !self.ensure_site_table_available(&site_id_str).await? {
            return Ok(Vec::new());
        }

        let table = table_name(&site_id_str);
        let start = since.timestamp_micros();

        let sql = format!(
            r#"
            SELECT
                CAST(agent_id AS VARCHAR) AS agent_id,
                MAX(started_at) AS last_seen,
                CAST(COUNT(*) AS BIGINT) AS total_spans,
                CAST(SUM(input_tokens) AS BIGINT) AS in_tok,
                CAST(SUM(output_tokens) AS BIGINT) AS out_tok,
                CAST(SUM(cost_usd) AS DOUBLE) AS cost
            FROM {table}
            WHERE site_id = '{site_id_str}'
              AND started_at >= to_timestamp_micros({start})
            GROUP BY agent_id
            ORDER BY last_seen DESC
            "#
        );

        let df = self.ctx.sql(&sql).await.map_err(StoreError::query)?;
        let batches = df.collect().await.map_err(StoreError::query)?;

        let mut out = Vec::new();
        for batch in &batches {
            use arrow::array::{Float64Array, Int64Array, StringArray, TimestampMicrosecondArray};
            let agent_col = batch
                .column_by_name("agent_id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>());
            let last_col = batch
                .column_by_name("last_seen")
                .and_then(|c| c.as_any().downcast_ref::<TimestampMicrosecondArray>());
            let spans_col = batch
                .column_by_name("total_spans")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
            let in_col = batch
                .column_by_name("in_tok")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
            let out_col = batch
                .column_by_name("out_tok")
                .and_then(|c| c.as_any().downcast_ref::<Int64Array>());
            let cost_col = batch
                .column_by_name("cost")
                .and_then(|c| c.as_any().downcast_ref::<Float64Array>());

            if let (Some(a), Some(l), Some(s), Some(i), Some(o), Some(c)) =
                (agent_col, last_col, spans_col, in_col, out_col, cost_col)
            {
                for row in 0..batch.num_rows() {
                    out.push(AgentSummary {
                        agent_id: a.value(row).to_string(),
                        last_seen_at: chrono::DateTime::from_timestamp_micros(l.value(row))
                            .unwrap_or_default()
                            .with_timezone(&chrono::Utc),
                        total_spans: s.value(row) as u64,
                        total_input_tokens: i.value(row) as u64,
                        total_output_tokens: o.value(row) as u64,
                        total_cost_usd: c.value(row),
                    });
                }
            }
        }

        Ok(out)
    }

    pub async fn session_cost_usd(
        &self,
        site_id: ulid::Ulid,
        agent_session_id: &str,
    ) -> Result<f64, StoreError> {
        let site_id_str = site_id.to_string();
        if !self.ensure_site_table_available(&site_id_str).await? {
            return Ok(0.0);
        }

        let table = table_name(&site_id_str);
        let sql = format!(
            r#"
            SELECT COALESCE(SUM(cost_usd), 0.0) AS total
            FROM {table}
            WHERE site_id = '{site_id_str}'
              AND agent_session_id = '{}'
            "#,
            agent_session_id.replace('\'', "''")
        );

        let df = self.ctx.sql(&sql).await.map_err(StoreError::query)?;
        let batches = df.collect().await.map_err(StoreError::query)?;
        for batch in &batches {
            use arrow::array::Float64Array;
            if let Some(col) = batch.column(0).as_any().downcast_ref::<Float64Array>() {
                if !col.is_empty() {
                    return Ok(col.value(0));
                }
            }
        }
        Ok(0.0)
    }
}

fn rows_from_batches(batches: &[arrow::record_batch::RecordBatch]) -> Vec<SpanRow> {
    use arrow::array::{Array, Float64Array, StringArray, TimestampMicrosecondArray, UInt32Array};
    let mut out = Vec::new();
    for batch in batches {
        let id_col = batch
            .column_by_name("id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
        let agent_col = batch
            .column_by_name("agent_id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
        let sess_col = batch
            .column_by_name("agent_session_id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
        let kind_col = batch
            .column_by_name("kind")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
        let model_col = batch
            .column_by_name("model")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
        let started_col = batch
            .column_by_name("started_at")
            .and_then(|c| c.as_any().downcast_ref::<TimestampMicrosecondArray>());
        let ended_col = batch
            .column_by_name("ended_at")
            .and_then(|c| c.as_any().downcast_ref::<TimestampMicrosecondArray>());
        let in_col = batch
            .column_by_name("input_tokens")
            .and_then(|c| c.as_any().downcast_ref::<UInt32Array>());
        let out_col = batch
            .column_by_name("output_tokens")
            .and_then(|c| c.as_any().downcast_ref::<UInt32Array>());
        let cost_col = batch
            .column_by_name("cost_usd")
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>());
        let tool_col = batch
            .column_by_name("tool_name")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());
        let stop_col = batch
            .column_by_name("stop_reason")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());

        if let (
            Some(id),
            Some(agent),
            Some(sess),
            Some(kind),
            Some(model),
            Some(st),
            Some(en),
            Some(it),
            Some(ot),
            Some(co),
        ) = (
            id_col,
            agent_col,
            sess_col,
            kind_col,
            model_col,
            started_col,
            ended_col,
            in_col,
            out_col,
            cost_col,
        ) {
            for row in 0..batch.num_rows() {
                let tool_name = tool_col.and_then(|t| {
                    if t.is_null(row) {
                        None
                    } else {
                        Some(t.value(row).to_string())
                    }
                });
                let stop_reason = stop_col.and_then(|t| {
                    if t.is_null(row) {
                        None
                    } else {
                        Some(t.value(row).to_string())
                    }
                });
                out.push(SpanRow {
                    id: id.value(row).to_string(),
                    agent_id: agent.value(row).to_string(),
                    agent_session_id: sess.value(row).to_string(),
                    kind: kind.value(row).to_string(),
                    model: model.value(row).to_string(),
                    started_at: chrono::DateTime::from_timestamp_micros(st.value(row))
                        .unwrap_or_default()
                        .with_timezone(&chrono::Utc),
                    ended_at: chrono::DateTime::from_timestamp_micros(en.value(row))
                        .unwrap_or_default()
                        .with_timezone(&chrono::Utc),
                    input_tokens: it.value(row),
                    output_tokens: ot.value(row),
                    cost_usd: co.value(row),
                    tool_name,
                    stop_reason,
                });
            }
        }
    }
    out
}

fn table_name(site_id: &str) -> String {
    format!("spans_{}", site_id.replace('-', "_"))
}

#[cfg(test)]
mod tests {
    use super::SpanReader;
    use chrono::Utc;
    use stomatopod_core::query::spans::SpanQuery;
    use tempfile::tempdir;
    use ulid::Ulid;

    #[test]
    fn empty_site_span_queries_return_defaults_without_table_errors() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        rt.block_on(async {
            let temp = tempdir().expect("tempdir");
            let reader = SpanReader::new(temp.path().to_path_buf())
                .await
                .expect("reader init");
            let site_id = Ulid::new();
            let since = Utc::now() - chrono::Duration::days(1);

            let spans = reader
                .query_spans(&SpanQuery {
                    site_id,
                    agent_id: None,
                    session_id: None,
                    since,
                    until: Utc::now(),
                    limit: 100,
                })
                .await
                .expect("query spans");
            assert!(spans.is_empty());

            let summary = reader
                .summarize_agents(site_id, since)
                .await
                .expect("summarize agents");
            assert!(summary.is_empty());

            let session_cost = reader
                .session_cost_usd(site_id, "session-1")
                .await
                .expect("session cost");
            assert_eq!(session_cost, 0.0);
        });
    }
}
