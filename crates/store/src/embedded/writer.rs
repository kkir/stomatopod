use std::{path::PathBuf, sync::Arc, time::Duration};

use arrow::{
    array::{
        FixedSizeBinaryBuilder, StringBuilder, StringDictionaryBuilder,
        TimestampMicrosecondBuilder, UInt16Builder,
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

use stomatopod_core::domain::event::Event;

use super::{
    arrow_schema::event_schema, buffer::EventBuffer, reader::EmbeddedReader, util::ulid_to_str,
    wal::Wal,
};

pub struct ParquetWriter {
    data_dir: PathBuf,
    flush_rows: usize,
    flush_interval_s: u64,
}

impl ParquetWriter {
    pub fn new(data_dir: PathBuf, flush_rows: usize, flush_interval_s: u64) -> Self {
        Self {
            data_dir,
            flush_rows,
            flush_interval_s,
        }
    }

    pub async fn run(
        self,
        mut rx: mpsc::Receiver<Vec<Event>>,
        buffer: Arc<EventBuffer>,
        wal: Arc<Wal>,
        reader: Arc<EmbeddedReader>,
    ) {
        let mut interval = tokio::time::interval(Duration::from_secs(self.flush_interval_s));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                Some(events) = rx.recv() => {
                    // Write to WAL first for durability, then buffer for queries
                    if let Err(e) = wal.append(&events) {
                        error!("WAL append error: {e}");
                    }
                    buffer.push_batch(events);

                    // Flush if buffer is getting full
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

    async fn flush_buffer(&self, buffer: &EventBuffer, reader: &EmbeddedReader) {
        let events = buffer.drain(self.flush_rows);
        if events.is_empty() {
            return;
        }

        // Group events by (site_id, date) for partition layout
        let mut by_partition: std::collections::HashMap<(String, String), Vec<Event>> =
            std::collections::HashMap::new();

        for event in events {
            let date = event.timestamp.format("%Y-%m-%d").to_string();
            let key = (event.site_id.to_string(), date);
            by_partition.entry(key).or_default().push(event);
        }

        for ((site_id, date), partition_events) in by_partition {
            if let Err(e) = self
                .write_partition(&site_id, &date, partition_events, reader)
                .await
            {
                error!("Parquet write error for {site_id}/{date}: {e}");
            }
        }
    }

    async fn write_partition(
        &self,
        site_id: &str,
        date: &str,
        events: Vec<Event>,
        reader: &EmbeddedReader,
    ) -> anyhow::Result<()> {
        // Hive-style date partition. DataFusion's `ListingTable` defaults to
        // `listing_table_ignore_subdirectory=true`, which only descends into
        // path segments containing `=`. A bare `<date>/foo.parquet` is
        // silently skipped by the reader; `date=<date>/foo.parquet` is not.
        let dir = self.data_dir.join(site_id).join(format!("date={date}"));
        tokio::fs::create_dir_all(&dir).await?;

        let part_id = Ulid::new().to_string();
        let tmp_path = dir.join(format!("{part_id}.tmp"));
        let final_path = dir.join(format!("{part_id}.parquet"));

        let schema = event_schema();
        let props = WriterProperties::builder()
            .set_writer_version(WriterVersion::PARQUET_2_0)
            .set_compression(Compression::SNAPPY)
            .set_max_row_group_size(50_000)
            .build();

        let batch = events_to_record_batch(&events, schema.clone())?;

        tokio::task::spawn_blocking({
            let tmp = tmp_path.clone();
            let final_ = final_path.clone();
            move || -> anyhow::Result<()> {
                let file = std::fs::File::create(&tmp)?;
                let mut writer = ArrowWriter::try_new(file, schema, Some(props))?;
                writer.write(&batch)?;
                writer.close()?;
                // Atomic rename — DataFusion never sees partial files
                std::fs::rename(&tmp, &final_)?;
                Ok(())
            }
        })
        .await??;

        // Tell DataFusion reader about the new site partition
        reader.ensure_site_registered(site_id).await?;

        info!(
            "Flushed {} events → {}/{}/{}.parquet",
            events.len(),
            site_id,
            date,
            part_id
        );
        Ok(())
    }
}

#[doc(hidden)]
pub fn events_to_record_batch(
    events: &[Event],
    schema: arrow::datatypes::SchemaRef,
) -> anyhow::Result<RecordBatch> {
    let n = events.len();

    let mut id_b = StringBuilder::with_capacity(n, n * 26);
    let mut site_id_b = StringBuilder::with_capacity(n, n * 26);
    let mut name_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut kind_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut ts_b = TimestampMicrosecondBuilder::with_capacity(n).with_timezone("UTC");
    let mut recv_b = TimestampMicrosecondBuilder::with_capacity(n).with_timezone("UTC");
    let mut url_b = StringBuilder::with_capacity(n, n * 64);
    let mut referrer_b = StringBuilder::with_capacity(n, n * 32);
    let mut utm_src_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut utm_med_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut utm_cmp_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut utm_term_b = StringBuilder::with_capacity(n, n * 16);
    let mut utm_con_b = StringBuilder::with_capacity(n, n * 16);
    let mut browser_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut browser_ver_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut os_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut os_ver_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut dev_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut sw_b = UInt16Builder::with_capacity(n);
    let mut sh_b = UInt16Builder::with_capacity(n);
    let mut lang_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut ip_b = StringBuilder::with_capacity(n, n * 16);
    let mut country_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut region_b: StringDictionaryBuilder<Int32Type> = StringDictionaryBuilder::new();
    let mut city_b = StringBuilder::with_capacity(n, n * 16);
    let mut session_b = FixedSizeBinaryBuilder::with_capacity(n, 16);
    let mut props_b = StringBuilder::with_capacity(n, n * 8);

    // Scratch buffers for the per-row ULID strings — avoids two `String`
    // allocations per event.
    let mut ulid_buf = [0u8; 26];
    let mut site_buf = [0u8; 26];
    for e in events {
        id_b.append_value(ulid_to_str(e.id, &mut ulid_buf));
        site_id_b.append_value(ulid_to_str(e.site_id, &mut site_buf));
        name_b.append_value(&e.name);
        kind_b.append_value(e.kind.as_str());
        ts_b.append_value(e.timestamp.timestamp_micros());
        recv_b.append_value(e.received_at.timestamp_micros());
        url_b.append_value(&e.url);
        referrer_b.append_option(e.referrer.as_deref());
        utm_src_b.append_option(e.utm_source.as_deref());
        utm_med_b.append_option(e.utm_medium.as_deref());
        utm_cmp_b.append_option(e.utm_campaign.as_deref());
        utm_term_b.append_option(e.utm_term.as_deref());
        utm_con_b.append_option(e.utm_content.as_deref());
        browser_b.append_value(&e.browser);
        browser_ver_b.append_value(&e.browser_version);
        os_b.append_value(&e.os);
        os_ver_b.append_value(&e.os_version);
        dev_b.append_value(e.device_type.as_str());
        sw_b.append_option(e.screen_width);
        sh_b.append_option(e.screen_height);
        lang_b.append_option(e.language.as_deref());
        ip_b.append_value(&e.ip_anonymized);
        country_b.append_option(e.country_code.as_deref());
        region_b.append_option(e.region.as_deref());
        city_b.append_option(e.city.as_deref());
        session_b.append_value(e.session_id)?;
        props_b.append_option(e.properties.as_ref().map(|p| p.to_string()).as_deref());
    }

    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(id_b.finish()),
            Arc::new(site_id_b.finish()),
            Arc::new(name_b.finish()),
            Arc::new(kind_b.finish()),
            Arc::new(ts_b.finish()),
            Arc::new(recv_b.finish()),
            Arc::new(url_b.finish()),
            Arc::new(referrer_b.finish()),
            Arc::new(utm_src_b.finish()),
            Arc::new(utm_med_b.finish()),
            Arc::new(utm_cmp_b.finish()),
            Arc::new(utm_term_b.finish()),
            Arc::new(utm_con_b.finish()),
            Arc::new(browser_b.finish()),
            Arc::new(browser_ver_b.finish()),
            Arc::new(os_b.finish()),
            Arc::new(os_ver_b.finish()),
            Arc::new(dev_b.finish()),
            Arc::new(sw_b.finish()),
            Arc::new(sh_b.finish()),
            Arc::new(lang_b.finish()),
            Arc::new(ip_b.finish()),
            Arc::new(country_b.finish()),
            Arc::new(region_b.finish()),
            Arc::new(city_b.finish()),
            Arc::new(session_b.finish()),
            Arc::new(props_b.finish()),
        ],
    )?)
}
