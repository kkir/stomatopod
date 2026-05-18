use std::{net::IpAddr, path::Path, str::FromStr};

use maxminddb::{geoip2, Reader};
use tracing::warn;

pub struct GeoLookup {
    reader: Option<Reader<Vec<u8>>>,
}

#[derive(Debug, Default, Clone)]
pub struct GeoInfo {
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
}

impl GeoLookup {
    pub fn new(mmdb_path: Option<&Path>) -> Self {
        let reader = mmdb_path.and_then(|p| {
            Reader::open_readfile(p)
                .map_err(|e| warn!("Could not load GeoIP database from {}: {e}", p.display()))
                .ok()
        });
        Self { reader }
    }

    pub fn lookup(&self, ip: &str) -> GeoInfo {
        let reader = match &self.reader {
            Some(r) => r,
            None => return GeoInfo::default(),
        };

        let addr = match IpAddr::from_str(ip) {
            Ok(a) => a,
            Err(_) => return GeoInfo::default(),
        };

        match reader.lookup::<geoip2::City>(addr) {
            Ok(city) => GeoInfo {
                country_code: city
                    .country
                    .and_then(|c| c.iso_code)
                    .map(|s| s.to_string()),
                region: city
                    .subdivisions
                    .as_deref()
                    .and_then(|s| s.first())
                    .and_then(|s| s.names.as_ref())
                    .and_then(|n| n.get("en"))
                    .map(|s| s.to_string()),
                city: city
                    .city
                    .and_then(|c| c.names)
                    .and_then(|n| n.get("en").copied())
                    .map(|s| s.to_string()),
            },
            Err(_) => GeoInfo::default(),
        }
    }
}

/// Anonymize an IP address:
/// - IPv4: zero the last octet  (1.2.3.4 → 1.2.3.0)
/// - IPv6: zero the last 80 bits, keeping the /48 prefix
pub fn anonymize_ip(ip: &str) -> String {
    match IpAddr::from_str(ip) {
        Ok(IpAddr::V4(v4)) => {
            let [a, b, c, _] = v4.octets();
            format!("{a}.{b}.{c}.0")
        }
        Ok(IpAddr::V6(v6)) => {
            let mut segments = v6.segments();
            // Keep first 3 groups (48 bits), zero the rest
            for seg in &mut segments[3..] {
                *seg = 0;
            }
            std::net::Ipv6Addr::from(segments).to_string()
        }
        Err(_) => ip.to_string(),
    }
}

/// Extract the real client IP from request headers.
/// Precedence: CF-Connecting-IP → X-Real-IP → X-Forwarded-For (first non-private) → TCP source
pub fn extract_ip(headers: &axum::http::HeaderMap, peer_addr: std::net::SocketAddr) -> String {
    // Cloudflare
    if let Some(cf_ip) = headers
        .get("CF-Connecting-IP")
        .and_then(|v| v.to_str().ok())
    {
        return cf_ip.trim().to_string();
    }

    // Nginx / common reverse proxies
    if let Some(real_ip) = headers.get("X-Real-IP").and_then(|v| v.to_str().ok()) {
        return real_ip.trim().to_string();
    }

    // X-Forwarded-For — take the leftmost non-private IP
    if let Some(xff) = headers
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
    {
        for part in xff.split(',') {
            let ip = part.trim();
            if let Ok(addr) = IpAddr::from_str(ip) {
                if !is_private(&addr) {
                    return ip.to_string();
                }
            }
        }
    }

    peer_addr.ip().to_string()
}

fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            matches!((a, b),
                (10, _) | (172, 16..=31) | (192, 168) | (127, _) | (169, 254)
            )
        }
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}
