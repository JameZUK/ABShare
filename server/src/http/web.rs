//! Serves the built PWA (SPEC §6) from `web_dir`. Only plain, known-safe paths are served;
//! hashed build output is immutable, everything else is revalidated.

use super::{ApiError, State};
use axum::extract::State as S;
use axum::http::{HeaderValue, Method, Uri, header};
use axum::response::{IntoResponse, Response};

fn ctype(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// A request path as a relative file path, or `None` if it is anything but plain segments.
pub fn clean(path: &str) -> Option<String> {
    let rel = path.strip_prefix('/')?;
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let ok = rel.len() <= 200
        && rel.split('/').all(|seg| {
            !seg.is_empty()
                && !seg.starts_with('.')
                && seg.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        });
    ok.then(|| rel.to_owned())
}

pub async fn fallback(S(st): S<State>, method: Method, uri: Uri) -> Response {
    let not_found = || ApiError::not_found().into_response();
    let Some(dir) = st.web_dir.as_ref() else { return not_found() };
    if method != Method::GET && method != Method::HEAD {
        return not_found();
    }
    if uri.path().starts_with("/v1/") {
        return not_found();
    }
    let Some(rel) = clean(uri.path()) else { return not_found() };
    let Ok(bytes) = tokio::fs::read(dir.join(&rel)).await else { return not_found() };
    let mut r = bytes.into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype(&rel)));
    let cache = if rel.starts_with("_app/immutable/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    r
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn only_plain_paths_are_served() {
        assert_eq!(clean("/").as_deref(), Some("index.html"));
        assert_eq!(clean("/_app/immutable/x-1a.js").as_deref(), Some("_app/immutable/x-1a.js"));
        for bad in
            ["/../etc/passwd", "/a/../b", "/.env", "//x", "/a//b", "/a%2fb", "/a\\b", "x", "/a/", "/.well-known/x"]
        {
            assert_eq!(clean(bad), None, "{bad}");
        }
    }
}
