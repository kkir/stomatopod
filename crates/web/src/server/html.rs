//! Server-rendered HTML for the login screen. These used to be minijinja
//! templates; they are now plain Rust string builders (the same approach
//! `routes::digest` uses for its public pages), so the crate carries no
//! template engine.
//!
//! The main dashboard is the Dioxus app and is server-rendered by
//! `dioxus-server`; nothing here overlaps with it.

/// Minimal HTML escape for interpolated text (element content and double-quoted
/// attribute values). Covers the five characters that matter in those
/// contexts.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Shared `<head>` contents: charset/viewport, theme color, the Space Grotesk
/// webfont, and the dashboard stylesheet served at `/app.css`.
fn head(title: &str) -> String {
    // `r##"…"##` so the `"#04080b"` colour literal's `"#` doesn't close the
    // raw string early.
    format!(
        r##"<meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{title}</title>
    <meta name="theme-color" content="#04080b" />
    <link rel="preconnect" href="https://fonts.googleapis.com" />
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin />
    <link href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@500;600;700&display=swap" rel="stylesheet" />
    <link rel="stylesheet" href="/app.css" />"##,
        title = escape(title),
    )
}

/// The login screen. Port of the former `login.jinja`; `error` renders the
/// invalid-credentials notice when present.
pub fn login_page(error: Option<&str>) -> String {
    let error_html = match error {
        Some(msg) => format!(r#"<p class="form-error">{}</p>"#, escape(msg)),
        None => String::new(),
    };
    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    {head}
  </head>
  <body class="auth">
    <form method="post" action="/login" class="card auth-card">
      <div class="logo">
        <span class="logo-mark" aria-hidden="true"></span> Stomatopod
      </div>
      <h1>Sign in</h1>
      <p class="auth-subtitle">Welcome back. Your data hasn't moved.</p>
      {error_html}
      <div class="field">
        <label for="email">Email</label>
        <input id="email" type="email" name="email" required autofocus autocomplete="email" />
      </div>
      <div class="field">
        <label for="password">Password</label>
        <input id="password" type="password" name="password" required autocomplete="current-password" />
      </div>
      <button type="submit" class="btn btn-primary btn-block">Sign in</button>
      <div class="auth-foot">Self-hosted, AGPL-3.0.</div>
    </form>
  </body>
</html>"#,
        head = head("Login - Stomatopod"),
    )
}
