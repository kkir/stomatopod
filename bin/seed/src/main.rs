//! Seed a local Stomatopod instance with demo analytics traffic.
//!
//! Requires a running dev server (`mise run dev`). Creates a demo site if
//! needed, then posts pageviews and custom events over a past window so the
//! overview chart, breakdowns, and Events page have something to show.
//!
//! ```text
//! mise run seed
//! STOMATOPOD_PUBLIC_KEY=pk_… cargo run -p stomatopod-seed -- --no-custom
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use chrono::{Datelike, Duration, Timelike, Utc};
use clap::Parser;
use rand::distributions::WeightedIndex;
use rand::prelude::*;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

const PAGES: &[(&str, u32)] = &[
    ("/", 28),
    ("/pricing", 14),
    ("/docs", 12),
    ("/docs/getting-started", 8),
    ("/blog", 7),
    ("/blog/privacy-first-analytics", 5),
    ("/blog/cookieless-sessions", 4),
    ("/about", 4),
    ("/login", 6),
    ("/signup", 5),
    ("/features", 4),
    ("/changelog", 3),
];

const REFERRERS: &[(Option<&str>, u32)] = &[
    (None, 35),
    (Some("https://www.google.com/"), 22),
    (Some("https://news.ycombinator.com/"), 8),
    (Some("https://t.co/demo"), 7),
    (Some("https://www.reddit.com/r/selfhosted/"), 6),
    (Some("https://duckduckgo.com/"), 5),
    (Some("https://github.com/"), 5),
    (Some("https://www.bing.com/"), 4),
    (Some("https://lobste.rs/"), 3),
    (Some("https://www.linkedin.com/"), 3),
    (Some("https://producthunt.com/"), 2),
];

/// Real browser UAs only — the ingest path drops bot markers (`curl/`,
/// `python-requests`, `bot`, …).
const USER_AGENTS: &[(&str, u32)] = &[
    (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
        22,
    ),
    (
        "Mozilla/5.0 (X11; Linux x86_64; rv:123.0) Gecko/20100101 Firefox/123.0",
        12,
    ),
    (
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_3) AppleWebKit/605.1.15 \
         (KHTML, like Gecko) Version/17.3 Safari/605.1.15",
        10,
    ),
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 Edg/122.0.0.0",
        10,
    ),
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36",
        14,
    ),
    (
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_3 like Mac OS X) AppleWebKit/605.1.15 \
         (KHTML, like Gecko) Version/17.3 Mobile/15E148 Safari/604.1",
        12,
    ),
    (
        "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/122.0.0.0 Mobile Safari/537.36",
        10,
    ),
    (
        "Mozilla/5.0 (iPad; CPU OS 17_3 like Mac OS X) AppleWebKit/605.1.15 \
         (KHTML, like Gecko) Version/17.3 Mobile/15E148 Safari/604.1",
        5,
    ),
    (
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:122.0) Gecko/20100101 Firefox/122.0",
        5,
    ),
];

const UTM_CAMPAIGNS: &[(&str, &str, &str)] = &[
    ("twitter", "social", "launch-week"),
    ("newsletter", "email", "march-digest"),
    ("google", "cpc", "brand-search"),
    ("producthunt", "referral", "ph-launch"),
    ("hn", "social", "show-hn"),
];

const CUSTOM_EVENTS: &[(&str, f64)] = &[
    ("signup", 0.04),
    ("cta_click", 0.08),
    ("docs_search", 0.05),
    ("pricing_toggle", 0.03),
    ("newsletter_subscribe", 0.02),
];

const SAMPLE_IPS: &[&str] = &[
    "8.8.8.8",
    "1.1.1.1",
    "9.9.9.9",
    "208.67.222.222",
    "64.6.64.6",
    "94.140.14.14",
    "185.228.168.9",
    "76.76.2.0",
    "149.112.112.112",
    "20.236.44.162",
];

const LANGUAGES: &[&str] = &[
    "en-US", "en-GB", "de-DE", "fr-FR", "es-ES", "ja-JP", "pt-BR",
];

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "stomatopod-seed",
    about = "Seed a running Stomatopod server with demo analytics traffic"
)]
struct Args {
    /// Server base URL.
    #[arg(
        long,
        default_value = "http://localhost:8080",
        env = "STOMATOPOD_SERVER"
    )]
    server: String,

    /// Admin email for dashboard login (skipped when --public-key is set).
    #[arg(
        long,
        default_value = "admin@localhost",
        env = "STOMATOPOD_ADMIN_EMAIL"
    )]
    email: String,

    /// Admin password (required unless --public-key is set).
    #[arg(long, default_value = "", env = "STOMATOPOD_ADMIN_PASSWORD")]
    password: String,

    /// Existing site public tracker key (skip login / site create).
    #[arg(long, default_value = "", env = "STOMATOPOD_PUBLIC_KEY")]
    public_key: String,

    /// Domain for a newly created demo site.
    #[arg(long, default_value = "demo.localhost", env = "STOMATOPOD_SITE_DOMAIN")]
    domain: String,

    /// Name for a newly created demo site.
    #[arg(long, default_value = "Demo Site", env = "STOMATOPOD_SITE_NAME")]
    name: String,

    /// Days of history to generate.
    #[arg(long, default_value_t = 30, env = "SEED_DAYS")]
    days: u32,

    /// Approximate number of pageviews.
    #[arg(long, default_value_t = 2500, env = "SEED_EVENTS")]
    events: u32,

    /// Concurrent POST workers.
    #[arg(long, default_value_t = 16, env = "SEED_WORKERS")]
    workers: usize,

    /// Skip custom events (no ingest key).
    #[arg(long)]
    no_custom: bool,
}

// ---------------------------------------------------------------------------
// Planned events
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct PlannedEvent {
    kind: EventKind,
    name: String,
    url: String,
    referrer: Option<String>,
    ts_ms: i64,
    ua: String,
    ip: String,
    session_tag: String,
    width: u16,
    height: u16,
    language: String,
    properties: Option<Value>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventKind {
    Pageview,
    Custom,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn weighted_pick<'a, T>(items: &'a [(T, u32)], rng: &mut impl Rng) -> &'a T {
    let weights: Vec<u32> = items.iter().map(|(_, w)| *w).collect();
    let dist = WeightedIndex::new(&weights).expect("weights");
    &items[dist.sample(rng)].0
}

fn day_weight(weekday: chrono::Weekday) -> f64 {
    use chrono::Weekday::*;
    match weekday {
        Mon => 0.95,
        Tue => 1.05,
        Wed => 1.10,
        Thu => 1.10,
        Fri => 0.95,
        Sat => 0.55,
        Sun => 0.50,
    }
}

fn screen_for_ua(ua: &str, rng: &mut impl Rng) -> (u16, u16) {
    if ua.contains("iPhone") || ua.contains("Android") || ua.contains("Mobile") {
        *[(390, 844), (412, 915), (360, 800)].choose(rng).unwrap()
    } else if ua.contains("iPad") {
        (1024, 1366)
    } else {
        *[(1920, 1080), (1440, 900), (1536, 864), (2560, 1440)]
            .choose(rng)
            .unwrap()
    }
}

fn plan_events(days: u32, target_pageviews: u32, rng: &mut impl Rng) -> Vec<PlannedEvent> {
    let now = Utc::now();
    let mut day_weights: Vec<(chrono::DateTime<Utc>, f64)> = Vec::with_capacity(days as usize);
    for i in 0..days {
        let d = (now - Duration::days((days - 1 - i) as i64))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        day_weights.push((d, day_weight(d.weekday())));
    }
    let total_w: f64 = day_weights.iter().map(|(_, w)| w).sum();
    let visitor_pool = (target_pageviews / 15).max(40) as usize;

    let mut events = Vec::new();
    for (day_idx, (day, weight)) in day_weights.iter().enumerate() {
        let n = ((target_pageviews as f64) * (weight / total_w))
            .round()
            .max(1.0) as usize;
        let visitors_today = ((n as f64) * rng.gen_range(0.35..0.55)).round().max(5.0) as usize;
        let visitors_today = visitors_today.min(visitor_pool);
        let mut visitor_ids: Vec<usize> = (0..visitor_pool).collect();
        visitor_ids.shuffle(rng);
        visitor_ids.truncate(visitors_today);

        for _ in 0..n {
            let vid = visitor_ids[rng.gen_range(0..visitor_ids.len())];
            let session_tag = format!("v{vid}-d{day_idx}");
            let ua = weighted_pick(USER_AGENTS, rng).to_string();
            let base_ip = SAMPLE_IPS[vid % SAMPLE_IPS.len()];
            let mut parts: Vec<&str> = base_ip.split('.').collect();
            let last = format!("{}", (vid * 7 + day_idx) % 250);
            // Keep owned last octet alive for format
            let last_owned = last;
            parts[3] = &last_owned;
            let ip = parts.join(".");

            let path = *weighted_pick(PAGES, rng);
            let referrer = weighted_pick(REFERRERS, rng).map(|s| s.to_string());

            // Roughly business-hours UTC: average of uniforms centered near 17:00.
            let hour = {
                let s: f64 = (0..6).map(|_| rng.gen::<f64>()).sum::<f64>() / 6.0;
                ((s - 0.5) * 12.0 + 17.0).round().clamp(0.0, 23.0) as u32
            };
            let minute = rng.gen_range(0..60);
            let second = rng.gen_range(0..60);
            let mut ts = day
                .with_hour(hour)
                .and_then(|t| t.with_minute(minute))
                .and_then(|t| t.with_second(second))
                .unwrap_or(*day);
            if ts > now {
                ts = now - Duration::minutes(rng.gen_range(1..90));
            }

            let mut url = format!("https://demo.localhost{path}");
            if rng.gen::<f64>() < 0.18 && matches!(path, "/" | "/pricing" | "/signup") {
                let (src, med, camp) = UTM_CAMPAIGNS[rng.gen_range(0..UTM_CAMPAIGNS.len())];
                url = format!(
                    "{url}?utm_source={src}&utm_medium={med}&utm_campaign={camp}&utm_content=hero"
                );
            }

            let (width, height) = screen_for_ua(&ua, rng);
            let language = LANGUAGES[rng.gen_range(0..LANGUAGES.len())].to_string();
            let ts_ms = ts.timestamp_millis();

            events.push(PlannedEvent {
                kind: EventKind::Pageview,
                name: "pageview".into(),
                url: url.clone(),
                referrer: referrer.clone(),
                ts_ms,
                ua: ua.clone(),
                ip: ip.clone(),
                session_tag: session_tag.clone(),
                width,
                height,
                language: language.clone(),
                properties: None,
            });

            for &(ev_name, rate) in CUSTOM_EVENTS {
                if rng.gen::<f64>() < rate {
                    events.push(PlannedEvent {
                        kind: EventKind::Custom,
                        name: ev_name.into(),
                        url: url.clone(),
                        referrer: referrer.clone(),
                        ts_ms: ts_ms + rng.gen_range(500..15_000),
                        ua: ua.clone(),
                        ip: ip.clone(),
                        session_tag: session_tag.clone(),
                        width,
                        height,
                        language: language.clone(),
                        properties: Some(json!({"seed": true, "path": path})),
                    });
                }
            }
        }
    }

    events.sort_by_key(|e| e.ts_ms);
    events
}

// ---------------------------------------------------------------------------
// HTTP bootstrap
// ---------------------------------------------------------------------------

async fn ensure_reachable(client: &Client, base: &Url) -> Result<()> {
    let url = base.join("/login")?;
    match client.get(url).send().await {
        Ok(resp) if resp.status().as_u16() < 500 => Ok(()),
        Ok(resp) => bail!("server returned {} for /login", resp.status()),
        Err(e) => bail!("cannot reach {base} ({e}). Start the dev server first: mise run dev"),
    }
}

async fn login(client: &Client, base: &Url, email: &str, password: &str) -> Result<()> {
    let url = base.join("/login")?;
    let resp = client
        .post(url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(format!(
            "email={}&password={}",
            urlencoding_form(email),
            urlencoding_form(password)
        ))
        .send()
        .await
        .context("login request")?;

    // Cookie jar is shared; success leaves a session cookie. Probe /api/v1/me.
    let me = client.get(base.join("/api/v1/me")?).send().await?;
    if me.status() == StatusCode::OK {
        println!("  logged in as {email}");
        return Ok(());
    }
    let login_status = resp.status();
    let me_status = me.status();
    let body = me.text().await.unwrap_or_default();
    bail!("login failed (login HTTP {login_status}, /api/v1/me HTTP {me_status}): {body}");
}

fn urlencoding_form(s: &str) -> String {
    // Minimal form-urlencoded for email/password (no spaces expected).
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'@' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Debug, Deserialize)]
struct SitesList {
    #[serde(default)]
    sites: Vec<SiteSummary>,
}

#[derive(Debug, Deserialize)]
struct SiteSummary {
    id: String,
    domain: String,
    name: String,
    public_key: String,
}

async fn resolve_site(
    client: &Client,
    base: &Url,
    public_key: Option<&str>,
    domain: &str,
    name: &str,
) -> Result<(String, Option<String>)> {
    if let Some(pk) = public_key {
        return Ok((pk.to_string(), None));
    }

    let list: SitesList = client
        .get(base.join("/api/v1/sites")?)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    if let Some(site) = list.sites.first() {
        println!("  using existing site {} ({})", site.name, site.domain);
        return Ok((site.public_key.clone(), Some(site.id.clone())));
    }

    let resp = client
        .post(base.join("/api/v1/sites")?)
        .json(&json!({"domain": domain, "name": name}))
        .send()
        .await?
        .error_for_status()?;
    let site: SiteSummary = resp.json().await?;
    println!("  created site {} ({})", site.name, site.domain);
    Ok((site.public_key, Some(site.id)))
}

async fn mint_ingest_key(client: &Client, base: &Url, site_id: &str) -> Result<Option<String>> {
    let resp = client
        .post(base.join("/api/v1/keys")?)
        .json(&json!({
            "name": "dev-seed",
            "scope": "ingest",
            "site_id": site_id,
        }))
        .send()
        .await?;
    if !resp.status().is_success() {
        eprintln!(
            "  warning: could not mint ingest key (HTTP {}); skipping custom events",
            resp.status()
        );
        return Ok(None);
    }
    let body: Value = resp.json().await?;
    let secret = body
        .get("secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if secret.is_none() {
        eprintln!("  warning: create-key response missing secret; skipping custom events");
    } else {
        let prefix = body
            .get("display_prefix")
            .and_then(|v| v.as_str())
            .unwrap_or("sk_…");
        println!("  minted ingest key {prefix}…");
    }
    Ok(secret)
}

// ---------------------------------------------------------------------------
// Posting
// ---------------------------------------------------------------------------

async fn post_pageview(client: &Client, base: &Url, public_key: &str, ev: &PlannedEvent) -> bool {
    let mut body = json!({
        "k": public_key,
        "n": "pageview",
        "u": ev.url,
        "w": ev.width,
        "h": ev.height,
        "l": ev.language,
        "t": ev.ts_ms,
    });
    if let Some(r) = &ev.referrer {
        body["r"] = json!(r);
    }

    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&ev.ua) {
        headers.insert(USER_AGENT, v);
    }
    if let Ok(v) = HeaderValue::from_str(&ev.ip) {
        headers.insert("X-Forwarded-For", v);
    }

    match client
        .post(match base.join("/api/v1/event") {
            Ok(u) => u,
            Err(_) => return false,
        })
        .headers(headers)
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success() || resp.status() == StatusCode::NO_CONTENT,
        Err(_) => false,
    }
}

async fn post_custom(client: &Client, base: &Url, ingest_key: &str, ev: &PlannedEvent) -> bool {
    let mut body = json!({
        "name": ev.name,
        "url": ev.url,
        "timestamp": ev.ts_ms,
        "session_id": ev.session_tag,
    });
    if let Some(r) = &ev.referrer {
        body["referrer"] = json!(r);
    }
    if let Some(p) = &ev.properties {
        body["properties"] = p.clone();
    }

    match client
        .post(match base.join("/api/v1/ingest") {
            Ok(u) => u,
            Err(_) => return false,
        })
        .bearer_auth(ingest_key)
        .header(USER_AGENT, &ev.ua)
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success() || resp.status() == StatusCode::NO_CONTENT,
        Err(_) => false,
    }
}

async fn post_all(
    client: Arc<Client>,
    base: Url,
    events: Vec<PlannedEvent>,
    public_key: Arc<str>,
    ingest_key: Option<Arc<str>>,
    workers: usize,
) -> (usize, usize, usize, usize) {
    let sem = Arc::new(Semaphore::new(workers.max(1)));
    let pv_ok = Arc::new(AtomicUsize::new(0));
    let pv_fail = Arc::new(AtomicUsize::new(0));
    let cu_ok = Arc::new(AtomicUsize::new(0));
    let cu_fail = Arc::new(AtomicUsize::new(0));

    let total_pv = events
        .iter()
        .filter(|e| e.kind == EventKind::Pageview)
        .count();
    let mut set = JoinSet::new();
    let mut submitted_pv = 0usize;

    for ev in events {
        let is_pageview = ev.kind == EventKind::Pageview;
        let permit = sem.clone().acquire_owned().await.expect("semaphore");
        let client = client.clone();
        let base = base.clone();
        let public_key = public_key.clone();
        let ingest_key = ingest_key.clone();
        let pv_ok = pv_ok.clone();
        let pv_fail = pv_fail.clone();
        let cu_ok = cu_ok.clone();
        let cu_fail = cu_fail.clone();

        set.spawn(async move {
            let _permit = permit;
            match ev.kind {
                EventKind::Pageview => {
                    if post_pageview(&client, &base, &public_key, &ev).await {
                        pv_ok.fetch_add(1, Ordering::Relaxed);
                    } else {
                        pv_fail.fetch_add(1, Ordering::Relaxed);
                    }
                }
                EventKind::Custom => {
                    if let Some(key) = ingest_key.as_ref() {
                        if post_custom(&client, &base, key, &ev).await {
                            cu_ok.fetch_add(1, Ordering::Relaxed);
                        } else {
                            cu_fail.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        });

        if is_pageview {
            submitted_pv += 1;
            if submitted_pv.is_multiple_of(250) || submitted_pv == total_pv {
                println!("    queued {submitted_pv}/{total_pv} pageviews…");
            }
        }
    }

    while set.join_next().await.is_some() {}

    (
        pv_ok.load(Ordering::Relaxed),
        pv_fail.load(Ordering::Relaxed),
        cu_ok.load(Ordering::Relaxed),
        cu_fail.load(Ordering::Relaxed),
    )
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.days == 0 || args.events == 0 {
        bail!("--days and --events must be >= 1");
    }

    let base = Url::parse(&args.server).context("invalid --server URL")?;
    let client = Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    println!("Seeding {}", args.server);
    ensure_reachable(&client, &base).await?;

    let public_key_arg = args.public_key.trim();
    let (public_key, site_id) = if public_key_arg.is_empty() {
        if args.password.is_empty() {
            bail!(
                "set STOMATOPOD_ADMIN_PASSWORD (or --password), or set \
                 STOMATOPOD_PUBLIC_KEY to seed an existing site"
            );
        }
        login(&client, &base, &args.email, &args.password).await?;
        resolve_site(&client, &base, None, &args.domain, &args.name).await?
    } else {
        println!(
            "  using public key {}…",
            &public_key_arg[..public_key_arg.len().min(12)]
        );
        resolve_site(
            &client,
            &base,
            Some(public_key_arg),
            &args.domain,
            &args.name,
        )
        .await?
    };

    let ingest_key = if !args.no_custom {
        if let Some(id) = site_id.as_deref() {
            mint_ingest_key(&client, &base, id).await?
        } else {
            None
        }
    } else {
        None
    };

    println!(
        "  planning ~{} pageviews across {} days…",
        args.events, args.days
    );
    let mut rng = rand::thread_rng();
    let planned = plan_events(args.days, args.events, &mut rng);
    let pv_n = planned
        .iter()
        .filter(|e| e.kind == EventKind::Pageview)
        .count();
    let cu_n = planned
        .iter()
        .filter(|e| e.kind == EventKind::Custom)
        .count();
    println!("  {pv_n} pageviews, {cu_n} custom events");

    let t0 = Instant::now();
    println!("  posting…");
    let (pv_ok, pv_fail, cu_ok, cu_fail) = post_all(
        Arc::new(client),
        base,
        planned,
        Arc::from(public_key),
        ingest_key.map(Arc::from),
        args.workers,
    )
    .await;

    let elapsed = t0.elapsed().as_secs_f64();
    println!();
    println!("Done in {elapsed:.1}s");
    println!("  pageviews: {pv_ok} ok, {pv_fail} failed");
    if !args.no_custom {
        println!("  custom:    {cu_ok} ok, {cu_fail} failed");
    }
    println!();
    println!("Open the dashboard Overview — data may take a second or two to flush");
    println!("from the WAL. Range tip: use 30d if you seeded the default window.");

    if pv_fail > 0 || cu_fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}
