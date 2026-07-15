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
        allow_ephemeral: true,
        ..Default::default()
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
        allow_ephemeral: true,
        ..Default::default()
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

// ---- Tier-2 analytics: entry/exit, export ----

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
async fn prune_events_before_removes_old_partitions() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let old = Utc::now() - chrono::Duration::days(40);
    let recent = Utc::now() - chrono::Duration::hours(1);
    let mut e_old = make_event(site_id, "/old");
    e_old.timestamp = old;
    e_old.received_at = old;
    let mut e_new = make_event(site_id, "/new");
    e_new.timestamp = recent;
    e_new.received_at = recent;
    backend.ingest_events(vec![e_old, e_new]).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let cutoff = Utc::now() - chrono::Duration::days(30);
    let removed = backend.prune_events_before(cutoff).await.unwrap();
    assert!(removed >= 1, "expected at least one old partition removed");

    let range = TimeRange {
        start: Utc::now() - chrono::Duration::days(60),
        end: Utc::now() + chrono::Duration::hours(1),
    };
    let pv = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range,
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap();
    assert_eq!(
        pv.total_pageviews, 1,
        "only the recent pageview should remain"
    );
}

#[tokio::test]
async fn pageviews_report_bounce_rate_and_avg_duration() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let (site_id, range) = seed_sessions(&backend).await;

    let pv = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range: range.clone(),
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap();

    // 5 pageviews across 3 sessions; only session B is a single-pageview bounce.
    assert_eq!(pv.total_pageviews, 5);
    assert_eq!(
        pv.total_sessions, 3,
        "total_sessions should be unique sessions, not sum of per-bucket counts"
    );
    assert!(
        (pv.bounce_rate - (1.0 / 3.0 * 100.0)).abs() < 0.01,
        "1 of 3 sessions bounced, got {}",
        pv.bounce_rate
    );
    // Session A spans 10s, B 0s, C 5s → mean 5s.
    assert!(
        (pv.avg_duration_secs - 5.0).abs() < 0.01,
        "expected ~5s avg duration, got {}",
        pv.avg_duration_secs
    );
}

/// Regression: if many days of pageviews incorrectly share one session_id
/// (the old ingest used receive-day, not event-day), avg duration balloons
/// to multi-day spans. Distinct per-day sessions keep duration local.
#[tokio::test]
async fn avg_duration_does_not_span_days_when_sessions_are_per_day() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();
    let day0 = now - chrono::Duration::days(3);
    let day1 = now - chrono::Duration::days(2);
    let day2 = now - chrono::Duration::days(1);

    // Three days, each a 30s two-pageview session with its own session_id.
    let mut events = Vec::new();
    for (i, start) in [day0, day1, day2].into_iter().enumerate() {
        let mut a = mk(
            site_id,
            (i + 1) as u8,
            "/home",
            EventKind::Pageview,
            "pageview",
            start,
            None,
        );
        a.session_id = [i as u8 + 1; 16];
        let mut b = mk(
            site_id,
            (i + 1) as u8,
            "/pricing",
            EventKind::Pageview,
            "pageview",
            start + chrono::Duration::seconds(30),
            None,
        );
        b.session_id = [i as u8 + 1; 16];
        events.push(a);
        events.push(b);
    }
    backend.ingest_events(events).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let range = TimeRange {
        start: now - chrono::Duration::days(7),
        end: now + chrono::Duration::hours(1),
    };
    let pv = backend
        .query_pageviews(&PageviewsQuery {
            site_id,
            range,
            granularity: Granularity::Day,
            filters: vec![],
        })
        .await
        .unwrap();

    assert_eq!(pv.total_pageviews, 6);
    assert_eq!(pv.total_sessions, 3);
    assert!(
        (pv.avg_duration_secs - 30.0).abs() < 0.5,
        "per-day sessions should avg ~30s, got {} (would be ~1-2 days if collapsed)",
        pv.avg_duration_secs
    );
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

// ---- P0.2 / P0.3: funnel evaluation + WAL crash recovery ----

#[tokio::test]
async fn funnel_query_counts_step_sessions_and_conversion() {
    use stomatopod_core::query::funnel::{FunnelQuery, FunnelStep};

    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg_bulk(&dir)).await.unwrap();
    let site_id = Ulid::new();
    let now = Utc::now();

    // Session A + B: pageview then signup. Session C: pageview only.
    let mut events = Vec::new();
    for (sess, with_signup) in [([1u8; 16], true), ([2u8; 16], true), ([3u8; 16], false)] {
        let mut pv = make_event(site_id, "/start");
        pv.session_id = sess;
        pv.timestamp = now - chrono::Duration::seconds(10);
        events.push(pv);
        if with_signup {
            let mut su = make_event(site_id, "/thanks");
            su.session_id = sess;
            su.name = "signup".into();
            su.kind = EventKind::Custom;
            su.timestamp = now - chrono::Duration::seconds(5);
            events.push(su);
        }
    }
    backend.ingest_events(events).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let result = backend
        .query_funnel(&FunnelQuery {
            site_id,
            range: TimeRange {
                start: now - chrono::Duration::hours(1),
                end: now + chrono::Duration::minutes(5),
            },
            steps: vec![
                FunnelStep {
                    name: "Landing".into(),
                    event_name: "pageview".into(),
                    filters: vec![],
                },
                FunnelStep {
                    name: "Signup".into(),
                    event_name: "signup".into(),
                    filters: vec![],
                },
            ],
            window_secs: 86_400,
        })
        .await
        .unwrap();

    assert_eq!(result.steps.len(), 2);
    assert_eq!(result.steps[0].sessions, 3);
    assert!((result.steps[0].conversion_rate - 1.0).abs() < f64::EPSILON);
    assert_eq!(result.steps[1].sessions, 2);
    assert!((result.steps[1].conversion_rate - (2.0 / 3.0)).abs() < 0.01);
    assert!((result.steps[1].drop_off_rate - (1.0 / 3.0)).abs() < 0.01);
}

/// Events written to the WAL but not yet flushed to Parquet must survive a
/// process restart (reopen of [`EmbeddedBackend`]).
///
/// The Parquet writer uses `tokio::time::interval`, whose first tick is
/// immediately ready. If events land before that tick is consumed, the
/// writer flushes on the first tick. We sleep briefly after open so the
/// empty first tick is drained before ingesting.
#[tokio::test]
async fn wal_replay_recovers_events_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let site_id = Ulid::new();

    // Phase 1: accept events into WAL + memory buffer; do not flush Parquet.
    {
        let cfg = EmbeddedConfig {
            data_dir: data_dir.clone(),
            wal_fsync_interval_ms: 0,
            parquet_flush_rows: 10_000,
            parquet_flush_interval_s: 3_600,
            allow_ephemeral: true,
            ..Default::default()
        };
        let backend = EmbeddedBackend::open(&cfg).await.unwrap();
        // Drain the writer's immediate first interval tick while the buffer
        // is still empty so ingest does not race into an early flush.
        tokio::time::sleep(Duration::from_millis(50)).await;

        backend
            .ingest_events(vec![
                make_event(site_id, "/wal-a"),
                make_event(site_id, "/wal-b"),
                make_event(site_id, "/wal-c"),
            ])
            .await
            .unwrap();
        // Writer appends to WAL on channel receive; give it a beat.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let parquet_site = data_dir.join("parquet").join(site_id.to_string());
        assert!(
            !parquet_site.exists(),
            "test setup broken: parquet flushed before simulated crash"
        );
        // Simulate crash: drop backend without waiting for interval flush.
        drop(backend);
        // Let the writer task observe the closed channel and exit.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Phase 2: reopen — WAL replay fills the buffer; one interval tick should
    // drain the whole batch (flush_rows large enough for all recovered rows).
    {
        let cfg = EmbeddedConfig {
            data_dir: data_dir.clone(),
            wal_fsync_interval_ms: 0,
            parquet_flush_rows: 10_000,
            parquet_flush_interval_s: 1,
            allow_ephemeral: true,
            ..Default::default()
        };
        let backend = EmbeddedBackend::open(&cfg).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;

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
        assert_eq!(
            result.total_pageviews, 3,
            "WAL replay must restore unflushed events after restart"
        );
    }
}

/// Direct WAL append → reopen → replay into an in-memory buffer.
#[tokio::test]
async fn wal_append_and_replay_round_trip() {
    use stomatopod_store::embedded::{buffer::EventBuffer, wal::Wal};

    let dir = tempfile::tempdir().unwrap();
    let wal_dir = dir.path().join("wal");
    let site_id = Ulid::new();
    let events = vec![make_event(site_id, "/a"), make_event(site_id, "/b")];

    {
        let wal = Wal::open(&wal_dir, 0).unwrap();
        wal.append(&events).unwrap();
        wal.fsync().unwrap();
        drop(wal);
    }

    let wal = Wal::open(&wal_dir, 0).unwrap();
    let buffer = EventBuffer::new(1024);
    wal.replay(&buffer).unwrap();
    assert_eq!(buffer.len(), 2, "replay must restore appended events");
    let drained = buffer.drain(10);
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].url, "/a");
    assert_eq!(drained[1].url, "/b");
}
