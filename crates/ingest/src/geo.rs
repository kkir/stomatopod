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
                country_code: city.country.and_then(|c| c.iso_code).map(|s| s.to_string()),
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
    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
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
            matches!(
                (a, b),
                (10, _) | (172, 16..=31) | (192, 168) | (127, _) | (169, 254)
            )
        }
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use std::net::SocketAddr;

    #[test]
    fn anonymize_ipv4_zeros_last_octet() {
        assert_eq!(anonymize_ip("192.168.1.42"), "192.168.1.0");
        assert_eq!(anonymize_ip("10.0.0.255"), "10.0.0.0");
        assert_eq!(anonymize_ip("8.8.8.8"), "8.8.8.0");
    }

    #[test]
    fn anonymize_ipv6_zeros_last_80_bits() {
        let result = anonymize_ip("2001:db8:1234:5678:9abc:def0:1234:5678");
        let addr: std::net::Ipv6Addr = result.parse().unwrap();
        let segs = addr.segments();
        // First 3 groups (48 bits) preserved, rest zeroed
        assert_eq!(segs[0], 0x2001);
        assert_eq!(segs[1], 0x0db8);
        assert_eq!(segs[2], 0x1234);
        assert_eq!(segs[3], 0);
        assert_eq!(segs[7], 0);
    }

    #[test]
    fn anonymize_invalid_ip_returns_original() {
        assert_eq!(anonymize_ip("not-an-ip"), "not-an-ip");
        assert_eq!(anonymize_ip(""), "");
    }

    #[test]
    fn extract_ip_prefers_cloudflare_header() {
        let mut headers = HeaderMap::new();
        headers.insert("CF-Connecting-IP", "1.2.3.4".parse().unwrap());
        headers.insert("X-Real-IP", "5.6.7.8".parse().unwrap());
        let peer = SocketAddr::from(([127, 0, 0, 1], 1234));
        assert_eq!(extract_ip(&headers, peer), "1.2.3.4");
    }

    #[test]
    fn extract_ip_falls_back_to_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Real-IP", "5.6.7.8".parse().unwrap());
        let peer = SocketAddr::from(([127, 0, 0, 1], 1234));
        assert_eq!(extract_ip(&headers, peer), "5.6.7.8");
    }

    #[test]
    fn extract_ip_uses_first_public_xff_addr() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Forwarded-For",
            "10.0.0.1, 172.16.0.1, 203.0.113.42".parse().unwrap(),
        );
        let peer = SocketAddr::from(([127, 0, 0, 1], 1234));
        // 10.0.0.1 and 172.16.0.1 are private, 203.0.113.42 is public
        assert_eq!(extract_ip(&headers, peer), "203.0.113.42");
    }

    #[test]
    fn extract_ip_falls_back_to_peer_addr() {
        let headers = HeaderMap::new();
        let peer = SocketAddr::from(([203, 0, 113, 1], 1234));
        assert_eq!(extract_ip(&headers, peer), "203.0.113.1");
    }

    #[test]
    fn geo_lookup_without_db_returns_empty() {
        let geo = GeoLookup::new(None);
        let info = geo.lookup("8.8.8.8");
        assert!(info.country_code.is_none());
        assert!(info.region.is_none());
        assert!(info.city.is_none());
    }

    #[test]
    fn geo_lookup_with_invalid_ip_returns_empty() {
        let geo = GeoLookup::new(None);
        let info = geo.lookup("not-an-ip");
        assert!(info.country_code.is_none());
    }
}
