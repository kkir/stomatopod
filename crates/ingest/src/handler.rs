use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use dashmap::DashMap;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;
use ulid::Ulid;

use stomatopod_core::{
    domain::event::{Event, EventKind},
    traits::MetaStore,
};

use crate::{
    geo::{anonymize_ip, extract_ip, GeoLookup},
    session::{current_utc_day, derive_session_id},
    ua,
};

/// The JSON body sent by the tracking script.
/// Short field names reduce beacon payload size.
#[derive(Debug, Deserialize)]
pub struct IngestPayload {
    /// Site public key.
    pub k: String,
    /// Event name ("pageview" or custom name).
    pub n: String,
    /// Full URL of the page.
    pub u: String,
    /// Referrer URL.
    pub r: Option<String>,
    /// Screen width in pixels.
    pub w: Option<u16>,
    /// Screen height in pixels.
    pub h: Option<u16>,
    /// Browser language tag.
    pub l: Option<String>,
    /// Custom event properties (JSON object).
    pub p: Option<serde_json::Value>,
    /// Client timestamp (Unix ms). Falls back to server time if absent.
    pub t: Option<i64>,
}

/// Minimal ingest context — extracted from AppState by the router.
pub struct IngestContext {
    pub meta: Arc<dyn MetaStore>,
    pub tx: mpsc::Sender<Event>,
    pub geo: Arc<GeoLookup>,
    pub site_cache: Arc<DashMap<String, Ulid>>,
}

pub async fn handle_ingest_inner(
    ctx: &IngestContext,
    peer_addr: SocketAddr,
    headers: &HeaderMap,
    payload: IngestPayload,
) -> StatusCode {
    // 1. Validate site key (hot path: check cache first)
    let site_id = if let Some(id) = ctx.site_cache.get(&payload.k) {
        *id
    } else {
        match ctx.meta.get_site_by_key(&payload.k).await {
            Ok(Some(site)) => {
                ctx.site_cache.insert(payload.k.clone(), site.id);
                site.id
            }
            Ok(None) => return StatusCode::UNAUTHORIZED,
            Err(e) => {
                warn!("Site key lookup error: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        }
    };

    // 2. Extract and anonymize IP
    let raw_ip = extract_ip(headers, peer_addr);
    let ip_anon = anonymize_ip(&raw_ip);

    // 3. Parse User-Agent
    let ua_str = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ua_info = ua::parse(ua_str);

    // Skip bots
    if ua_info.browser == "Bot" {
        return StatusCode::NO_CONTENT;
    }

    // 4. Session ID (cookieless, resets at UTC midnight)
    let session_id = derive_session_id(
        site_id.to_bytes().as_ref(),
        ip_anon.as_bytes(),
        ua_str.as_bytes(),
        current_utc_day(),
    );

    // 5. Geo lookup (non-blocking; MaxMind MMDB is memory-mapped)
    let geo = ctx.geo.lookup(&raw_ip);

    // 6. Parse UTM params from URL
    let utms = extract_utm(&payload.u);

    // 7. Determine event kind
    let kind = if payload.n == "pageview" {
        EventKind::Pageview
    } else {
        EventKind::Custom
    };

    // 8. Timestamp
    let timestamp = payload
        .t
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let event = Event {
        id: Ulid::new(),
        site_id,
        name: payload.n,
        kind,
        timestamp,
        received_at: Utc::now(),
        url: payload.u,
        referrer: payload.r,
        utm_source: utms.source,
        utm_medium: utms.medium,
        utm_campaign: utms.campaign,
        utm_term: utms.term,
        utm_content: utms.content,
        browser: ua_info.browser,
        browser_version: ua_info.browser_version,
        os: ua_info.os,
        os_version: ua_info.os_version,
        device_type: ua_info.device_type,
        screen_width: payload.w,
        screen_height: payload.h,
        language: payload.l,
        ip_anonymized: ip_anon,
        country_code: geo.country_code,
        region: geo.region,
        city: geo.city,
        session_id,
        properties: payload.p,
    };

    // 9. Non-blocking channel send; 429 on back-pressure
    match ctx.tx.try_send(event) {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => StatusCode::TOO_MANY_REQUESTS,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

struct UtmParams {
    source: Option<String>,
    medium: Option<String>,
    campaign: Option<String>,
    term: Option<String>,
    content: Option<String>,
}

fn extract_utm(url: &str) -> UtmParams {
    let mut out = UtmParams {
        source: None,
        medium: None,
        campaign: None,
        term: None,
        content: None,
    };
    let query = match url.find('?') {
        Some(pos) => &url[pos + 1..],
        None => return out,
    };
    let query = query.split('#').next().unwrap_or(query);

    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("");
        let val = kv.next().map(|v| urlencoding_decode(v));
        match key {
            "utm_source" => out.source = val,
            "utm_medium" => out.medium = val,
            "utm_campaign" => out.campaign = val,
            "utm_term" => out.term = val,
            "utm_content" => out.content = val,
            _ => {}
        }
    }
    out
}

fn urlencoding_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &bytes[i + 1..i + 3];
            if let (Some(hi), Some(lo)) = (from_hex(hex[0]), from_hex(hex[1])) {
                decoded.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            decoded.push(b' ');
            i += 1;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
