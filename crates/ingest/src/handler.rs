use std::net::SocketAddr;

use axum::http::{HeaderMap, StatusCode};
use chrono::Utc;
use dashmap::DashMap;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;
use ulid::Ulid;

use stomatopod_core::{
    domain::event::{DeviceType, Event, EventKind},
    traits::MetaStore,
};

use crate::{
    geo::{anonymize_ip, extract_ip, GeoLookup},
    session::{derive_session_id, utc_day},
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
    /// When true, honor CF/X-Real-IP/X-Forwarded-For (see auth config).
    pub trust_forwarded_headers: bool,
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
    let raw_ip = extract_ip(headers, peer_addr, ctx.trust_forwarded_headers);
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

    // 4. Timestamp first so session day matches the event time. Using receive
    // time (current_utc_day) collapses backdated beacons - e.g. seed history
    // or delayed sendBeacon - into one mega-session and inflates avg duration.
    let timestamp = payload
        .t
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    // 5. Session ID (cookieless, resets at UTC midnight of the event day)
    let session_id = derive_session_id(
        site_id.to_bytes().as_ref(),
        ip_anon.as_bytes(),
        ua_str.as_bytes(),
        utc_day(timestamp),
    );

    // 6. Geo lookup (non-blocking; MaxMind MMDB is memory-mapped)
    let geo = ctx.geo.lookup(&raw_ip);

    // 7. Parse UTM params from URL
    let utms = extract_utm(&payload.u);

    // 8. Determine event kind
    let kind = if payload.n == "pageview" {
        EventKind::Pageview
    } else {
        EventKind::Custom
    };

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

/// JSON body for server-side custom events posted to `POST /api/v1/ingest`
/// with an ingest API key. Unlike `IngestPayload` (the browser beacon),
/// fields are full-named and there is no device/network context — the
/// caller is a backend, not a browser.
#[derive(Debug, Deserialize)]
pub struct ServerEventPayload {
    /// Custom event name, e.g. "signup". Required.
    pub name: String,
    /// Optional originating URL (used for UTM extraction + page reports).
    pub url: Option<String>,
    /// Custom event properties (JSON object).
    pub properties: Option<serde_json::Value>,
    /// Client timestamp (Unix ms). Falls back to server time if absent.
    pub timestamp: Option<i64>,
    /// Optional referrer.
    pub referrer: Option<String>,
    /// Optional stable session/user identifier. Hashed into the session id
    /// so server events can participate in session analytics and funnels.
    /// Absent → each event is its own session.
    pub session_id: Option<String>,
}

/// Build and enqueue a server-side custom event. The site is resolved from
/// the ingest key by the caller, so no body-borne site key is consulted and
/// none of the browser-specific machinery (IP/UA parsing, bot filtering,
/// geo, cookieless session derivation) applies.
pub async fn handle_server_ingest(
    tx: &mpsc::Sender<Event>,
    site_id: Ulid,
    payload: ServerEventPayload,
) -> StatusCode {
    if payload.name.is_empty() {
        return StatusCode::BAD_REQUEST;
    }

    let timestamp = payload
        .timestamp
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let url = payload.url.unwrap_or_default();
    let utms = extract_utm(&url);

    // Session: hash a caller-supplied id for stability, else a fresh random
    // id per event (each server event is then its own session).
    let session_id: [u8; 16] = match payload.session_id.as_deref() {
        Some(s) if !s.is_empty() => blake3::hash(s.as_bytes()).as_bytes()[..16]
            .try_into()
            .expect("blake3 hash is 32 bytes"),
        _ => Ulid::new().to_bytes()[..16]
            .try_into()
            .expect("ulid is 16 bytes"),
    };

    let event = Event {
        id: Ulid::new(),
        site_id,
        name: payload.name,
        kind: EventKind::Custom,
        timestamp,
        received_at: Utc::now(),
        url,
        referrer: payload.referrer,
        utm_source: utms.source,
        utm_medium: utms.medium,
        utm_campaign: utms.campaign,
        utm_term: utms.term,
        utm_content: utms.content,
        browser: "Server".to_string(),
        browser_version: String::new(),
        os: "Server".to_string(),
        os_version: String::new(),
        device_type: DeviceType::Unknown,
        screen_width: None,
        screen_height: None,
        language: None,
        ip_anonymized: "0.0.0.0".to_string(),
        country_code: None,
        region: None,
        city: None,
        session_id,
        properties: payload.properties,
    };

    match tx.try_send(event) {
        Ok(_) => StatusCode::NO_CONTENT,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => StatusCode::TOO_MANY_REQUESTS,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[doc(hidden)]
pub struct UtmParams {
    pub source: Option<String>,
    pub medium: Option<String>,
    pub campaign: Option<String>,
    pub term: Option<String>,
    pub content: Option<String>,
}

#[doc(hidden)]
pub fn extract_utm(url: &str) -> UtmParams {
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
        // Match key first; only decode the value if this slot is actually a utm field.
        let slot: &mut Option<String> = match key {
            "utm_source" => &mut out.source,
            "utm_medium" => &mut out.medium,
            "utm_campaign" => &mut out.campaign,
            "utm_term" => &mut out.term,
            "utm_content" => &mut out.content,
            _ => continue,
        };
        if let Some(val) = kv.next() {
            *slot = Some(urlencoding_decode(val));
        }
    }
    out
}

#[doc(hidden)]
pub fn urlencoding_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    // Fast path: nothing to decode. Skip the Vec allocation and bytewise loop.
    if memchr::memchr2(b'%', b'+', bytes).is_none() {
        return s.to_owned();
    }
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
    // Valid UTF-8 in the common case (ASCII or properly-encoded UTF-8 bytes);
    // fall back to lossy conversion for the rare invalid sequence.
    String::from_utf8(decoded)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utm_all_params_extracted() {
        let params = extract_utm(
            "https://example.com/?utm_source=google&utm_medium=cpc&utm_campaign=spring&utm_term=rust&utm_content=banner",
        );
        assert_eq!(params.source.as_deref(), Some("google"));
        assert_eq!(params.medium.as_deref(), Some("cpc"));
        assert_eq!(params.campaign.as_deref(), Some("spring"));
        assert_eq!(params.term.as_deref(), Some("rust"));
        assert_eq!(params.content.as_deref(), Some("banner"));
    }

    #[test]
    fn utm_no_query_string_returns_none() {
        let params = extract_utm("https://example.com/page");
        assert!(params.source.is_none());
        assert!(params.medium.is_none());
        assert!(params.campaign.is_none());
        assert!(params.term.is_none());
        assert!(params.content.is_none());
    }

    #[test]
    fn utm_ignores_fragment_params() {
        let params = extract_utm("https://example.com/?utm_source=twitter#utm_medium=wrong");
        assert_eq!(params.source.as_deref(), Some("twitter"));
        assert!(params.medium.is_none());
    }

    #[test]
    fn utm_percent_encoded_values_decoded() {
        let params = extract_utm("https://example.com/?utm_campaign=hello%20world");
        assert_eq!(params.campaign.as_deref(), Some("hello world"));
    }

    #[test]
    fn utm_plus_encoded_space_decoded() {
        let params = extract_utm("https://example.com/?utm_term=hello+world");
        assert_eq!(params.term.as_deref(), Some("hello world"));
    }

    #[test]
    fn utm_partial_percent_encoding_handled() {
        // Incomplete % sequence should be passed through
        let params = extract_utm("https://example.com/?utm_source=a%2");
        assert!(params.source.is_some());
    }

    #[test]
    fn utm_empty_value_still_present() {
        let params = extract_utm("https://example.com/?utm_source=");
        assert_eq!(params.source.as_deref(), Some(""));
    }

    #[test]
    fn utm_non_utm_params_ignored() {
        let params = extract_utm("https://example.com/?foo=bar&baz=qux");
        assert!(params.source.is_none());
    }
}
