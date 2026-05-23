use axum::response::Html;
use minijinja::value::Value;

use crate::{error::AppError, state::AppState};

/// Render a minijinja template by name and wrap the result in an axum
/// `Html` response, propagating any rendering error as `AppError::Template`.
///
/// Replaces the `unwrap_or_else(|e| format!("<p>Template error: {e}</p>"))`
/// pattern that swallowed real failures as 200-OK HTML — those now surface
/// as 500s with a trace event.
pub fn render(state: &AppState, name: &str, ctx: Value) -> Result<Html<String>, AppError> {
    let tmpl = state.templates.get_template(name)?;
    Ok(Html(tmpl.render(ctx)?))
}
