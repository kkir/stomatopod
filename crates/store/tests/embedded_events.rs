use std::time::Duration;

use chrono::{DateTime, Utc};
use ulid::Ulid;

use stomatopod_core::{
    config::EmbeddedConfig,
    domain::{
        annotation::Annotation,
        event::{DeviceType, Event, EventKind},
    },
    query::pageviews::{Filter, Granularity, PageviewsQuery, TimeRange, TopListField},
    traits::{MetaStore, StorageBackend},
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

#[tokio::test]
async fn paths_report_counts_top_sequences() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, range) = seed_sessions(&backend).await;

    let report = backend.query_paths(site_id, &range, 3, 25).await.unwrap();
    assert_eq!(report.total_sessions, 3, "three pageview sessions");
    // Session A walked /home -> /pricing (custom signup excluded).
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.steps == vec!["/home".to_string(), "/pricing".to_string()]),
        "expected /home -> /pricing path, got {:?}",
        report.rows
    );
}

#[tokio::test]
async fn sparklines_rank_top_pages() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, range) = seed_sessions(&backend).await;

    let spark = backend
        .query_top_sparklines(site_id, TopListField::Page, &range, 20, &[])
        .await
        .unwrap();
    let home = spark
        .rows
        .iter()
        .find(|r| r.value == "/home")
        .expect("/home should have a sparkline");
    // /home was viewed by all three sessions.
    assert_eq!(home.total, 3);
    assert_eq!(home.points.iter().sum::<u64>(), 3);
}

#[tokio::test]
async fn retention_grid_groups_into_one_cohort() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, range) = seed_sessions(&backend).await;

    let grid = backend.query_retention(site_id, &range).await.unwrap();
    // All seeded events fall in one week, so one cohort of three sessions.
    assert_eq!(grid.cohorts.len(), 1);
    assert_eq!(grid.cohorts[0].size, 3);
    assert_eq!(grid.cohorts[0].cells[0].returning, 3);
}

#[tokio::test]
async fn annotations_round_trip_through_meta() {
    use stomatopod_core::domain::{
        org::{Organization, Plan},
        site::Site,
    };
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    // Annotations carry a FK to sites, so seed an org + site first.
    let org = Organization {
        id: Ulid::new(),
        name: "Org".into(),
        slug: format!("org-{}", Ulid::new()),
        plan: Plan::SelfHosted,
        created_at: Utc::now(),
    };
    backend.create_org(&org).await.unwrap();
    let site = Site {
        id: Ulid::new(),
        org_id: org.id,
        domain: "ann.example.com".into(),
        name: "Ann".into(),
        timezone: "UTC".into(),
        public_key: format!("pk-{}", Ulid::new()),
        created_at: Utc::now(),
        is_active: true,
    };
    backend.create_site(&site).await.unwrap();
    let site_id = site.id;
    let day = chrono::NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();

    let ann = Annotation {
        id: Ulid::new(),
        site_id,
        date: day,
        text: "Deployed v2".into(),
        created_at: Utc::now(),
    };
    backend.create_annotation(&ann).await.unwrap();

    let in_range = backend
        .list_annotations(
            site_id,
            chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(in_range.len(), 1);
    assert_eq!(in_range[0].text, "Deployed v2");

    // A window before the annotation date excludes it.
    let out_of_range = backend
        .list_annotations(
            site_id,
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        )
        .await
        .unwrap();
    assert!(out_of_range.is_empty());

    backend.delete_annotation(ann.id).await.unwrap();
    assert!(backend.get_annotation(ann.id).await.unwrap().is_none());
}

// ---------------------------------------------------------------------------
// Tier-4 analytics: raw custom-event fetch + in-process aggregations for
// Core Web Vitals, scroll/engagement, A/B, revenue, heatmaps, and search.
// These drive real DataFusion SQL on a flushed parquet store end to end.
// ---------------------------------------------------------------------------

use stomatopod_core::query::tier4::{
    ClickHeatmap, EventPropRow, ExperimentList, ExperimentResult, RevenueBreakdown,
    RevenueDimension, RevenueSummary, ScrollHeatmap, ScrollReport, SearchReport, VitalsReport,
};

/// Build a custom event with the given name, session, url, and JSON props.
#[allow(clippy::too_many_arguments)]
fn custom_event(
    site_id: Ulid,
    ts: DateTime<Utc>,
    name: &str,
    session: [u8; 16],
    url: &str,
    props: serde_json::Value,
) -> Event {
    Event {
        id: Ulid::new(),
        site_id,
        name: name.into(),
        kind: EventKind::Custom,
        timestamp: ts,
        received_at: ts,
        url: url.into(),
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
        country_code: Some("US".into()),
        region: None,
        city: None,
        session_id: session,
        properties: Some(props),
    }
}

fn sid(n: u8) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0] = n;
    b
}

async fn fetch_props(
    backend: &EmbeddedBackend,
    site_id: Ulid,
    names: &[&str],
) -> Vec<EventPropRow> {
    let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    backend
        .query_event_props(site_id, &names, &around_now(), 100_000)
        .await
        .unwrap()
}

#[tokio::test]
async fn tier4_event_props_roundtrip_and_name_filter() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    backend
        .ingest_events(vec![
            custom_event(
                site_id,
                now,
                "__vital__",
                sid(1),
                "/a",
                serde_json::json!({"metric":"LCP","value":1200,"rating":"good","url":"/a"}),
            ),
            custom_event(
                site_id,
                now,
                "__scroll__",
                sid(1),
                "/a",
                serde_json::json!({"depth":50,"url":"/a"}),
            ),
        ])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Name filter returns only the requested event kind.
    let vitals = fetch_props(&backend, site_id, &["__vital__"]).await;
    assert_eq!(vitals.len(), 1);
    assert_eq!(vitals[0].name, "__vital__");
    assert_eq!(
        vitals[0].properties.as_ref().unwrap()["metric"],
        serde_json::json!("LCP")
    );

    // Empty names returns all custom events.
    let all = fetch_props(&backend, site_id, &[]).await;
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn tier4_core_web_vitals_percentiles() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    let mut events = vec![];
    for (i, v) in [1000.0, 2000.0, 3000.0, 4000.0].into_iter().enumerate() {
        let rating = if v < 2500.0 { "good" } else { "poor" };
        events.push(custom_event(
            site_id,
            now,
            "__vital__",
            sid(i as u8),
            "/p",
            serde_json::json!({"metric":"LCP","value":v,"rating":rating,"url":"/p"}),
        ));
    }
    backend.ingest_events(events).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let rows = fetch_props(&backend, site_id, &["__vital__"]).await;
    let report = VitalsReport::from_rows(&rows, None);
    assert_eq!(report.lcp.samples, 4);
    assert_eq!(report.lcp.p50, 2000.0);
    assert_eq!(report.lcp.p75, 3000.0);
    assert_eq!(report.lcp.good_pct, 50.0);
}

#[tokio::test]
async fn tier4_scroll_engagement_per_session_max() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    backend
        .ingest_events(vec![
            custom_event(
                site_id,
                now,
                "__scroll__",
                sid(1),
                "/blog",
                serde_json::json!({"depth":25,"url":"/blog"}),
            ),
            custom_event(
                site_id,
                now,
                "__scroll__",
                sid(1),
                "/blog",
                serde_json::json!({"depth":75,"url":"/blog"}),
            ),
            custom_event(
                site_id,
                now,
                "__scroll__",
                sid(2),
                "/blog",
                serde_json::json!({"depth":25,"url":"/blog"}),
            ),
        ])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let rows = fetch_props(&backend, site_id, &["__scroll__"]).await;
    let report = ScrollReport::from_rows(&rows, Some("/blog"));
    assert_eq!(report.sessions_with_scroll_data, 2);
    assert_eq!(report.reached_25pct, 100.0);
    assert_eq!(report.reached_75pct, 50.0);
    assert_eq!(report.reached_100pct, 0.0);

    let heatmap = ScrollHeatmap::from_rows(&rows, "/blog");
    assert_eq!(heatmap.sessions, 2);
    assert_eq!(heatmap.scroll_distribution[0].reached_pct, 100.0);
}

#[tokio::test]
async fn tier4_revenue_summary_and_breakdown() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    backend
        .ingest_events(vec![
            custom_event(
                site_id,
                now,
                "purchase",
                sid(1),
                "/checkout",
                serde_json::json!({"revenue":50.0,"order_id":"o1"}),
            ),
            // Duplicate order — must not double-count.
            custom_event(
                site_id,
                now,
                "purchase",
                sid(1),
                "/checkout",
                serde_json::json!({"revenue":50.0,"order_id":"o1"}),
            ),
            custom_event(
                site_id,
                now,
                "purchase",
                sid(2),
                "/checkout",
                serde_json::json!({"revenue":30.0,"order_id":"o2"}),
            ),
        ])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let rows = fetch_props(&backend, site_id, &[]).await;
    let summary = RevenueSummary::from_rows(&rows, 10, "USD");
    assert_eq!(summary.orders, 2);
    assert_eq!(summary.total_revenue, 80.0);
    assert_eq!(summary.aov, 40.0);
    assert_eq!(summary.revenue_per_session, 8.0);

    let breakdown = RevenueBreakdown::from_rows(&rows, RevenueDimension::Country, "USD", 10);
    assert_eq!(breakdown.rows[0].value, "US");
    assert_eq!(breakdown.rows[0].revenue, 80.0);
}

#[tokio::test]
async fn tier4_experiment_conversion_winner() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    let mut events = vec![];
    // Variant A: 2 exposures, 0 conversions; Variant B: 2 exposures, 2 conversions.
    for i in 0..2u8 {
        let s = sid(10 + i);
        events.push(custom_event(
            site_id,
            now,
            "experiment_viewed",
            s,
            "/",
            serde_json::json!({"experiment":"cta","variant":"A"}),
        ));
    }
    for i in 0..2u8 {
        let s = sid(20 + i);
        events.push(custom_event(
            site_id,
            now,
            "experiment_viewed",
            s,
            "/",
            serde_json::json!({"experiment":"cta","variant":"B"}),
        ));
        events.push(custom_event(
            site_id,
            now + chrono::Duration::seconds(5),
            "signup",
            s,
            "/",
            serde_json::json!({}),
        ));
    }
    backend.ingest_events(events).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let rows = fetch_props(&backend, site_id, &[]).await;
    let list = ExperimentList::from_rows(&rows);
    assert_eq!(list.experiments.len(), 1);
    assert_eq!(list.experiments[0].variants, vec!["A", "B"]);

    let result = ExperimentResult::from_rows(&rows, "cta", Some("signup"));
    let a = result.variants.iter().find(|v| v.variant == "A").unwrap();
    let b = result.variants.iter().find(|v| v.variant == "B").unwrap();
    assert_eq!(a.conversions, 0);
    assert_eq!(b.conversions, 2);
    assert_eq!(result.winner.as_deref(), Some("B"));
}

#[tokio::test]
async fn tier4_search_and_click_heatmap() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    backend
        .ingest_events(vec![
            custom_event(
                site_id,
                now,
                "__search__",
                sid(1),
                "/search",
                serde_json::json!({"query":"pricing"}),
            ),
            custom_event(
                site_id,
                now,
                "__search__",
                sid(2),
                "/search",
                serde_json::json!({"query":"pricing"}),
            ),
            custom_event(
                site_id,
                now,
                "__search__",
                sid(3),
                "/search",
                serde_json::json!({"query":"docs"}),
            ),
            custom_event(
                site_id,
                now,
                "__click__",
                sid(1),
                "/p",
                serde_json::json!({"x":50,"y":30,"url":"/p","element":"BUTTON#buy"}),
            ),
            custom_event(
                site_id,
                now,
                "__click__",
                sid(2),
                "/p",
                serde_json::json!({"x":51,"y":31,"url":"/p","element":"BUTTON#buy"}),
            ),
        ])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let search_rows = fetch_props(&backend, site_id, &["__search__"]).await;
    let search = SearchReport::from_rows(&search_rows, 10);
    assert_eq!(search.total_searches, 3);
    assert_eq!(search.rows[0].query, "pricing");
    assert_eq!(search.rows[0].count, 2);

    let click_rows = fetch_props(&backend, site_id, &["__click__"]).await;
    let heatmap = ClickHeatmap::from_rows(&click_rows, "/p", 2, 1);
    assert_eq!(heatmap.total_clicks, 2);
    assert_eq!(heatmap.elements[0].element, "BUTTON#buy");
    assert_eq!(heatmap.elements[0].count, 2);
}
