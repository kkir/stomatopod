//! Client-side timestamp formatting in the browser's local timezone.
//!
//! Chart buckets and API timestamps are UTC. Labels convert with
//! `Intl.DateTimeFormat` so the dashboard matches the viewer's local clock.

use chrono::{DateTime, Utc};

/// How densely to label a timeseries point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TsLabelStyle {
    /// Day-or-coarser buckets: `Jan 5` / `01/05`.
    Day,
    /// Hourly buckets: `Jan 5, 3pm` style.
    Hour,
}

/// Format a UTC instant for chart/tooltips in `timezone` (IANA name).
///
/// On wasm, uses `Intl.DateTimeFormat` so DST and IANA names work without
/// shipping chrono-tz. On native (SSR), falls back to UTC formatting.
pub fn format_ts(ts: DateTime<Utc>, timezone: &str, style: TsLabelStyle) -> String {
    let tz = if timezone.trim().is_empty() {
        "UTC"
    } else {
        timezone.trim()
    };

    #[cfg(target_arch = "wasm32")]
    {
        if let Some(s) = format_ts_js(ts, tz, style) {
            return s;
        }
    }

    // SSR / JS failure: UTC calendar labels; live client uses Intl.
    let _ = tz;
    match style {
        TsLabelStyle::Day => ts.format("%b %e").to_string().replace("  ", " "),
        TsLabelStyle::Hour => ts
            .format("%b %e, %H:%M")
            .to_string()
            .replace("  ", " "),
    }
}

/// Pick a label style from bucket spacing (hour vs day+).
pub fn label_style_for_buckets(buckets: &[DateTime<Utc>]) -> TsLabelStyle {
    if buckets.len() < 2 {
        return TsLabelStyle::Day;
    }
    let delta = buckets[1] - buckets[0];
    if delta.num_hours() <= 2 && delta.num_minutes() > 0 {
        TsLabelStyle::Hour
    } else {
        TsLabelStyle::Day
    }
}

/// Short display name for a timezone (last path segment, or the whole string).
pub fn timezone_short_label(tz: &str) -> String {
    tz.rsplit('/').next().unwrap_or(tz).replace('_', " ")
}

/// Browser-resolved IANA timezone, or `"UTC"` when unavailable.
pub fn browser_timezone() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        detect_browser_timezone().unwrap_or_else(|| "UTC".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        "UTC".into()
    }
}

#[cfg(target_arch = "wasm32")]
fn detect_browser_timezone() -> Option<String> {
    let intl = js_sys::Reflect::get(&js_sys::global(), &wasm_bindgen::JsValue::from_str("Intl")).ok()?;
    let dtf_ctor = js_sys::Reflect::get(&intl, &wasm_bindgen::JsValue::from_str("DateTimeFormat")).ok()?;
    let dtf = js_sys::Reflect::construct(&dtf_ctor.into(), &js_sys::Array::new()).ok()?;
    let resolved = js_sys::Reflect::apply(
        &js_sys::Reflect::get(&dtf, &wasm_bindgen::JsValue::from_str("resolvedOptions"))
            .ok()?
            .into(),
        &dtf,
        &js_sys::Array::new(),
    )
    .ok()?;
    let tz = js_sys::Reflect::get(&resolved, &wasm_bindgen::JsValue::from_str("timeZone")).ok()?;
    tz.as_string().filter(|s| !s.is_empty())
}

#[cfg(target_arch = "wasm32")]
fn format_ts_js(ts: DateTime<Utc>, timezone: &str, style: TsLabelStyle) -> Option<String> {
    use wasm_bindgen::JsValue;

    let ms = ts.timestamp_millis() as f64;
    let date = js_sys::Date::new(&JsValue::from_f64(ms));

    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &options,
        &JsValue::from_str("timeZone"),
        &JsValue::from_str(timezone),
    );
    let _ = js_sys::Reflect::set(
        &options,
        &JsValue::from_str("month"),
        &JsValue::from_str("short"),
    );
    let _ = js_sys::Reflect::set(
        &options,
        &JsValue::from_str("day"),
        &JsValue::from_str("numeric"),
    );
    if style == TsLabelStyle::Hour {
        let _ = js_sys::Reflect::set(
            &options,
            &JsValue::from_str("hour"),
            &JsValue::from_str("numeric"),
        );
        let _ = js_sys::Reflect::set(
            &options,
            &JsValue::from_str("minute"),
            &JsValue::from_str("2-digit"),
        );
    }

    // Prefer Intl for timeZone support; fall back to toLocaleString.
    let intl = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("Intl")).ok()?;
    let dtf_ctor =
        js_sys::Reflect::get(&intl, &JsValue::from_str("DateTimeFormat")).ok()?;
    let args = js_sys::Array::new();
    args.push(&JsValue::UNDEFINED);
    args.push(&options);
    let dtf = js_sys::Reflect::construct(&dtf_ctor.into(), &args).ok()?;
    let format_fn =
        js_sys::Reflect::get(&dtf, &JsValue::from_str("format")).ok()?;
    let call_args = js_sys::Array::new();
    call_args.push(&date);
    let formatted = js_sys::Reflect::apply(&format_fn.into(), &dtf, &call_args).ok()?;
    let s = formatted.as_string()?;
    if s.is_empty() {
        return None;
    }
    Some(s)
}

