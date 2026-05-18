/// Derive a cookieless session ID using BLAKE3.
///
/// Inputs: site_id bytes, anonymized IP, User-Agent string, UTC day
/// (as big-endian u32 days since Unix epoch). Session resets at midnight UTC.
///
/// 128-bit output gives negligible collision probability for any practical scale.
pub fn derive_session_id(
    site_id_bytes: &[u8],
    ip_anon: &[u8],
    ua_bytes: &[u8],
    utc_day: u32,
) -> [u8; 16] {
    let day_bytes = utc_day.to_be_bytes();
    let mut hasher = blake3::Hasher::new();
    hasher.update(site_id_bytes);
    hasher.update(b"|");
    hasher.update(ip_anon);
    hasher.update(b"|");
    hasher.update(ua_bytes);
    hasher.update(b"|");
    hasher.update(&day_bytes);
    let hash = hasher.finalize();
    let bytes = hash.as_bytes();
    let mut out = [0u8; 16];
    out.copy_from_slice(&bytes[..16]);
    out
}

pub fn current_utc_day() -> u32 {
    use chrono::Utc;
    let now = Utc::now();
    (now.timestamp() / 86400) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_deterministic() {
        let id1 = derive_session_id(b"site1", b"1.2.3.0", b"Mozilla/5.0", 19000);
        let id2 = derive_session_id(b"site1", b"1.2.3.0", b"Mozilla/5.0", 19000);
        assert_eq!(id1, id2);
    }

    #[test]
    fn session_id_changes_with_different_day() {
        let id1 = derive_session_id(b"site1", b"1.2.3.0", b"Mozilla/5.0", 19000);
        let id2 = derive_session_id(b"site1", b"1.2.3.0", b"Mozilla/5.0", 19001);
        assert_ne!(id1, id2);
    }

    #[test]
    fn session_id_changes_with_different_ip() {
        let id1 = derive_session_id(b"site1", b"1.2.3.0", b"Mozilla/5.0", 19000);
        let id2 = derive_session_id(b"site1", b"1.2.4.0", b"Mozilla/5.0", 19000);
        assert_ne!(id1, id2);
    }

    #[test]
    fn session_id_changes_with_different_ua() {
        let id1 = derive_session_id(b"site1", b"1.2.3.0", b"Chrome/120", 19000);
        let id2 = derive_session_id(b"site1", b"1.2.3.0", b"Firefox/121", 19000);
        assert_ne!(id1, id2);
    }

    #[test]
    fn session_id_changes_with_different_site() {
        let id1 = derive_session_id(b"site1", b"1.2.3.0", b"Mozilla/5.0", 19000);
        let id2 = derive_session_id(b"site2", b"1.2.3.0", b"Mozilla/5.0", 19000);
        assert_ne!(id1, id2);
    }

    #[test]
    fn session_id_is_16_bytes() {
        let id = derive_session_id(b"site", b"ip", b"ua", 0);
        assert_eq!(id.len(), 16);
    }

    #[test]
    fn empty_inputs_produce_valid_id() {
        let id = derive_session_id(b"", b"", b"", 0);
        assert_eq!(id.len(), 16);
    }
}
