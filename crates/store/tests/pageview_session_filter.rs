use std::time::Duration;
use chrono::Utc;
use ulid::Ulid;

use stomatopod_core::{
    config::EmbeddedConfig,
    domain::event::{DeviceType, Event, EventKind},
    query::pageviews::{Granularity, PageviewsQuery, TimeRange},
    traits::StorageBackend,
};
use stomatopod_store::embedded::EmbeddedBackend;

fn cfg(dir: &tempfile::TempDir) -> EmbeddedConfig {
    EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        wal_fsync_interval_ms: 200,
        parquet_flush_rows: 1,
        parquet_flush_interval_s: 1,
        allow_ephemeral: true,
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
        screen_width: None,
        screen_height: None,
        language: None,
        ip_anonymized: "127.0.0.0".into(),
        country_code: None,
        region: None,
        city: None,
        session_id: Ulid::new().to_bytes(),
        properties: None,
    }
}

#[tokio::test]
async fn test_pageview_metrics() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    
    // Create a pageview session
    let mut ev_pv = make_event(site_id, "/a");
    ev_pv.timestamp = now;
    ev_pv.received_at = now;
    
    // Create a separate custom-only session
    let mut ev_custom = make_event(site_id, "/a");
    ev_custom.timestamp = now;
    ev_custom.received_at = now;
    ev_custom.name = "__click__".into();
    ev_custom.kind = EventKind::Custom;
    // `make_event` generates a unique session ID for each call, so this session
    // is distinct from the pageview session above.
    
    let events = vec![ev_pv, ev_custom];
    backend.ingest_events(events).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let res = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range: TimeRange {
                start: now - chrono::Duration::hours(1),
                end: now + chrono::Duration::hours(1),
            },
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap();

    assert_eq!(res.total_pageviews, 1);
    assert_eq!(res.total_sessions, 1);
}

#[tokio::test]
async fn test_pageview_metrics_custom_only() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    
    let mut ev_custom = make_event(site_id, "/a");
    ev_custom.timestamp = now;
    ev_custom.received_at = now;
    ev_custom.name = "__click__".into();
    ev_custom.kind = EventKind::Custom;
    
    backend.ingest_events(vec![ev_custom]).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let res = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range: TimeRange {
                start: now - chrono::Duration::hours(1),
                end: now + chrono::Duration::hours(1),
            },
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap();

    assert_eq!(res.total_sessions, 0); 
    assert_eq!(res.total_pageviews, 0); 
}
