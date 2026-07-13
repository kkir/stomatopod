//! `DashQuery`, the shared query-string struct spread into routes via the
//! `?:..q` catch-all syntax (see `crates/ui/src/routes.rs`). It mirrors the
//! range/from/to/compare/filter params the server's `DashQuery` extractor
//! reads in `crates/web/src/extractors.rs`.
//!
//! `FromQuery` is implemented for any type with `From<&str>` (dioxus-router
//! 0.7 blanket impl), so `DashQuery` only needs `From<&str>` + `Display`.
//! The router hands `from_query` the raw, still percent-encoded query
//! string, so decoding happens here; `Display` is percent-encoded again by
//! the router's own href-building code, but only for characters outside
//! its safe set (space, quotes, `#`, `<`, `>`), so the `&`/`=` separators
//! and our own `%XX` escapes survive untouched.

use std::fmt;

#[derive(Clone, PartialEq, Debug, Default)]
pub struct DashQuery {
    pub range: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub compare: bool,
    pub filters: Vec<String>,
}

impl DashQuery {
    /// A copy of this query with the preset range swapped in and any
    /// custom from/to cleared (a preset range and a custom range are
    /// mutually exclusive on the server side too).
    pub fn with_range(&self, range: &str) -> Self {
        Self {
            range: Some(range.to_string()),
            from: None,
            to: None,
            compare: self.compare,
            filters: self.filters.clone(),
        }
    }

    /// A copy of this query with a custom inclusive from/to window.
    /// Clears the preset `range` so the server uses the custom dates.
    pub fn with_custom_range(&self, from: &str, to: &str) -> Self {
        Self {
            range: None,
            from: Some(from.to_string()),
            to: Some(to.to_string()),
            compare: self.compare,
            filters: self.filters.clone(),
        }
    }

    /// True when a custom from/to window is set (overrides preset range).
    pub fn is_custom_range(&self) -> bool {
        self.from.as_ref().is_some_and(|s| !s.is_empty())
            && self.to.as_ref().is_some_and(|s| !s.is_empty())
    }

    /// A copy of this query with one filter token removed (used by
    /// `FilterPill`'s remove button).
    pub fn without_filter(&self, filter: &str) -> Self {
        Self {
            filters: self
                .filters
                .iter()
                .filter(|f| f.as_str() != filter)
                .cloned()
                .collect(),
            ..self.clone()
        }
    }
}

/// Turn a raw filter token (`field:op:value`) into a short human label
/// for pills, e.g. `country:eq:US` → `Country is US`.
pub fn humanize_filter(token: &str) -> String {
    let mut parts = token.splitn(3, ':');
    let field = parts.next().unwrap_or(token);
    let op = parts.next().unwrap_or("eq");
    let value = parts.next().unwrap_or("");

    let field_label = match field {
        "url" => "Page",
        "referrer" => "Referrer",
        "country" => "Country",
        "region" => "Region",
        "browser" => "Browser",
        "os" => "OS",
        "device_type" => "Device",
        "utm_source" => "UTM source",
        "utm_medium" => "UTM medium",
        "utm_campaign" => "UTM campaign",
        "utm_term" => "UTM term",
        "utm_content" => "UTM content",
        "event_name" => "Event",
        other => other,
    };
    let op_label = match op {
        "eq" => "is",
        "not_eq" => "is not",
        "contains" => "contains",
        "starts_with" => "starts with",
        other => other,
    };
    if value.is_empty() {
        format!("{field_label} {op_label}")
    } else {
        format!("{field_label} {op_label} {value}")
    }
}

impl From<&str> for DashQuery {
    fn from(query: &str) -> Self {
        let mut q = DashQuery::default();
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let key = percent_decode(key);
            let value = percent_decode(value);
            match key.as_str() {
                "range" if !value.is_empty() => q.range = Some(value),
                "from" if !value.is_empty() => q.from = Some(value),
                "to" if !value.is_empty() => q.to = Some(value),
                "compare" => q.compare = matches!(value.as_str(), "1" | "true" | "on"),
                "filter" if !value.is_empty() => q.filters.push(value),
                _ => {}
            }
        }
        q
    }
}

impl fmt::Display for DashQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(range) = &self.range {
            parts.push(format!("range={}", percent_encode(range)));
        }
        if let Some(from) = &self.from {
            parts.push(format!("from={}", percent_encode(from)));
        }
        if let Some(to) = &self.to {
            parts.push(format!("to={}", percent_encode(to)));
        }
        if self.compare {
            parts.push("compare=1".to_string());
        }
        for filter in &self.filters {
            parts.push(format!("filter={}", percent_encode(filter)));
        }
        write!(f, "{}", parts.join("&"))
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                Some(byte) => {
                    out.push(byte);
                    i += 3;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_range_and_filters() {
        let q = DashQuery {
            range: Some("30d".to_string()),
            from: None,
            to: None,
            compare: true,
            filters: vec!["browser:eq:Firefox".to_string(), "page:eq:/a b".to_string()],
        };
        let encoded = q.to_string();
        let decoded = DashQuery::from(encoded.as_str());
        assert_eq!(decoded, q);
    }

    #[test]
    fn parses_empty_query() {
        assert_eq!(DashQuery::from(""), DashQuery::default());
    }

    #[test]
    fn humanizes_filter_tokens() {
        assert_eq!(humanize_filter("country:eq:US"), "Country is US");
        assert_eq!(
            humanize_filter("url:starts_with:/blog"),
            "Page starts with /blog"
        );
        assert_eq!(humanize_filter("browser:not_eq:IE"), "Browser is not IE");
    }
}
