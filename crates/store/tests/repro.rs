use chrono::Utc;
use std::time::Duration;
use stomatopod_core::domain::event::{DeviceType, Event, EventKind};
use stomatopod_core::query::pageviews::PageviewsQuery;
use stomatopod_core::traits::StorageBackend;
use stomatopod_store::embedded::EmbeddedBackend;
use tempfile::TempDir;
use ulid::Ulid;

// We use the public new() for config if available, or just mock it by using open_in_memory or whatever.
// Let's just look at `stomatopod_store::embedded::EmbeddedBackend` methods.

#[tokio::test]
async fn test_pageview_metrics() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&stomatopod_core::config::EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        wal_fsync_interval_ms: 1000,
        parquet_flush_rows: 1000,
        parquet_flush_interval_s: 1,
        allow_ephemeral: false,
    })
    .await
    .unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    let mut events = vec![];
    events.push(Event {
        id: Ulid::new(),
        site_id,
        name: "pageview".into(),
        kind: EventKind::Pageview,
        timestamp: now,
        received_at: now,
        url: "/a".into(),
        referrer: None,
        utm_source: None,
        utm_medium: None,
        utm_campaign: None,
        utm_term: None,
        utm_content: None,
        browser: "Chrome".into(),
        browser_version: "1".into(),
        os: "macOS".into(),
        os_version: "1".into(),
        device_type: DeviceType::Desktop,
        screen_width: None,
        screen_height: None,
        language: None,
        ip_anonymized: "127.0.0.0".into(),
        country_code: None,
        region: None,
        city: None,
        session_id: Ulid::new().to_bytes(),
        properties: None,
    });
    events.push(Event {
        id: Ulid::new(),
        site_id,
        name: "__click__".into(),
        kind: EventKind::Custom,
        timestamp: now,
        received_at: now,
        url: "/a".into(),
        referrer: None,
        utm_source: None,
        utm_medium: None,
        utm_campaign: None,
        utm_term: None,
        utm_content: None,
        browser: "Chrome".into(),
        browser_version: "1".into(),
        os: "macOS".into(),
        os_version: "1".into(),
        device_type: DeviceType::Desktop,
        screen_width: None,
        screen_height: None,
        language: None,
        ip_anonymized: "127.0.0.0".into(),
        country_code: None,
        region: None,
        city: None,
        session_id: Ulid::new().to_bytes(),
        properties: None,
    });
    backend.ingest_events(events).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let res = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range: stomatopod_core::query::pageviews::TimeRange {
                start: now - chrono::Duration::hours(1),
                end: now + chrono::Duration::hours(1),
            },
            granularity: stomatopod_core::query::pageviews::Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap();

    assert_eq!(res.total_pageviews, 1);
}
#[tokio::test]
async fn test_pageview_metrics_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let backend = stomatopod_store::embedded::EmbeddedBackend::open(
        &stomatopod_core::config::EmbeddedConfig {
            data_dir: dir.path().to_path_buf(),
            wal_fsync_interval_ms: 1000,
            parquet_flush_rows: 1000,
            parquet_flush_interval_s: 1,
            allow_ephemeral: false,
        },
    )
    .await
    .unwrap();
    let site_id = ulid::Ulid::new();
    let now = chrono::Utc::now();
    let mut events = vec![];
    events.push(stomatopod_core::domain::event::Event {
        id: ulid::Ulid::new(),
        site_id,
        name: "__click__".into(),
        kind: stomatopod_core::domain::event::EventKind::Custom,
        timestamp: now,
        received_at: now,
        url: "/a".into(),
        referrer: None,
        utm_source: None,
        utm_medium: None,
        utm_campaign: None,
        utm_term: None,
        utm_content: None,
        browser: "Chrome".into(),
        browser_version: "1".into(),
        os: "macOS".into(),
        os_version: "1".into(),
        device_type: stomatopod_core::domain::event::DeviceType::Desktop,
        screen_width: None,
        screen_height: None,
        language: None,
        ip_anonymized: "127.0.0.0".into(),
        country_code: None,
        region: None,
        city: None,
        session_id: ulid::Ulid::new().to_bytes(),
        properties: None,
    });
    backend.ingest_events(events).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let res = backend
        .query_pageviews(&stomatopod_core::query::pageviews::PageviewsQuery {
            site_id,
            range: stomatopod_core::query::pageviews::TimeRange {
                start: now - chrono::Duration::hours(1),
                end: now + chrono::Duration::hours(1),
            },
            granularity: stomatopod_core::query::pageviews::Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap();

    assert_eq!(res.total_sessions, 0); // Is it 0?
}
