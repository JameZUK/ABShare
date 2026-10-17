//! `/v1/media/*` (SPEC §5.2): streams and covers, authenticated by the media cookie (D9) or a
//! bearer token. A book outside the caller's libraries is "not found".

use super::{ApiError, MEDIA_COOKIE, State, cookie, now};
use crate::auth::sessions::Session;
use crate::auth::store;
use crate::library::{Book, allowed};
use crate::media::{MAX_SEGMENTS_PER_RESPONSE, Media, Source, bitrate_for, open_inside};
use abshare_core::ids::is_valid_id;
use abshare_core::range::{self, RangeReply};
use axum::body::Body;
use axum::extract::{FromRequestParts, Path, Query, State as S};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub fn routes() -> Router<State> {
    Router::new()
        .route("/{id}", get(stream))
        .route("/{id}/state", get(state))
        .route("/{id}/file", get(save_file))
        .route("/covers/{id}/{hash}", get(cover))
}

/// The caller, from the media cookie or `Authorization: Bearer`.
pub struct MediaAuthed(pub Session);

impl FromRequestParts<State> for MediaAuthed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &State) -> Result<Self, Self::Rejection> {
        let t = now();
        if let Some(s) = cookie(&parts.headers, MEDIA_COOKIE).and_then(|c| st.sessions.media(&c, t)) {
            return Ok(Self(s));
        }
        let bearer = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        bearer.and_then(|b| st.sessions.access(b, t)).map(Self).ok_or_else(ApiError::unauthenticated)
    }
}

fn media(st: &State) -> Result<&Arc<Media>, ApiError> {
    st.media.as_ref().ok_or_else(ApiError::not_found)
}

/// The book (if the caller may see it) and its audio files, resolved inside its library root.
async fn resolve(st: &State, s: &Session, id: &str) -> Result<(Book, Source), ApiError> {
    resolve_with_perms(st, s, id).await.map(|(b, src, _)| (b, src))
}

async fn resolve_with_perms(st: &State, s: &Session, id: &str) -> Result<(Book, Source, store::Perms), ApiError> {
    if !is_valid_id(id) {
        return Err(ApiError::not_found());
    }
    let m = media(st)?;
    let cat = st.catalog().await?;
    let uid = s.user;
    let perms = st
        .db
        .run(move |c| Ok(store::user_by_id(c, uid)?.map(|u| u.perms)))
        .await?
        .ok_or_else(ApiError::unauthenticated)?;
    let ok = allowed(&cat, &perms.libraries);
    let book = cat
        .by_id
        .get(id)
        .map(|&i| cat.books[i].clone())
        .filter(|b| ok.contains(&b.library_id))
        .ok_or_else(ApiError::not_found)?;
    let lib_name = cat
        .libraries
        .iter()
        .find(|(lid, _)| *lid == book.library_id)
        .map(|(_, n)| n.clone())
        .ok_or_else(ApiError::not_found)?;
    let root = m.roots.get(&lib_name).cloned().ok_or_else(ApiError::not_found)?;
    let bid = book.id.clone();
    let rels: Vec<String> = st
        .db
        .run(move |c| {
            let mut q = c
                .prepare("SELECT rel_path FROM book_part WHERE book_id = ?1 ORDER BY ord")
                .map_err(crate::db::sql_err)?;
            let r = q.query_map([&bid], |r| r.get(0)).map_err(crate::db::sql_err)?;
            r.collect::<Result<_, _>>().map_err(crate::db::sql_err)
        })
        .await?;
    let mut parts = Vec::new();
    for rel in rels {
        let p = root.join(&rel);
        // Re-checked at open time too; this rejects anything that escapes the root early.
        if open_inside(&root, &p).is_err() {
            return Err(ApiError::not_found());
        }
        parts.push(p);
    }
    if parts.is_empty() {
        return Err(ApiError::not_found());
    }
    let src = Source { book_id: book.id.clone(), parts, duration_ms: book.duration_ms, needs_copy: book.needs_copy };
    Ok((book, src, perms))
}

fn base(h: &mut HeaderMap, ctype: &'static str) {
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert("content-disposition", HeaderValue::from_static("inline"));
    h.insert("content-security-policy", HeaderValue::from_static("default-src 'none'; sandbox"));
}

fn hv(s: String) -> HeaderValue {
    HeaderValue::from_str(&s).unwrap_or_else(|_| HeaderValue::from_static("0"))
}

fn unsatisfiable(len: u64) -> Response {
    let mut r = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
    r.headers_mut().insert(header::CONTENT_RANGE, hv(format!("bytes */{len}")));
    r
}

fn unplayable() -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": "unplayable", "message": "This book couldn't be prepared for playback. The library may have a damaged file." }))).into_response()
}

fn preparing() -> Response {
    (
        StatusCode::CONFLICT,
        Json(
            json!({ "error": "preparing", "message": "This book is being prepared for playback. Try again shortly." }),
        ),
    )
        .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamQuery {
    q: Option<String>,
    /// `dl=1`: a download reading the book in order. Data saver then encodes the whole
    /// response in one ffmpeg run before sending it.
    dl: Option<u8>,
}

async fn stream(
    S(st): S<State>,
    MediaAuthed(s): MediaAuthed,
    Path(id): Path<String>,
    Query(q): Query<StreamQuery>,
    req: HeaderMap,
) -> Result<Response, ApiError> {
    let (_, src, perms) = resolve_with_perms(&st, &s, &id).await?;
    if q.dl.is_some() && !perms.can_offline {
        return Err(no_permission("Downloads aren’t enabled for your account."));
    }
    let m = media(&st)?.clone();
    if !m.ready(&src) {
        if m.copy_failed(&src.book_id) {
            return Ok(unplayable());
        }
        m.ensure_copy(src);
        return Ok(preparing());
    }
    let file = m.play_file(&src).ok_or_else(ApiError::not_found)?;
    let range_h = req.get(header::RANGE).and_then(|v| v.to_str().ok());
    let saver = q.q.as_deref().is_some_and(|k| k != "original");
    if saver || src.needs_copy {
        m.mark_used(&src.book_id, !saver);
    }
    match q.q.as_deref().unwrap_or("original") {
        "original" => serve_file(&src, &file, range_h, &m).await,
        k => {
            let kbps: u64 = k.parse().map_err(|_| ApiError::bad("Unknown quality."))?;
            let bitrate = bitrate_for(kbps).ok_or_else(|| ApiError::bad("Unknown quality."))?;
            let layout = m.layout(src.duration_ms, bitrate).ok_or_else(ApiError::not_found)?;
            let len = layout.total_bytes();
            // No Range header is treated as `bytes=0-`: every data-saver answer is capped, so one
            // request can never make the server encode a whole book (review M3, D23).
            let parsed = match range::parse(range_h, len) {
                RangeReply::Full => RangeReply::Partial { start: 0, end: len - 1 },
                other => other,
            };
            let (status, start, end) = match parsed {
                RangeReply::Full => return Ok(unsatisfiable(len)),
                RangeReply::Partial { start, end } => match layout.cap_range(start, end, MAX_SEGMENTS_PER_RESPONSE) {
                    Ok(e) => (StatusCode::PARTIAL_CONTENT, start, e),
                    Err(_) => return Ok(unsatisfiable(len)),
                },
                RangeReply::Unsatisfiable => return Ok(unsatisfiable(len)),
            };
            let segs = layout.segments_for(start, end).map_err(|_| ApiError::bad("range"))?;
            let first = *segs.start();
            if q.dl == Some(1) {
                let count = segs.end() - segs.start() + 1;
                m.fill_span(&file, &src.book_id, &layout, *segs.start(), count).await.map_err(ApiError::internal)?;
                m.read_ahead(file.clone(), src.book_id.clone(), layout, segs.end() + 1, MAX_SEGMENTS_PER_RESPONSE);
            } else if q.dl.is_some() {
                return Err(ApiError::bad("Unknown download flag."));
            }
            let generation = (status == StatusCode::PARTIAL_CONTENT).then(|| m.new_seek(s.device));
            let book = src.book_id.clone();
            let device = s.device;
            let stream =
                futures_util::stream::unfold((m, segs, file, book), move |(m, mut segs, file, book)| async move {
                    let i = segs.next()?;
                    let off = layout.segment_offset(i);
                    let seek = if i == first { generation.map(|g| (device, g)) } else { None };
                    let item = match m.segment(&file, &book, &layout, i, false, seek).await {
                        Ok(bytes) => {
                            let lo = start.saturating_sub(off) as usize;
                            let hi = ((end + 1 - off) as usize).min(bytes.len());
                            m.prefetch(file.clone(), book.clone(), layout, i);
                            if lo > hi {
                                // A cached segment shorter than the layout says (a stale cache):
                                // end the response rather than panic.
                                Err(std::io::Error::other("segment shorter than expected"))
                            } else {
                                Ok(bytes.slice(lo..hi))
                            }
                        }
                        Err(e) => Err(std::io::Error::other(e)),
                    };
                    Some((item, (m, segs, file, book)))
                });
            let mut r = (status, Body::from_stream(stream)).into_response();
            let h = r.headers_mut();
            base(h, "audio/mpeg");
            h.insert(header::CONTENT_LENGTH, hv((end - start + 1).to_string()));
            h.insert(header::ETAG, hv(format!("\"s{kbps}-{len:x}\"")));
            if status == StatusCode::PARTIAL_CONTENT {
                h.insert(header::CONTENT_RANGE, hv(format!("bytes {start}-{end}/{len}")));
            }
            Ok(r)
        }
    }
}

async fn serve_file(src: &Source, file: &PathBuf, range_h: Option<&str>, m: &Media) -> Result<Response, ApiError> {
    let ctype = match file.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("m4b" | "m4a" | "mp4") => "audio/mp4",
        Some("mp3") => "audio/mpeg",
        Some("ogg" | "oga" | "opus") => "audio/ogg",
        Some("flac") => "audio/flac",
        _ => "application/octet-stream",
    };
    let f = if src.needs_copy {
        std::fs::OpenOptions::new().read(true).open(file).map_err(|_| ApiError::not_found())?
    } else {
        let root = m.roots.values().find(|r| file.starts_with(r)).cloned().ok_or_else(ApiError::not_found)?;
        open_inside(&root, file).map_err(|_| ApiError::not_found())?
    };
    let md = f.metadata().map_err(ApiError::internal)?;
    let len = md.len();
    let mtime =
        md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    if len == 0 {
        return Err(ApiError::not_found());
    }
    let (status, start, end) = match range::parse(range_h, len) {
        RangeReply::Full => (StatusCode::OK, 0, len - 1),
        RangeReply::Partial { start, end } => (StatusCode::PARTIAL_CONTENT, start, end),
        RangeReply::Unsatisfiable => return Ok(unsatisfiable(len)),
    };
    let mut f = tokio::fs::File::from_std(f);
    f.seek(std::io::SeekFrom::Start(start)).await.map_err(ApiError::internal)?;
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(f.take(end - start + 1)));
    let mut r = (status, body).into_response();
    let h = r.headers_mut();
    base(h, ctype);
    h.insert(header::CONTENT_LENGTH, hv((end - start + 1).to_string()));
    h.insert(header::ETAG, hv(format!("\"{len:x}-{mtime:x}\"")));
    if status == StatusCode::PARTIAL_CONTENT {
        h.insert(header::CONTENT_RANGE, hv(format!("bytes {start}-{end}/{len}")));
    }
    Ok(r)
}

async fn state(
    S(st): S<State>,
    MediaAuthed(s): MediaAuthed,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (_, src) = resolve(&st, &s, &id).await?;
    let m = media(&st)?.clone();
    let ready = m.ready(&src);
    let failed = !ready && m.copy_failed(&src.book_id);
    if !ready && !failed {
        m.ensure_copy(src);
    }
    Ok(Json(json!({ "ready": ready, "failed": failed })))
}

fn no_permission(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, "not_permitted", message)
}

/// A name for a saved file: the title in plain characters, never a path.
pub fn save_name(title: &str, ext: &str) -> String {
    let mut name: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | ',' | '(' | ')') { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    name.truncate(120);
    let name = name.trim();
    format!("{}.{ext}", if name.is_empty() { "audiobook" } else { name })
}

/// `GET /v1/media/{id}/file`: the book as one file to keep, for accounts allowed to save
/// files (off by default). The original single file, or the playable copy.
async fn save_file(
    S(st): S<State>,
    MediaAuthed(s): MediaAuthed,
    Path(id): Path<String>,
    req: HeaderMap,
) -> Result<Response, ApiError> {
    let (book, src, perms) = resolve_with_perms(&st, &s, &id).await?;
    if !perms.can_save {
        return Err(no_permission("Saving files isn’t enabled for your account."));
    }
    let m = media(&st)?.clone();
    if !m.ready(&src) {
        if m.copy_failed(&src.book_id) {
            return Ok(unplayable());
        }
        m.ensure_copy(src);
        return Ok(preparing());
    }
    let file = m.play_file(&src).ok_or_else(ApiError::not_found)?;
    let range_h = req.get(header::RANGE).and_then(|v| v.to_str().ok());
    let mut r = serve_file(&src, &file, range_h, &m).await?;
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| e.len() <= 4 && e.bytes().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "bin".into());
    let ascii = save_name(&book.title, &ext);
    // RFC 6266: a plain ASCII name for every browser, and the full title in UTF-8.
    let utf8: String = format!("{}.{ext}", book.title.chars().take(120).collect::<String>())
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    r.headers_mut()
        .insert("content-disposition", hv(format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{utf8}")));
    Ok(r)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoverQuery {
    size: Option<u32>,
}

async fn cover(
    S(st): S<State>,
    MediaAuthed(s): MediaAuthed,
    Path((id, hash)): Path<(String, String)>,
    Query(q): Query<CoverQuery>,
) -> Result<Response, ApiError> {
    if !is_valid_id(&id) || hash.len() != 16 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::not_found());
    }
    let size = q.size.unwrap_or(320);
    if !crate::covers::SIZES.contains(&size) {
        return Err(ApiError::not_found());
    }
    let cat = st.catalog().await?;
    let uid = s.user;
    let perms = st
        .db
        .run(move |c| Ok(store::user_by_id(c, uid)?.map(|u| u.perms.libraries)))
        .await?
        .ok_or_else(ApiError::unauthenticated)?;
    let ok = allowed(&cat, &perms);
    let book = cat
        .by_id
        .get(&id)
        .map(|&i| &cat.books[i])
        .filter(|b| ok.contains(&b.library_id))
        .ok_or_else(ApiError::not_found)?;
    if book.cover.as_deref() != Some(hash.as_str()) {
        return Err(ApiError::not_found());
    }
    let m = media(&st)?;
    let bytes = tokio::fs::read(m.data_dir.join("covers").join(&id).join(format!("{hash}-{size}.jpg")))
        .await
        .map_err(|_| ApiError::not_found())?;
    let mut r = bytes.into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
    h.insert("content-security-policy", HeaderValue::from_static("default-src 'none'; sandbox"));
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::save_name;

    #[test]
    fn saved_file_names_are_plain_and_never_paths() {
        assert_eq!(save_name("The Copper Tide", "m4b"), "The Copper Tide.m4b");
        assert_eq!(save_name("../../etc/passwd", "mp3"), "etc passwd.mp3");
        assert_eq!(save_name("Quote\" and\r\nnewline; x=1", "m4b"), "Quote and newline x 1.m4b");
        assert_eq!(save_name("Ünïcödé", "m4b"), "n c d.m4b");
        assert_eq!(save_name("***", "m4b"), "audiobook.m4b");
        assert!(save_name(&"x".repeat(500), "m4b").len() <= 124);
    }
}
