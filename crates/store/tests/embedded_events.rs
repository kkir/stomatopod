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

// ---- Tier-2 analytics: entry/exit, realtime, goals, export ----

/// Build a single event with explicit session, url, kind/name and timestamp.
#[allow(clippy::too_many_arguments)]
fn mk(
    site_id: Ulid,
    sid: u8,
    url: &str,
    kind: EventKind,
    name: &str,
    ts: chrono::DateTime<Utc>,
    properties: Option<serde_json::Value>,
) -> Event {
    let mut e = make_event(site_id, url);
    e.id = Ulid::new();
    e.session_id = [sid; 16];
    e.kind = kind;
    e.name = name.into();
    e.timestamp = ts;
    e.received_at = ts;
    e.referrer = Some("https://ref.example/".into());
    e.properties = properties;
    e
}

/// Seed three sessions (A: /home → /pricing, B: /home bounce, C: /blog →
/// /home) plus one `signup` custom event, then flush to parquet. Returns the
/// site id and a wide query range.
async fn seed_sessions(backend: &EmbeddedBackend) -> (Ulid, TimeRange) {
    let site_id = Ulid::new();
    let now = Utc::now();
    let t = |secs: i64| now - chrono::Duration::seconds(300 - secs);
    let pv = EventKind::Pageview;
    backend
        .ingest_events(vec![
            mk(site_id, 1, "/home", pv, "pageview", t(0), None),
            mk(site_id, 1, "/pricing", pv, "pageview", t(10), None),
            mk(site_id, 2, "/home", pv, "pageview", t(0), None),
            mk(site_id, 3, "/blog", pv, "pageview", t(0), None),
            mk(site_id, 3, "/home", pv, "pageview", t(5), None),
            mk(
                site_id,
                1,
                "/welcome",
                EventKind::Custom,
                "signup",
                t(12),
                Some(serde_json::json!({"plan": "pro"})),
            ),
        ])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let range = TimeRange {
        start: now - chrono::Duration::hours(1),
        end: now + chrono::Duration::hours(1),
    };
    (site_id, range)
}

#[tokio::test]
async fn entry_pages_report_counts_sessions_and_bounces() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, range) = seed_sessions(&backend).await;

    let entry = backend
        .query_entry_pages(site_id, &range, 20, &[])
        .await
        .unwrap();

    let home = entry
        .rows
        .iter()
        .find(|r| r.url == "/home")
        .expect("/home should be a top entry page");
    // Sessions A and B both start on /home; only B (single pageview) bounced.
    assert_eq!(home.sessions, 2, "two sessions enter on /home");
    assert!(
        (home.bounce_rate - 50.0).abs() < 0.01,
        "one of two /home entries bounced, got {}",
        home.bounce_rate
    );
    assert!(
        entry.rows.iter().any(|r| r.url == "/blog"),
        "/blog is also an entry page"
    );
}

#[tokio::test]
async fn exit_pages_report_counts_exits() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, range) = seed_sessions(&backend).await;

    let exit = backend
        .query_exit_pages(site_id, &range, 20, &[])
        .await
        .unwrap();

    // Sessions B and C both end on /home; A ends on /pricing.
    let home = exit
        .rows
        .iter()
        .find(|r| r.url == "/home")
        .expect("/home should be an exit page");
    assert_eq!(home.exits, 2, "two sessions exit on /home");
    assert!(
        exit.rows
            .iter()
            .any(|r| r.url == "/pricing" && r.exits == 1),
        "/pricing is an exit page with one exit"
    );
}

#[tokio::test]
async fn realtime_snapshot_reports_active_sessions_and_events() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, _range) = seed_sessions(&backend).await;

    let rt = backend.query_realtime(site_id, 30).await.unwrap();
    assert_eq!(rt.active_sessions, 3, "three distinct sessions are active");
    assert!(rt.pageviews_per_minute > 0.0);
    // Last pageview per session: A→/pricing, B→/home, C→/home.
    let home = rt
        .top_pages
        .iter()
        .find(|p| p.url == "/home")
        .expect("/home is an active page");
    assert_eq!(home.active_sessions, 2);
    // The signup custom event shows up in the live feed.
    assert!(
        rt.recent_events.iter().any(|e| e.name == "signup"),
        "recent events should include the signup, got {:?}",
        rt.recent_events
    );
}

#[tokio::test]
async fn goal_stats_compute_completions_and_conversion_rate() {
    use stomatopod_core::query::{analytics::GoalQuery, pageviews::Granularity};

    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, range) = seed_sessions(&backend).await;

    let stats = backend
        .query_goal(&GoalQuery {
            site_id,
            event_name: "signup".into(),
            filters: vec![],
            granularity: Granularity::Day,
            range,
        })
        .await
        .unwrap();

    assert_eq!(stats.completions, 1, "one signup event");
    assert_eq!(stats.unique_completions, 1);
    // 1 converting session out of 3 total → ~33.3%.
    assert!(
        (stats.conversion_rate - 33.333).abs() < 0.1,
        "conversion rate should be ~33.3%, got {}",
        stats.conversion_rate
    );
    assert!(
        !stats.timeseries.is_empty(),
        "timeseries should have a bucket"
    );
}

#[tokio::test]
async fn sessions_export_derives_entry_exit_and_bounce() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, range) = seed_sessions(&backend).await;

    let sessions = backend.query_sessions(site_id, &range, 100).await.unwrap();
    assert_eq!(sessions.len(), 3, "three derived sessions");

    // Session A (id [1;16]) hex: "0101...01".
    let a = sessions
        .iter()
        .find(|s| s.session_id == "01".repeat(16))
        .expect("session A present");
    assert_eq!(a.entry_url, "/home");
    assert_eq!(a.exit_url, "/pricing");
    assert_eq!(a.pageviews, 2);
    assert!(!a.is_bounce);

    let b = sessions
        .iter()
        .find(|s| s.session_id == "02".repeat(16))
        .expect("session B present");
    assert_eq!(b.entry_url, "/home");
    assert_eq!(b.exit_url, "/home");
    assert!(b.is_bounce, "single-pageview session B is a bounce");
}

#[tokio::test]
async fn events_export_returns_raw_rows() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, range) = seed_sessions(&backend).await;

    let events = backend
        .query_events_list(site_id, &range, 1000)
        .await
        .unwrap();
    assert_eq!(events.len(), 6, "all six seeded events are exported");
    let signup = events
        .iter()
        .find(|e| e.name == "signup")
        .expect("signup event present");
    assert_eq!(signup.kind, "custom");
    assert!(
        signup.properties.as_deref().unwrap_or("").contains("pro"),
        "properties JSON should round-trip, got {:?}",
        signup.properties
    );
}
