use std::{path::PathBuf, sync::Arc, time::Duration};

use arrow::{
    array::{
        Float64Builder, StringBuilder, StringDictionaryBuilder, TimestampMicrosecondBuilder,
        UInt32Builder,
    },
    datatypes::Int32Type,
    record_batch::RecordBatch,
};
use parquet::{
    arrow::ArrowWriter,
    basic::Compression,
    file::properties::{WriterProperties, WriterVersion},
};
use tokio::sync::mpsc;
use tracing::{error, info};
use ulid::Ulid;

use stomatopod_core::domain::agent_span::AgentSpan;

use super::{buffer::SpanBuffer, reader::SpanReader, schema::agent_span_schema, wal::SpanWal};

pub struct SpanParquetWriter {
    data_dir: PathBuf,
    flush_rows: usize,
    flush_interval_s: u64,
}

impl SpanParquetWriter {
    pub fn new(data_dir: PathBuf, flush_rows: usize, flush_interval_s: u64) -> Self {
        Self {
            data_dir,
            flush_rows,
            flush_interval_s,
        }
    }

    pub async fn run(
        self,
        mut rx: mpsc::Receiver<Vec<AgentSpan>>,
        buffer: Arc<SpanBuffer>,
        wal: Arc<SpanWal>,
        reader: Arc<SpanReader>,
    ) {
        let mut interval = tokio::time::interval(Duration::from_secs(self.flush_interval_s));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                Some(spans) = rx.recv() => {
                    if let Err(e) = wal.append(&spans) {
                        error!("Span WAL append error: {e}");
                    }
                    buffer.push_batch(spans);

                    if buffer.is_above_threshold() {
                        self.flush_buffer(&buffer, &reader).await;
                    }
                }
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        self.flush_buffer(&buffer, &reader).await;
                    }
                }
                else => break,
            }
        }
    }

    async fn flush_buffer(&self, buffer: &SpanBuffer, reader: &SpanReader) {
        let spans = buffer.drain(self.flush_rows);
        if spans.is_empty() {
            return;
        }

        // Partition by (site_id, date) — NOT by agent_id. Agent cardinality
        // can explode (e.g. one agent per CI run); top-level partitioning by
        // agent_id would create a small-files problem.
        let mut by_partition: std::collections::HashMap<(String, String), Vec<AgentSpan>> =
            std::collections::HashMap::new();
        for span in spans {
            let date = span.started_at.format("%Y-%m-%d").to_string();
            let key = (span.site_id.to_string(), date);
            by_partition.entry(key).or_default().push(span);
        }

        for ((site_id, date), partition_spans) in by_partition {
            if let Err(e) = self
                .write_partition(&site_id, &date, partition_spans, reader)
                .await
            {
                error!("Span Parquet write error for {site_id}/{date}: {e}");
            }
        }
    }

    async fn write_partition(
        &self,
        site_id: &str,
        date: &str,
        spans: Vec<AgentSpan>,
        reader: &SpanReader,
    ) -> anyhow::Result<()> {
        // Hive-style date partition: DataFusion's ListingTable defaults to
        // `listing_table_ignore_subdirectory=true`, which only descends into
        // segments containing `=`. So `date=2026-05-19/foo.parquet` is
        // recognised but a bare `2026-05-19/foo.parquet` is silently ignored.
        let dir = self.data_dir.join(site_id).join(format!("date={date}"));
        tokio::fs::create_dir_all(&dir).await?;

        let part_id = Ulid::new().to_string();
        let tmp_path = dir.join(format!("{part_id}.tmp"));
        let final_path = dir.join(format!("{part_id}.parquet"));

        let schema = agent_span_schema();
        let props = WriterProperties::builder()
            .set_writer_version(WriterVersion::PARQUET_2_0)
            .set_compression(Compression::SNAPPY)
            .set_max_row_group_size(50_000)
            .build();

        let batch = spans_to_record_batch(&spans, schema.clone())?;

        tokio::task::spawn_blocking({
            let tmp = tmp_path.clone();
            let final_ = final_path.clone();
            move || -> anyhow::Result<()> {
                let file = std::fs::File::create(&tmp)?;
                let mut writer = ArrowWriter::try_new(file, schema, Some(props))?;
                writer.write(&batch)?;
                writer.close()?;
                std::fs::rename(&tmp, &final_)?;
                Ok(())
            }
        })
        .await??;

        reader.ensure_site_registered(site_id).await?;

        info!(
            "Flushed {} spans → {}/{}/{}.parquet",
            spans.len(),
            site_id,
            date,
            part_id
        );
        Ok(())
    }
}

#[doc(hidden)]
pub fn spans_to_record_batch(
    spans: &[AgentSpan],
    schema: arrow::datatypes::SchemaRef,
) -> anyhow::Result<RecordBatch> {
    let n = spans.len();

    let mut id_b = StringBuilder::with_capacity(n, n * 26);
    let mut site_id_b = StringBuilder::with_capacity(n, n * 26);
    let mut agent_id_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut sess_b = StringBuilder::with_capacity(n, n * 26);
    let mut parent_b = StringBuilder::with_capacity(n, n * 26);
    let mut kind_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut model_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut started_b = TimestampMicrosecondBuilder::with_capacity(n).with_timezone("UTC");
    let mut ended_b = TimestampMicrosecondBuilder::with_capacity(n).with_timezone("UTC");
    let mut in_tok_b = UInt32Builder::with_capacity(n);
    let mut out_tok_b = UInt32Builder::with_capacity(n);
    let mut cache_r_b = UInt32Builder::with_capacity(n);
    let mut cache_c_b = UInt32Builder::with_capacity(n);
    let mut cost_b = Float64Builder::with_capacity(n);
    let mut tool_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut tool_hash_b = StringBuilder::with_capacity(n, n * 16);
    let mut stop_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut props_b = StringBuilder::with_capacity(n, n * 8);

    for s in spans {
        id_b.append_value(s.id.to_string());
        site_id_b.append_value(s.site_id.to_string());
        agent_id_b.append_value(&s.agent_id);
        sess_b.append_value(&s.agent_session_id);
        parent_b.append_option(s.parent_span_id.map(|u| u.to_string()).as_deref());
        kind_b.append_value(s.kind.as_str());
        model_b.append_value(&s.model);
        started_b.append_value(s.started_at.timestamp_micros());
        ended_b.append_value(s.ended_at.timestamp_micros());
        in_tok_b.append_value(s.input_tokens);
        out_tok_b.append_value(s.output_tokens);
        cache_r_b.append_value(s.cache_read_tokens);
        cache_c_b.append_value(s.cache_creation_tokens);
        cost_b.append_value(s.cost_usd);
        tool_b.append_option(s.tool_name.as_deref());
        tool_hash_b.append_option(s.tool_input_hash.as_deref());
        stop_b.append_option(s.stop_reason.as_deref());
        props_b.append_option(s.properties.as_ref().map(|p| p.to_string()).as_deref());
    }

    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(id_b.finish()),
            Arc::new(site_id_b.finish()),
            Arc::new(agent_id_b.finish()),
            Arc::new(sess_b.finish()),
            Arc::new(parent_b.finish()),
            Arc::new(kind_b.finish()),
            Arc::new(model_b.finish()),
            Arc::new(started_b.finish()),
            Arc::new(ended_b.finish()),
            Arc::new(in_tok_b.finish()),
            Arc::new(out_tok_b.finish()),
            Arc::new(cache_r_b.finish()),
            Arc::new(cache_c_b.finish()),
            Arc::new(cost_b.finish()),
            Arc::new(tool_b.finish()),
            Arc::new(tool_hash_b.finish()),
            Arc::new(stop_b.finish()),
            Arc::new(props_b.finish()),
        ],
    )?)
}
