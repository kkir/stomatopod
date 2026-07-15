use chrono::{DateTime, Utc};

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

/// UTC calendar day as days since Unix epoch. Used as the session-day key so
/// events bucket with their event time, not receive time.
pub fn utc_day(dt: DateTime<Utc>) -> u32 {
    (dt.timestamp() / 86400) as u32
}

pub fn current_utc_day() -> u32 {
    utc_day(Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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

    #[test]
    fn utc_day_matches_calendar_day_boundary() {
        // 1970-01-01 00:00 UTC → day 0; 1970-01-02 00:00 UTC → day 1.
        let d0 = Utc.timestamp_opt(0, 0).unwrap();
        let d1 = Utc.timestamp_opt(86_400, 0).unwrap();
        assert_eq!(utc_day(d0), 0);
        assert_eq!(utc_day(d1), 1);
        // Late in day 0 still day 0.
        let almost = Utc.timestamp_opt(86_399, 0).unwrap();
        assert_eq!(utc_day(almost), 0);
    }

    #[test]
    fn historical_event_day_differs_from_today() {
        let historical = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let today = Utc::now();
        // Guard: only assert when "now" is not that same calendar day.
        if utc_day(historical) != utc_day(today) {
            assert_ne!(utc_day(historical), current_utc_day());
        }
    }
}
