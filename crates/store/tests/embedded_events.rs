use std::time::Duration;

use chrono::Utc;
use ulid::Ulid;

use stomatopod_core::{
    config::EmbeddedConfig,
    domain::event::{DeviceType, Event, EventKind},
    query::pageviews::{Granularity, PageviewsQuery, TimeRange, TopListField},
    traits::StorageBackend,
};
use stomatopod_store::embedded::EmbeddedBackend;

fn cfg(dir: &tempfile::TempDir) -> EmbeddedConfig {
    EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        wal_fsync_interval_ms: 200,
        parquet_flush_rows: 1,
        parquet_flush_interval_s: 1,
    }
}

fn make_event(site_id: Ulid, url: &str) -> Event {
    let now = Utc::now();
    Event {
        id: Ulid::new(),
        site_id,
        name: "pageview".into(),
        kind: EventKind::Pageview,
        timestamp: now,
        received_at: now,
        url: url.into(),
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
        device_type: DeviceType::Desktop,
        screen_width: Some(1920),
        screen_height: Some(1080),
        language: Some("en".into()),
        ip_anonymized: "127.0.0.0".into(),
        country_code: Some("US".into()),
        region: None,
        city: None,
        session_id: [0u8; 16],
        properties: None,
    }
}

/// Regression test for the partition-layout bug: events written to parquet
/// must be visible to DataFusion queries after the in-memory buffer is
/// drained. Earlier code wrote a flat `<date>/` layout that the listing
/// table silently skipped because of `listing_table_ignore_subdirectory`.
#[tokio::test]
async fn pageview_round_trip_through_parquet() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg(&dir)).await.unwrap();
    let site_id = Ulid::new();

    backend
        .ingest_events(vec![
            make_event(site_id, "/a"),
            make_event(site_id, "/a"),
            make_event(site_id, "/b"),
        ])
        .await
        .unwrap();

    // Let the flush worker drain the channel and write parquet, then drop
    // the in-memory buffer so the read path has to hit disk.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Sanity check: files landed on disk in Hive-style `date=...` layout.
    let site_dir = dir.path().join("parquet").join(site_id.to_string());
    let mut hive_dirs = vec![];
    let mut e = tokio::fs::read_dir(&site_dir).await.unwrap();
    while let Some(d) = e.next_entry().await.unwrap() {
        hive_dirs.push(d.file_name().to_string_lossy().into_owned());
    }
    assert!(
        hive_dirs.iter().any(|n| n.starts_with("date=")),
        "expected Hive-style date= partitions, got {hive_dirs:?}"
    );

    let now = Utc::now();
    let result = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range: TimeRange {
                start: now - chrono::Duration::minutes(5),
                end: now + chrono::Duration::minutes(5),
            },
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap();
    assert_eq!(
        result.total_pageviews, 3,
        "parquet files must be reachable through ListingTable"
    );

    let top = backend
        .query_top_list(
            site_id,
            TopListField::Page,
            &TimeRange {
                start: now - chrono::Duration::minutes(5),
                end: now + chrono::Duration::minutes(5),
            },
            10,
        )
        .await
        .unwrap();
    assert_eq!(top.rows.iter().map(|r| r.pageviews).sum::<u64>(), 3);
}

/// Older builds wrote `<site>/<YYYY-MM-DD>/foo.parquet`. The reader must
/// migrate those to `<site>/date=<YYYY-MM-DD>/foo.parquet` on startup so
/// historical data stays queryable.
#[tokio::test]
async fn legacy_flat_partitions_are_migrated_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let site_id = Ulid::new();

    // First boot to lay down a valid parquet file in the new layout.
    {
        let backend = EmbeddedBackend::open(&cfg(&dir)).await.unwrap();
        backend
            .ingest_events(vec![make_event(site_id, "/legacy")])
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // Rewrite the layout to look like the old flat structure.
    let site_dir = dir.path().join("parquet").join(site_id.to_string());
    let mut entries = std::fs::read_dir(&site_dir).unwrap();
    let hive_dir = entries.next().unwrap().unwrap().path();
    let date_segment = hive_dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .strip_prefix("date=")
        .unwrap()
        .to_string();
    let flat_dir = site_dir.join(&date_segment);
    std::fs::rename(&hive_dir, &flat_dir).unwrap();
    assert!(flat_dir.exists(), "flat layout did not get set up");
    assert!(!hive_dir.exists());

    // Reopening should migrate the directory back to Hive form.
    let backend = EmbeddedBackend::open(&cfg(&dir)).await.unwrap();
    assert!(hive_dir.exists(), "reader did not rename flat → Hive");
    assert!(!flat_dir.exists());

    let now = Utc::now();
    let result = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range: TimeRange {
                start: now - chrono::Duration::days(1),
                end: now + chrono::Duration::days(1),
            },
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap();
    assert_eq!(result.total_pageviews, 1);
}
