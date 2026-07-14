//! Integration tests for [`stomatopod_ingest::batch::run_batcher`].

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use ulid::Ulid;

use stomatopod_core::{
    config::EmbeddedConfig,
    domain::event::{DeviceType, Event, EventKind},
    query::pageviews::{Granularity, PageviewsQuery, TimeRange},
    traits::StorageBackend,
};
use stomatopod_ingest::batch::run_batcher;
use stomatopod_store::embedded::EmbeddedBackend;

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
        os_version: "6".into(),
        device_type: DeviceType::Desktop,
        screen_width: Some(1920),
        screen_height: Some(1080),
        language: Some("en".into()),
        ip_anonymized: "127.0.0.0".into(),
        country_code: Some("US".into()),
        region: None,
        city: None,
        session_id: [1u8; 16],
        properties: None,
    }
}

async fn open_backend() -> (tempfile::TempDir, Arc<EmbeddedBackend>) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        wal_fsync_interval_ms: 0,
        parquet_flush_rows: 1,
        parquet_flush_interval_s: 1,
        allow_ephemeral: true,
        ..Default::default()
    };
    let backend = Arc::new(EmbeddedBackend::open(&cfg).await.unwrap());
    (dir, backend)
}

async fn wait_pageviews(backend: &EmbeddedBackend, site_id: Ulid, want: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let now = Utc::now();
        let result = backend
            .query_pageviews(&PageviewsQuery {
                site_id,
                range: TimeRange {
                    start: now - chrono::Duration::hours(1),
                    end: now + chrono::Duration::minutes(5),
                },
                granularity: Granularity::Day,
                filters: vec![],
            })
            .await
            .unwrap();
        if result.total_pageviews >= want {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for {want} pageviews, got {}",
                result.total_pageviews
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Filling the batch triggers an immediate flush (no need to wait for timer).
#[tokio::test]
async fn batcher_flushes_when_batch_size_reached() {
    let (_dir, backend) = open_backend().await;
    let site_id = Ulid::new();
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let be: Arc<dyn StorageBackend> = backend.clone();
    let handle = tokio::spawn(async move {
        // Large interval so only size-based flush fires in this test.
        run_batcher(rx, be, 3, 60_000).await;
    });

    for path in ["/a", "/b", "/c"] {
        tx.send(make_event(site_id, path)).await.unwrap();
    }

    wait_pageviews(&backend, site_id, 3).await;
    drop(tx);
    handle.await.unwrap();
}

/// A partial batch is flushed when the interval ticks.
#[tokio::test]
async fn batcher_flushes_on_interval() {
    let (_dir, backend) = open_backend().await;
    let site_id = Ulid::new();
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let be: Arc<dyn StorageBackend> = backend.clone();
    let handle = tokio::spawn(async move {
        // batch_size high; interval short.
        run_batcher(rx, be, 100, 50).await;
    });

    tx.send(make_event(site_id, "/interval-only"))
        .await
        .unwrap();
    wait_pageviews(&backend, site_id, 1).await;
    drop(tx);
    handle.await.unwrap();
}

/// Remaining buffered events are flushed when the sender is dropped.
#[tokio::test]
async fn batcher_final_flush_on_shutdown() {
    let (_dir, backend) = open_backend().await;
    let site_id = Ulid::new();
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let be: Arc<dyn StorageBackend> = backend.clone();
    let handle = tokio::spawn(async move {
        // Huge interval and batch so only shutdown flush can land the event.
        run_batcher(rx, be, 10_000, 60_000).await;
    });

    // Let the batcher consume the immediate first interval tick with an empty
    // buffer, then enqueue a partial batch and close the channel.
    tokio::time::sleep(Duration::from_millis(30)).await;
    tx.send(make_event(site_id, "/shutdown")).await.unwrap();
    // Give the batcher a moment to recv into its local buf.
    tokio::time::sleep(Duration::from_millis(30)).await;
    drop(tx);
    handle.await.unwrap();

    wait_pageviews(&backend, site_id, 1).await;
}
