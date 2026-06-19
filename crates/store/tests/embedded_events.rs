use std::time::Duration;

use chrono::{DateTime, Utc};
use ulid::Ulid;

use stomatopod_core::{
    config::EmbeddedConfig,
    domain::event::{DeviceType, Event, EventKind},
    query::pageviews::{Filter, Granularity, PageviewsQuery, TimeRange, TopListField},
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

/// Config that flushes the whole pending batch in a single interval tick.
/// `flush_buffer` drains at most `parquet_flush_rows` per flush, so a small
/// value would dribble multi-event batches out over many ticks; a large value
/// lets one tick land every row, keeping the seeded-data tests deterministic.
fn cfg_bulk(dir: &tempfile::TempDir) -> EmbeddedConfig {
    EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        wal_fsync_interval_ms: 0,
        parquet_flush_rows: 10_000,
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
            &[],
        )
        .await
        .unwrap();
    assert_eq!(top.rows.iter().map(|r| r.pageviews).sum::<u64>(), 3);
}

// ---------------------------------------------------------------------------
// Tier-1 analytics features: filters, OS/region dimensions, custom date range,
// and period-over-period comparison. These drive the real DataFusion SQL on a
// freshly-flushed parquet store, so they exercise the filter-clause builder
// and the new `TopListField` variants end to end.
// ---------------------------------------------------------------------------

/// A pageview with every dimension we slice/filter on settable. `ts` lets a
/// test place events in specific windows for range/comparison assertions.
#[allow(clippy::too_many_arguments)]
fn pageview(
    site_id: Ulid,
    ts: DateTime<Utc>,
    url: &str,
    browser: &str,
    os: &str,
    device: DeviceType,
    country: &str,
    region: &str,
) -> Event {
    Event {
        id: Ulid::new(),
        site_id,
        name: "pageview".into(),
        kind: EventKind::Pageview,
        timestamp: ts,
        received_at: ts,
        url: url.into(),
        referrer: None,
        utm_source: None,
        utm_medium: None,
        utm_campaign: None,
        utm_term: None,
        utm_content: None,
        browser: browser.into(),
        browser_version: "1".into(),
        os: os.into(),
        os_version: "1".into(),
        device_type: device,
        screen_width: None,
        screen_height: None,
        language: None,
        ip_anonymized: "127.0.0.0".into(),
        country_code: Some(country.into()),
        region: Some(region.into()),
        city: None,
        session_id: Ulid::new().to_bytes(),
        properties: None,
    }
}

fn around_now() -> TimeRange {
    let now = Utc::now();
    TimeRange {
        start: now - chrono::Duration::minutes(5),
        end: now + chrono::Duration::minutes(5),
    }
}

fn filter(s: &str) -> Filter {
    Filter::parse(s).expect("valid filter literal")
}

/// Ingest a fixed mix of six pageviews and wait for the parquet flush so the
/// read path hits disk. Returns the backend + site for assertions.
async fn seeded_backend(dir: &tempfile::TempDir) -> (EmbeddedBackend, Ulid) {
    let backend = EmbeddedBackend::open(&cfg_bulk(dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    backend
        .ingest_events(vec![
            // 3× Chrome / macOS / desktop / US-CA on /a
            pageview(
                site_id,
                now,
                "/a",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
            pageview(
                site_id,
                now,
                "/a",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
            pageview(
                site_id,
                now,
                "/a",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
            // 2× Safari / iOS / mobile / US-NY on /b
            pageview(
                site_id,
                now,
                "/b",
                "Safari",
                "iOS",
                DeviceType::Mobile,
                "US",
                "US-NY",
            ),
            pageview(
                site_id,
                now,
                "/b",
                "Safari",
                "iOS",
                DeviceType::Mobile,
                "US",
                "US-NY",
            ),
            // 1× Firefox / Linux / desktop / GB-ENG on /a/sub
            pageview(
                site_id,
                now,
                "/a/sub",
                "Firefox",
                "Linux",
                DeviceType::Desktop,
                "GB",
                "GB-ENG",
            ),
        ])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    (backend, site_id)
}

/// Look up one dimension value's pageview count in a `TopList`.
fn count_for(list: &stomatopod_core::query::pageviews::TopList, value: &str) -> u64 {
    list.rows
        .iter()
        .find(|r| r.value == value)
        .map(|r| r.pageviews)
        .unwrap_or(0)
}

#[tokio::test]
async fn top_list_covers_os_and_region_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    let (backend, site_id) = seeded_backend(&dir).await;
    let range = around_now();

    let os = backend
        .query_top_list(site_id, TopListField::Os, &range, 10, &[])
        .await
        .unwrap();
    assert_eq!(count_for(&os, "macOS"), 3);
    assert_eq!(count_for(&os, "iOS"), 2);
    assert_eq!(count_for(&os, "Linux"), 1);

    let region = backend
        .query_top_list(site_id, TopListField::Region, &range, 10, &[])
        .await
        .unwrap();
    assert_eq!(count_for(&region, "US-CA"), 3);
    assert_eq!(count_for(&region, "US-NY"), 2);
    assert_eq!(count_for(&region, "GB-ENG"), 1);
}

#[tokio::test]
async fn top_list_eq_filter_narrows_rows() {
    let dir = tempfile::tempdir().unwrap();
    let (backend, site_id) = seeded_backend(&dir).await;
    let range = around_now();

    // Pages visited by Chrome only — /a (3), not /b or /a/sub.
    let pages = backend
        .query_top_list(
            site_id,
            TopListField::Page,
            &range,
            10,
            &[filter("browser:eq:Chrome")],
        )
        .await
        .unwrap();
    assert_eq!(count_for(&pages, "/a"), 3);
    assert_eq!(count_for(&pages, "/b"), 0);
    assert_eq!(count_for(&pages, "/a/sub"), 0);
    assert_eq!(pages.rows.iter().map(|r| r.pageviews).sum::<u64>(), 3);
}

#[tokio::test]
async fn pageviews_eq_and_not_eq_filters_apply() {
    let dir = tempfile::tempdir().unwrap();
    let (backend, site_id) = seeded_backend(&dir).await;
    let range = around_now();

    let q = |filters: Vec<Filter>| PageviewsQuery {
        site_id,
        range: range.clone(),
        granularity: Granularity::Day,
        filters,
    };

    let safari = backend
        .query_pageviews(&q(vec![filter("browser:eq:Safari")]))
        .await
        .unwrap();
    assert_eq!(safari.total_pageviews, 2);

    // not_eq macOS → everything except the 3 macOS rows (2 Safari + 1 Firefox).
    let not_mac = backend
        .query_pageviews(&q(vec![filter("os:not_eq:macOS")]))
        .await
        .unwrap();
    assert_eq!(not_mac.total_pageviews, 3);

    // Two ANDed filters: desktop AND GB → only the Firefox row.
    let desktop_gb = backend
        .query_pageviews(&q(vec![
            filter("device_type:eq:desktop"),
            filter("country:eq:GB"),
        ]))
        .await
        .unwrap();
    assert_eq!(desktop_gb.total_pageviews, 1);
}

#[tokio::test]
async fn contains_and_starts_with_filters_match_url_prefixes() {
    let dir = tempfile::tempdir().unwrap();
    let (backend, site_id) = seeded_backend(&dir).await;
    let range = around_now();

    let q = |f: &str| PageviewsQuery {
        site_id,
        range: range.clone(),
        granularity: Granularity::Day,
        filters: vec![filter(f)],
    };

    // /a and /a/sub both contain and start with "/a" → 3 + 1 = 4.
    let contains = backend
        .query_pageviews(&q("url:contains:/a"))
        .await
        .unwrap();
    assert_eq!(contains.total_pageviews, 4);

    let starts = backend
        .query_pageviews(&q("url:starts_with:/a"))
        .await
        .unwrap();
    assert_eq!(starts.total_pageviews, 4);

    // "/b" matches only the 2 Safari pageviews.
    let only_b = backend
        .query_pageviews(&q("url:contains:/b"))
        .await
        .unwrap();
    assert_eq!(only_b.total_pageviews, 2);
}

#[tokio::test]
async fn custom_date_range_restricts_to_window() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();

    let recent = Utc::now();
    let old = recent - chrono::Duration::days(100);
    backend
        .ingest_events(vec![
            pageview(
                site_id,
                recent,
                "/now",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
            pageview(
                site_id,
                old,
                "/old",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
            pageview(
                site_id,
                old,
                "/old",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
        ])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // A from/to window around the old events only sees those two.
    let old_day = old.date_naive();
    let window = TimeRange::from_dates(
        old_day - chrono::Duration::days(1),
        old_day + chrono::Duration::days(1),
    );
    let q = PageviewsQuery {
        site_id,
        range: window,
        granularity: Granularity::Day,
        filters: vec![],
    };
    let result = backend.query_pageviews(&q).await.unwrap();
    assert_eq!(result.total_pageviews, 2);
}

#[tokio::test]
async fn previous_period_query_returns_prior_window_counts() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();

    let now = Utc::now();
    let this_week = now - chrono::Duration::days(2);
    let last_week = now - chrono::Duration::days(9);
    backend
        .ingest_events(vec![
            pageview(
                site_id,
                this_week,
                "/x",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
            pageview(
                site_id,
                this_week,
                "/x",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
            pageview(
                site_id,
                last_week,
                "/x",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
            pageview(
                site_id,
                last_week,
                "/x",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
            pageview(
                site_id,
                last_week,
                "/x",
                "Chrome",
                "macOS",
                DeviceType::Desktop,
                "US",
                "US-CA",
            ),
        ])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Current window: the last 7 days (2 events). Its `previous()` window is
    // the 7 days before that (3 events) — the comparison the dashboard shows.
    let current = TimeRange {
        start: now - chrono::Duration::days(7),
        end: now,
    };
    let prior = current.previous();
    let mk = |range: TimeRange| PageviewsQuery {
        site_id,
        range,
        granularity: Granularity::Day,
        filters: vec![],
    };

    let cur = backend.query_pageviews(&mk(current)).await.unwrap();
    let prev = backend.query_pageviews(&mk(prior)).await.unwrap();
    assert_eq!(cur.total_pageviews, 2);
    assert_eq!(prev.total_pageviews, 3);
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
