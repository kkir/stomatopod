//! Benchmarks for the flush hot path: converting a batch of events into an
//! Arrow `RecordBatch` before Parquet write.
//!
//! The function is called on every flush (default: thousands of events at a
//! time), and each row currently triggers several small heap allocations for
//! ULID/enum stringification. Bench at three batch sizes to see how throughput
//! scales with batch size.

use chrono::{TimeZone, Utc};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ulid::Ulid;

use stomatopod_core::domain::event::{DeviceType, Event, EventKind};
use stomatopod_store::embedded::arrow_schema::event_schema;
use stomatopod_store::embedded::writer::events_to_record_batch;

fn make_event(i: usize) -> Event {
    Event {
        id: Ulid::new(),
        site_id: Ulid::from_string("01H8MECVHA9XZBRD2K00000000").unwrap(),
        name: "pageview".to_string(),
        kind: EventKind::Pageview,
        timestamp: Utc.timestamp_opt(1_700_000_000 + i as i64, 0).unwrap(),
        received_at: Utc.timestamp_opt(1_700_000_000 + i as i64, 0).unwrap(),
        url: format!("https://example.com/page/{i}"),
        referrer: Some("https://google.com/".to_string()),
        utm_source: Some("google".to_string()),
        utm_medium: Some("cpc".to_string()),
        utm_campaign: Some("spring".to_string()),
        utm_term: None,
        utm_content: None,
        browser: "Chrome".to_string(),
        browser_version: "120".to_string(),
        os: "Windows".to_string(),
        os_version: "10/11".to_string(),
        device_type: if i.is_multiple_of(4) {
            DeviceType::Mobile
        } else {
            DeviceType::Desktop
        },
        screen_width: Some(1920),
        screen_height: Some(1080),
        language: Some("en-US".to_string()),
        ip_anonymized: "192.168.1.0".to_string(),
        country_code: Some("US".to_string()),
        region: Some("CA".to_string()),
        city: Some("San Francisco".to_string()),
        session_id: [42u8; 16],
        properties: None,
    }
}

fn make_events(n: usize) -> Vec<Event> {
    (0..n).map(make_event).collect()
}

fn bench_events_to_record_batch(c: &mut Criterion) {
    let schema = event_schema();
    let mut group = c.benchmark_group("events_to_record_batch");
    for &size in &[100usize, 1_000, 10_000] {
        let events = make_events(size);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &events, |b, evs| {
            b.iter(|| {
                let batch = events_to_record_batch(black_box(evs), schema.clone()).unwrap();
                black_box(batch);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_events_to_record_batch);
criterion_main!(benches);
