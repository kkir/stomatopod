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
