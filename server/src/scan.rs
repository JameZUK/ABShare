//! A library scan (SPEC §4.2): walk → probe new or changed files (jailed, a few at a time) →
//! assemble books in `core` → metadata, chapters, IDs, groups → covers → one transaction.

use crate::catalogue::{BookRow, Catalogue, GroupRow, ProblemRow};
use crate::config::{Config, Library};
use crate::covers;
use crate::probe::{self, ProbeRecord};
use crate::secret::{Secret, purpose};
use crate::walk::{self, DirExtras};
use abshare_core::assemble::{IgnoreReason, assemble};
use abshare_core::chapters;
use abshare_core::ids::{BookKey, book_id};
use abshare_core::metadata::{BookMeta, book_meta};
use abshare_core::model::{ProbedFile, Sidecars};
use abshare_core::series::{self, Item, Kind};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Debug, Default)]
pub struct Report {
    pub library: String,
    pub files: usize,
    pub probed: usize,
    pub books: usize,
    pub needs_copy: usize,
    pub missing: usize,
    pub groups: usize,
    pub shows: usize,
    pub covers: usize,
    pub problems: Vec<(String, &'static str, String)>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub async fn scan_library(
    cfg: &Config,
    cat: &mut Catalogue,
    secret: &Secret,
    lib: &Library,
    lib_no: u32,
) -> Result<Report, String> {
    crate::config::check_library_root(&lib.path)?;
    let mut rep = Report { library: lib.name.clone(), ..Report::default() };
    let lib_id = cat.library_id(&lib.name, &lib.path.to_string_lossy())?;
    let root = lib.path.clone();
    let w = tokio::task::spawn_blocking(move || walk::walk(&root)).await.map_err(|e| e.to_string())??;
    rep.files = w.audio.len();
    for (p, why) in &w.skipped {
        rep.problems.push((p.clone(), "skipped", (*why).to_owned()));
    }

    // Probe what is new or changed.
    let stored = cat.files(lib_id)?;
    let todo: Vec<_> = w
        .audio
        .iter()
        .filter(|f| stored.get(&f.rel).is_none_or(|s| s.size != f.size || s.mtime_ns != f.mtime_ns))
        .cloned()
        .collect();
    let sem = Arc::new(Semaphore::new(cfg.ffmpeg.jobs.max(1) * 2));
    let ff = Arc::new(crate::config::Ffmpeg {
        ffmpeg: cfg.ffmpeg.ffmpeg.clone(),
        ffprobe: cfg.ffmpeg.ffprobe.clone(),
        jail: cfg.ffmpeg.jail.clone(),
        libs: cfg.ffmpeg.libs.clone(),
        jobs: cfg.ffmpeg.jobs,
    });
    let mut handles = Vec::new();
    for f in todo {
        let (sem, ff, abs) = (sem.clone(), ff.clone(), lib.path.join(&f.rel));
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.ok();
            let rec = probe::probe(&ff, &abs, &f.ext).await;
            (f, rec)
        }));
    }
    let total = handles.len();
    for (n, h) in handles.into_iter().enumerate() {
        let (f, rec) = h.await.map_err(|e| e.to_string())?;
        cat.upsert_file(lib_id, &f.rel, f.size, f.mtime_ns, &rec, now())?;
        rep.probed += 1;
        if (n + 1) % 25 == 0 || n + 1 == total {
            eprintln!("  {}: probed {}/{total}", lib.name, n + 1);
        }
    }
    let keep: HashSet<String> = w.audio.iter().map(|f| f.rel.clone()).collect();
    cat.forget_files_not_in(lib_id, &keep)?;

    // Assemble from the catalogue's view of every file.
    let records = cat.files(lib_id)?;
    let mut rels: Vec<&String> = records.keys().collect();
    rels.sort();
    let files: Vec<ProbedFile> =
        rels.iter().map(|r| probe::to_probed(r, records[*r].size, &records[*r].probe)).collect();
    let probes: Vec<&ProbeRecord> = rels.iter().map(|r| &records[*r].probe).collect();
    let asm = assemble(&files);

    for ig in &asm.ignored {
        let f = &files[ig.file];
        let detail = match &ig.reason {
            IgnoreReason::Duplicate { kept } => format!("duplicate of {}", files[*kept].file_name()),
            _ => probes[ig.file].problem.clone().unwrap_or_else(|| format!("{:?}", ig.reason)),
        };
        let kind = match ig.reason {
            IgnoreReason::Duplicate { .. } => "duplicate",
            IgnoreReason::XheAac => "unsupported codec",
            _ => "unplayable",
        };
        rep.problems.push((f.rel_path.clone(), kind, detail));
    }

    let id_key = secret.derive(purpose::BOOK_ID);
    let previous_covers = cat.covers(lib_id)?;
    let mut asin_seen: HashSet<String> = HashSet::new();
    // IDs held by books in other libraries: an ASIN can't take one over (review M2).
    let elsewhere = cat.ids_elsewhere(lib_id)?;
    let overrides = cat.overrides()?;
    let mut metas: Vec<BookMeta> = Vec::new();
    let mut rows: Vec<BookRow> = Vec::new();
    for plan in &asm.books {
        let first = &files[plan.parts[0]];
        let folder_path = plan.key.split('#').next().unwrap_or(&plan.key).to_owned();
        let loose = plan.parts.len() == 1 && first.dir().is_empty();
        let folder_name = if loose { None } else { folder_path.rsplit('/').next() };
        let side = sidecars(&lib.path, &w.extras, first.dir());
        let mut meta = book_meta(&first.tags, folder_name, first.file_name(), &side, plan.parts.len() > 1);
        let parts: Vec<&ProbedFile> = plan.parts.iter().map(|&i| &files[i]).collect();
        let chapters = chapters::for_parts(&parts);
        let path_id = || book_id(&id_key, &BookKey::Path { library: lib_no, folder: &plan.key });
        let id = match meta.asin.as_deref() {
            Some(a) if asin_seen.insert(a.to_owned()) => {
                let by_asin = book_id(&id_key, &BookKey::Asin(a));
                if elsewhere.contains(&by_asin) {
                    rep.problems.push((
                        plan.key.clone(),
                        "duplicate ASIN",
                        format!("{a} is already used by a book in another library"),
                    ));
                    path_id()
                } else {
                    by_asin
                }
            }
            Some(a) => {
                rep.problems.push((plan.key.clone(), "duplicate ASIN", format!("{a} is also used by another folder")));
                path_id()
            }
            None => path_id(),
        };
        // An admin's corrections replace what the tags say (and so regroup series).
        if let Some(o) = overrides.get(&id) {
            o.apply(&mut meta);
        }
        // Cover: embedded first, then cover.*/folder.*, then a single image in the folder.
        let src = if first.has_cover {
            Some(covers::Source::Embedded {
                file: lib.path.join(&first.rel_path),
                ext: first.file_name().rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default(),
            })
        } else {
            cover_sidecar(&w.extras, first.dir())
                .map(|p| covers::Source::Sidecar { root: lib.path.clone(), rel: p.to_owned() })
        };
        let cover_src = src.as_ref().map(|s| fingerprint(s, &lib.path, &records));
        let reuse = previous_covers
            .get(&id)
            .filter(|(hash, from)| Some(from) == cover_src.as_ref() && covers::exists(&cfg.data_dir, &id, hash))
            .map(|(hash, _)| hash.clone());
        let cover = match (reuse, src) {
            (Some(h), _) => {
                rep.covers += 1;
                Some(h)
            }
            (None, Some(s)) => match covers::make(&cfg.ffmpeg, &cfg.data_dir, &id, s).await {
                Ok(h) => {
                    rep.covers += 1;
                    Some(h)
                }
                Err(e) => {
                    rep.problems.push((first.rel_path.clone(), "cover", e));
                    None
                }
            },
            (None, None) => None,
        };
        if plan.needs_copy {
            rep.needs_copy += 1;
        }
        rows.push(BookRow {
            id,
            folder: folder_path,
            meta_json: meta_json(&meta),
            duration_ms: plan.duration_ms,
            needs_copy: plan.needs_copy,
            cover_src: cover.as_ref().and(cover_src),
            cover,
            parts: parts.iter().map(|p| p.rel_path.clone()).collect(),
            chapters,
        });
        metas.push(meta);
    }

    let items: Vec<Item> =
        metas.iter().zip(&asm.books).map(|(m, b)| Item { meta: m, duration_ms: b.duration_ms }).collect();
    let groups: Vec<GroupRow> = series::group(&items)
        .into_iter()
        .map(|g| {
            let kind = match g.kind {
                Kind::Show => "show",
                Kind::BookSeries => "series",
            };
            GroupRow {
                id: book_id(&id_key, &BookKey::Group { library: lib_no, name: &g.name.to_lowercase() }),
                members: g
                    .members
                    .iter()
                    .enumerate()
                    .map(|(n, &i)| (rows[i].id.clone(), n >= g.episodes && g.kind == Kind::Show))
                    .collect(),
                name: g.name,
                qualifier: g.qualifier,
                kind,
            }
        })
        .collect();
    rep.groups = groups.len();
    rep.shows = groups.iter().filter(|g| g.kind == "show").count();
    rep.books = rows.len();
    let problems: Vec<ProblemRow> =
        rep.problems.iter().map(|(p, k, d)| ProblemRow { rel_path: p.clone(), kind: k, detail: d.clone() }).collect();
    let (_, gone, changed) = cat.replace_library(lib_id, &rows, &groups, &problems, now())?;
    rep.missing = gone;
    // A book's audio changed: its playable copy and data-saver segments were made from the
    // old files (stale audio, and a layout that no longer matches, review M2).
    for id in changed.iter().filter(|id| abshare_core::ids::is_valid_id(id)) {
        let _ = std::fs::remove_file(cfg.data_dir.join("copies").join(format!("{id}.m4b")));
        let _ = std::fs::remove_dir_all(cfg.data_dir.join("saver").join(id));
    }
    Ok(rep)
}

/// The book's folder and up to two parent folders (never the library root itself): rips
/// often keep the info text or cover one level above the audio.
fn nearby<'a>(extras: &'a BTreeMap<String, DirExtras>, dir: &str) -> Vec<&'a DirExtras> {
    let mut out = Vec::new();
    let mut d = dir.to_owned();
    for _ in 0..3 {
        if d.is_empty() {
            break;
        }
        if let Some(x) = extras.get(&d) {
            out.push(x);
        }
        d = d.rsplit_once('/').map_or(String::new(), |(p, _)| p.to_owned());
    }
    out
}

fn sidecars(root: &Path, extras: &BTreeMap<String, DirExtras>, dir: &str) -> Sidecars {
    let mut s = Sidecars::default();
    for rel in nearby(extras, dir).iter().flat_map(|x| x.texts.iter()).take(8) {
        // Opened safely and size-capped: a sidecar swapped for a link to a secret or to
        // /dev/zero after the walk is refused (review M1).
        let Some(text) = crate::media::read_inside(root, rel, 256 * 1024).ok().and_then(|b| String::from_utf8(b).ok())
        else {
            continue;
        };
        let name = rel.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
        match name.as_str() {
            "desc.txt" | "description.txt" => s.desc = Some(text),
            "reader.txt" | "narrator.txt" => s.reader = Some(text),
            _ => s.info.push(text),
        }
    }
    s
}

fn cover_sidecar<'a>(extras: &'a BTreeMap<String, DirExtras>, dir: &str) -> Option<&'a String> {
    nearby(extras, dir).into_iter().find_map(cover_in)
}

fn cover_in(x: &DirExtras) -> Option<&String> {
    let named = x.images.iter().find(|p| {
        let n = p.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
        n.starts_with("cover.") || n.starts_with("folder.")
    });
    named.or_else(|| (x.images.len() == 1).then(|| &x.images[0]))
}

/// Identifies a cover source: the file it comes from, with its size and modification time.
fn fingerprint(src: &covers::Source, root: &Path, records: &HashMap<String, crate::catalogue::StoredFile>) -> String {
    match src {
        covers::Source::Embedded { file, .. } => {
            let rel = file.strip_prefix(root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let (size, mtime) = records.get(&rel).map_or((0, 0), |r| (r.size, r.mtime_ns));
            format!("embedded:{rel}:{size}:{mtime}")
        }
        covers::Source::Sidecar { root: r, rel } => {
            let m = std::fs::symlink_metadata(r.join(rel)).ok();
            let (size, mtime) = m.map_or((0, 0), |m| (m.len(), std::os::unix::fs::MetadataExt::mtime(&m)));
            let _ = root;
            format!("sidecar:{rel}:{size}:{mtime}")
        }
    }
}

fn meta_json(m: &BookMeta) -> String {
    let people = |p: &[abshare_core::metadata::Person]| {
        p.iter().map(|x| json!({"name": x.name, "role": x.role})).collect::<Vec<_>>()
    };
    json!({
        "title": m.title,
        "subtitle": m.subtitle,
        "authors": people(&m.authors),
        "narrators": people(&m.narrators),
        "series": m.series.as_ref().map(|s| json!({
            "name": s.name,
            "qualifier": s.qualifier,
            "position": s.position.filter(|p| !p.is_companion()).map(|p| p.display()),
            "sort": s.position.map(|p| p.thousandths()),
        })),
        "description": m.description,
        "genres": m.genres,
        "year": m.year,
        "publisher": m.publisher,
        "language": m.language,
        "asin": m.asin,
        "isbn": m.isbn,
    })
    .to_string()
}

/// One library's scan, for `last-scan.json` (shown to admins).
pub fn summary_json(rep: &Report) -> serde_json::Value {
    let mut kinds: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (_, k, _) in &rep.problems {
        *kinds.entry(k).or_default() += 1;
    }
    serde_json::json!({
        "library": rep.library, "files": rep.files, "probed": rep.probed, "books": rep.books,
        "needs_copy": rep.needs_copy, "missing": rep.missing, "groups": rep.groups, "shows": rep.shows,
        "covers": rep.covers, "problems": kinds,
    })
}

/// Records the finished scan in the data directory, atomically.
pub fn write_last_scan(data_dir: &std::path::Path, libraries: &[serde_json::Value], took: f64) -> Result<(), String> {
    let v = serde_json::json!({ "at": now(), "took_seconds": (took * 10.0).round() / 10.0, "libraries": libraries });
    let tmp = data_dir.join("last-scan.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, data_dir.join("last-scan.json")).map_err(|e| e.to_string())
}

pub fn print(rep: &Report, show_problems: bool) {
    println!(
        "{}: {} files ({} probed now) → {} books ({} need a playable copy), {} series/shows ({} shows), {} covers, {} missing since last scan, {} problems",
        rep.library,
        rep.files,
        rep.probed,
        rep.books,
        rep.needs_copy,
        rep.groups,
        rep.shows,
        rep.covers,
        rep.missing,
        rep.problems.len()
    );
    if show_problems {
        let mut by: HashMap<&str, usize> = HashMap::new();
        for (_, k, _) in &rep.problems {
            *by.entry(k).or_default() += 1;
        }
        for (path, kind, detail) in &rep.problems {
            println!("  [{kind}] {path}: {detail}");
        }
        let mut kinds: Vec<_> = by.into_iter().collect();
        kinds.sort();
        println!("  totals: {kinds:?}");
    }
}
