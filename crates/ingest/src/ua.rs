use stomatopod_core::domain::event::DeviceType;

#[derive(Debug, Clone, Default)]
pub struct UaInfo {
    pub browser: String,
    pub browser_version: String,
    pub os: String,
    pub os_version: String,
    pub device_type: DeviceType,
}

/// Lightweight UA parser — no regex, no database, ~200ns per call.
///
/// Covers >95% of real-world browser traffic. Unknown agents are classified
/// as Desktop/Unknown rather than returning errors.
pub fn parse(ua: &str) -> UaInfo {
    if ua.is_empty() {
        return UaInfo::default();
    }

    // Bots early-exit (no session counted)
    if is_bot(ua) {
        return UaInfo {
            browser: "Bot".into(),
            device_type: DeviceType::Unknown,
            ..Default::default()
        };
    }

    let device_type = detect_device(ua);
    let (browser, browser_version) = detect_browser(ua);
    let (os, os_version) = detect_os(ua);

    UaInfo {
        browser,
        browser_version,
        os,
        os_version,
        device_type,
    }
}

// Markers ordered roughly by frequency of real bot traffic. Any UA that
// contains one of these (case-insensitive) is treated as a bot.
//
// We intentionally don't list "googlebot", "bingbot", "yandexbot",
// "duckduckbot", "facebot", or "uptimerobot" — each contains the generic
// "bot" substring, so the first marker already catches them.
const BOT_MARKERS: &[&str] = &[
    "bot",
    "crawler",
    "spider",
    "headless",
    "curl/",
    "python-requests",
    "go-http-client",
    "java/",
    "wget/",
    "scraper",
    "slurp",
    "baidu",
    "ia_archiver",
    "pingdom",
    "datadog",
    "newrelic",
    "libwww",
];

#[doc(hidden)]
pub fn is_bot(ua: &str) -> bool {
    let lower = ua.to_ascii_lowercase();
    BOT_MARKERS.iter().any(|&m| lower.contains(m))
}

fn detect_device(ua: &str) -> DeviceType {
    // Check tablet before mobile — iPad reports both "iPad" and some "Mobile"
    if ua.contains("iPad") || (ua.contains("Android") && ua.contains("Tablet")) {
        return DeviceType::Tablet;
    }
    if ua.contains("Mobile")
        || ua.contains("iPhone")
        || ua.contains("Android")
        || ua.contains("webOS")
        || ua.contains("BlackBerry")
        || ua.contains("IEMobile")
    {
        return DeviceType::Mobile;
    }
    DeviceType::Desktop
}

fn detect_browser(ua: &str) -> (String, String) {
    // Order matters: check more-specific tokens before generic ones.
    // Edge must come before Chrome (Edge UA contains "Chrome/").
    // Samsung Browser contains "Chrome/" too.
    const PATTERNS: &[(&str, &str)] = &[
        ("Edg/", "Edge"),
        ("OPR/", "Opera"),
        ("Opera/", "Opera"),
        ("SamsungBrowser/", "Samsung Browser"),
        ("Firefox/", "Firefox"),
        ("FxiOS/", "Firefox"),
        ("CriOS/", "Chrome"),
        ("Chrome/", "Chrome"),
        ("Safari/", "Safari"),
        ("MSIE ", "Internet Explorer"),
        ("Trident/", "Internet Explorer"),
    ];

    for (token, name) in PATTERNS {
        if let Some(pos) = ua.find(token) {
            let version = extract_version(ua, pos + token.len());
            return (name.to_string(), version);
        }
    }

    ("Unknown".into(), String::new())
}

fn detect_os(ua: &str) -> (String, String) {
    if ua.contains("Windows NT") {
        let ver = extract_windows_version(ua);
        return ("Windows".into(), ver);
    }
    // iPhone/iPad must be checked before "Mac OS X" — their UAs contain "like Mac OS X"
    if ua.contains("iPhone OS") {
        let ver = extract_after(ua, "iPhone OS ", '_', ' ');
        return ("iOS".into(), ver.replace('_', "."));
    }
    if ua.contains("iPad") {
        let ver = extract_after(ua, "OS ", '_', ' ');
        return ("iPadOS".into(), ver.replace('_', "."));
    }
    if ua.contains("Mac OS X") || ua.contains("macOS") {
        let ver = extract_after(ua, "Mac OS X ", '_', '/');
        return ("macOS".into(), ver.replace('_', "."));
    }
    if ua.contains("Android") {
        let ver = extract_after(ua, "Android ", ';', ' ');
        return ("Android".into(), ver);
    }
    if ua.contains("Linux") {
        return ("Linux".into(), String::new());
    }
    if ua.contains("CrOS") {
        return ("ChromeOS".into(), String::new());
    }
    ("Unknown".into(), String::new())
}

fn extract_version(ua: &str, start: usize) -> String {
    let slice = &ua[start..];
    let end = slice
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(slice.len());
    slice[..end].split('.').next().unwrap_or("").to_string()
}

fn extract_after(ua: &str, prefix: &str, term1: char, term2: char) -> String {
    ua.find(prefix)
        .map(|pos| {
            let start = pos + prefix.len();
            let slice = &ua[start..];
            let end = slice.find([term1, term2]).unwrap_or(slice.len().min(16));
            slice[..end].trim().to_string()
        })
        .unwrap_or_default()
}

fn extract_windows_version(ua: &str) -> String {
    let ver = extract_after(ua, "Windows NT ", ';', ')');
    match ver.as_str() {
        "10.0" => "10/11".into(),
        "6.3" => "8.1".into(),
        "6.2" => "8".into(),
        "6.1" => "7".into(),
        "6.0" => "Vista".into(),
        "5.1" | "5.2" => "XP".into(),
        _ => ver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_desktop() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
        let info = parse(ua);
        assert_eq!(info.browser, "Chrome");
        assert_eq!(info.browser_version, "120");
        assert_eq!(info.os, "Windows");
        assert_eq!(info.device_type, DeviceType::Desktop);
    }

    #[test]
    fn mobile_safari() {
        let ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";
        let info = parse(ua);
        assert_eq!(info.os, "iOS");
        assert_eq!(info.device_type, DeviceType::Mobile);
    }

    #[test]
    fn googlebot_is_bot() {
        let ua = "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";
        let info = parse(ua);
        assert_eq!(info.browser, "Bot");
    }

    #[test]
    fn firefox_linux() {
        let ua = "Mozilla/5.0 (X11; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0";
        let info = parse(ua);
        assert_eq!(info.browser, "Firefox");
        assert_eq!(info.os, "Linux");
    }
}
