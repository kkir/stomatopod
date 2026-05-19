//! Benchmarks for ingest-time hot paths: UTM parsing and User-Agent detection.
//!
//! Both run once per ingest request, so even small per-call savings matter at
//! high request rates. Each input set is a representative mix to keep the
//! medians meaningful.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use stomatopod_ingest::handler::{extract_utm, urlencoding_decode};
use stomatopod_ingest::ua;

const URLS: &[&str] = &[
    // No query string — should be a fast no-op
    "https://example.com/",
    "https://example.com/path/to/page",
    // Single utm param, no encoding
    "https://example.com/?utm_source=google",
    // Full utm set, no encoding
    "https://example.com/?utm_source=google&utm_medium=cpc&utm_campaign=spring&utm_term=rust&utm_content=banner",
    // Mixed params, only some are utm
    "https://example.com/?ref=hn&utm_source=newsletter&page=2&utm_medium=email",
    // Encoded values
    "https://example.com/?utm_campaign=hello%20world&utm_term=foo+bar",
    // Long query with many non-utm params
    "https://example.com/?a=1&b=2&c=3&d=4&e=5&f=6&g=7&utm_source=ads&h=8&i=9",
    // Fragment to skip
    "https://example.com/?utm_source=twitter#section",
];

const ENCODED_VALUES: &[&str] = &[
    "google",                       // no encoding
    "hello%20world",                // single %-encode
    "hello+world",                  // plus encoding
    "spring%20sale%202024",         // multiple encodings
    "newsletter-q1-2024-promotion", // long, no encoding
];

const USER_AGENTS: &[&str] = &[
    // Common browsers (not bots — should fail bot check after scanning)
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Linux; Android 13; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0",
    // Bots
    "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
    "Mozilla/5.0 (compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm)",
    "curl/8.4.0",
    "python-requests/2.31.0",
];

fn bench_extract_utm(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract_utm");
    group.throughput(Throughput::Elements(URLS.len() as u64));
    group.bench_function("mixed_urls", |b| {
        b.iter(|| {
            for url in URLS {
                let p = extract_utm(black_box(url));
                black_box(p);
            }
        });
    });
    group.finish();
}

fn bench_urlencoding_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("urlencoding_decode");
    group.throughput(Throughput::Elements(ENCODED_VALUES.len() as u64));
    group.bench_function("mixed_values", |b| {
        b.iter(|| {
            for v in ENCODED_VALUES {
                black_box(urlencoding_decode(black_box(v)));
            }
        });
    });
    group.finish();
}

fn bench_ua_is_bot(c: &mut Criterion) {
    let mut group = c.benchmark_group("ua_is_bot");
    group.throughput(Throughput::Elements(USER_AGENTS.len() as u64));
    group.bench_function("mixed_uas", |b| {
        b.iter(|| {
            for u in USER_AGENTS {
                black_box(ua::is_bot(black_box(u)));
            }
        });
    });
    group.finish();
}

fn bench_ua_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("ua_parse");
    group.throughput(Throughput::Elements(USER_AGENTS.len() as u64));
    group.bench_function("mixed_uas", |b| {
        b.iter(|| {
            for u in USER_AGENTS {
                black_box(ua::parse(black_box(u)));
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_extract_utm,
    bench_urlencoding_decode,
    bench_ua_is_bot,
    bench_ua_parse,
);
criterion_main!(benches);
