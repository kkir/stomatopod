//! Outbound URL validation for alert sinks (SSRF hardening).
//!
//! Operators configure webhook / Slack destinations; the server then POSTs to
//! those URLs. Without checks, a compromised admin session can probe
//! localhost, cloud metadata, or the internal network.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

/// Validate that `raw` is a safe destination for a server-side POST.
///
/// Rules:
/// - scheme must be `http` or `https`
/// - host must be present and not a known metadata / loopback hostname
/// - every address the host resolves to must be globally routable (not
///   private, loopback, link-local, or otherwise reserved)
pub fn validate_outbound_url(raw: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(raw).map_err(|e| format!("invalid url: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("url scheme must be http or https, got {other}")),
    }
    let host = url
        .host_str()
        .ok_or_else(|| "url is missing a host".to_string())?;

    if is_blocked_hostname(host) {
        return Err(format!(
            "host '{host}' is not allowed as an alert destination"
        ));
    }

    // Literal IP in the host — check without DNS.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public_ip(&ip) {
            return Err(format!("destination IP {ip} is not publicly routable"));
        }
        return Ok(());
    }

    let port = url
        .port_or_known_default()
        .ok_or_else(|| "url is missing a port".to_string())?;
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("failed to resolve host '{host}': {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("host '{host}' resolved to no addresses"));
    }
    for addr in &addrs {
        if !is_public_ip(&addr.ip()) {
            return Err(format!(
                "host '{host}' resolves to non-public address {}",
                addr.ip()
            ));
        }
    }
    Ok(())
}

fn is_blocked_hostname(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    matches!(
        h.as_str(),
        "localhost"
            | "localhost.localdomain"
            | "metadata"
            | "metadata.google.internal"
            | "metadata.goog"
            | "kubernetes.default"
            | "kubernetes.default.svc"
    ) || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".internal")
}

/// True for addresses we allow the server to POST to.
fn is_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            // 0.0.0.0/8, loopback, RFC1918, link-local, carrier-grade NAT,
            // benchmark, multicast, broadcast, IETF protocol assignments, etc.
            if v4.is_unspecified()
                || v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
            {
                return false;
            }
            // 100.64.0.0/10 shared address space (CGNAT)
            if o[0] == 100 && (o[1] & 0xc0) == 64 {
                return false;
            }
            // 169.254.0.0/16 already covered by is_link_local
            // 192.0.0.0/24 IETF protocol assignments (includes some metadata)
            if o[0] == 192 && o[1] == 0 && o[2] == 0 {
                return false;
            }
            // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24 documentation
            if (o[0] == 192 && o[1] == 0 && o[2] == 2)
                || (o[0] == 198 && o[1] == 51 && o[2] == 100)
                || (o[0] == 203 && o[1] == 0 && o[2] == 113)
            {
                return false;
            }
            // 198.18.0.0/15 benchmarking
            if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
                return false;
            }
            true
        }
        IpAddr::V6(v6) => {
            if v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
            {
                return false;
            }
            // IPv4-mapped addresses: check the embedded v4.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(&IpAddr::V4(v4));
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_loopback_literal() {
        assert!(validate_outbound_url("http://127.0.0.1/hook").is_err());
        assert!(validate_outbound_url("http://[::1]/hook").is_err());
    }

    #[test]
    fn rejects_private_literal() {
        assert!(validate_outbound_url("http://10.0.0.5/x").is_err());
        assert!(validate_outbound_url("http://192.168.1.1/x").is_err());
        assert!(validate_outbound_url("http://172.16.0.1/x").is_err());
    }

    #[test]
    fn rejects_metadata_link_local() {
        assert!(validate_outbound_url("http://169.254.169.254/latest").is_err());
    }

    #[test]
    fn rejects_localhost_name() {
        assert!(validate_outbound_url("http://localhost/hook").is_err());
        assert!(validate_outbound_url("http://foo.localhost/hook").is_err());
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(validate_outbound_url("file:///etc/passwd").is_err());
        assert!(validate_outbound_url("gopher://example.com/").is_err());
    }

    #[test]
    fn accepts_public_https_literal() {
        // 8.8.8.8 is Google Public DNS - publicly routable.
        assert!(validate_outbound_url("https://8.8.8.8/").is_ok());
    }

    #[test]
    fn accepts_well_known_public_host() {
        // Requires network DNS in CI; if resolution fails the test still
        // proves we only accept public resolutions (error is resolve, not
        // private-IP). Prefer a literal for hermetic CI above.
        let result = validate_outbound_url("https://example.com/hook");
        // example.com should resolve publicly in normal environments.
        if let Err(e) = &result {
            // Offline CI: tolerate resolve failure, not a private-IP reject.
            assert!(
                e.contains("failed to resolve") || e.contains("resolved to no"),
                "unexpected error: {e}"
            );
        }
    }
}
