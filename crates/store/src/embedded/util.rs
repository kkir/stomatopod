use ulid::Ulid;

/// Encode a ULID into a caller-provided 26-byte buffer using Crockford base32.
/// Returns a `&str` view into the buffer so the caller can pass it straight
/// into a `StringBuilder` without an intermediate `String` allocation.
///
/// This is in the per-record Arrow build hot path; replacing the
/// `Ulid::to_string()` calls with this shaved a measurable allocation per
/// event - see `crates/store/benches/record_batch.rs`.
pub fn ulid_to_str(u: Ulid, buf: &mut [u8; 26]) -> &str {
    const ALPHA: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let n: u128 = u.0;
    for (i, slot) in buf.iter_mut().enumerate() {
        let shift = (25 - i) * 5;
        *slot = ALPHA[((n >> shift) & 0x1F) as usize];
    }
    // SAFETY: every byte written is from ALPHA, which is ASCII.
    unsafe { std::str::from_utf8_unchecked(buf) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_to_str_matches_display() {
        let u = Ulid::new();
        let mut buf = [0u8; 26];
        assert_eq!(ulid_to_str(u, &mut buf), u.to_string());
    }

    #[test]
    fn ulid_to_str_known_value() {
        let u = Ulid::from_string("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
        let mut buf = [0u8; 26];
        assert_eq!(ulid_to_str(u, &mut buf), "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    }
}
