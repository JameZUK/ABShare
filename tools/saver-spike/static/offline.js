// Phase 0(a): download the book into IndexedDB in pieces and play it back offline.
// Option A: one Blob URL composed from the stored pieces.
// Option B: a service worker answers Range requests for /offline/<id> from IndexedDB.
'use strict';

const PIECE = 4 * 1024 * 1024;
const DB = 'abshare-spike';
const audio = document.getElementById('player');
const logEl = document.getElementById('log');
const progressEl = document.getElementById('progress');
const storageEl = document.getElementById('storage');
const statusEl = document.getElementById('status');
const ua = navigator.userAgent;
let stopRequested = false;
let blobUrl = null;

function note(msg) {
  const li = document.createElement('li');
  li.textContent = `${new Date().toISOString().slice(11, 23)} ${msg}`;
  logEl.prepend(li);
  while (logEl.children.length > 200) logEl.lastChild.remove();
  fetch('log', { method: 'POST', body: `${ua} :: offline :: ${msg}`, keepalive: true }).catch(() => {});
}

const mib = (n) => `${(n / 1048576).toFixed(1)} MiB`;
const what = () => document.querySelector('input[name=what]:checked').value;
const bookId = (src) => src.replace(/[^a-z0-9]/gi, '_');

function openDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      db.createObjectStore('manifest', { keyPath: 'id' });
      db.createObjectStore('pieces');
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function tx(db, stores, mode, fn) {
  return new Promise((resolve, reject) => {
    const t = db.transaction(stores, mode);
    let out;
    t.oncomplete = () => resolve(out);
    t.onerror = () => reject(t.error);
    t.onabort = () => reject(t.error);
    out = fn(t);
  });
}

function reqP(r) {
  return new Promise((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
}

async function getManifest(db, id) {
  const t = db.transaction('manifest');
  return reqP(t.objectStore('manifest').get(id));
}

async function showStorage() {
  if (!navigator.storage) { storageEl.textContent = 'Storage API not available'; return; }
  const est = await navigator.storage.estimate();
  const persisted = navigator.storage.persisted ? await navigator.storage.persisted() : 'unknown';
  storageEl.textContent = `Storage: ${mib(est.usage || 0)} used of ${mib(est.quota || 0)} · persistent: ${persisted}`;
}

async function showProgress() {
  const db = await openDb();
  const m = await getManifest(db, bookId(what()));
  db.close();
  if (!m) { progressEl.textContent = 'Nothing downloaded yet.'; return; }
  progressEl.textContent = `${mib(m.next)} of ${mib(m.total)} (${((m.next / m.total) * 100).toFixed(1)} %)${m.next >= m.total ? ' · complete' : ''}`;
}

async function download() {
  stopRequested = false;
  const src = what();
  const id = bookId(src);
  const db = await openDb();
  let m = await getManifest(db, id);
  const started = performance.now();
  let fetched = 0;
  note(`download ${src} from ${m ? mib(m.next) : 0}`);
  try {
    for (;;) {
      if (stopRequested) { note('download paused'); break; }
      const from = m ? m.next : 0;
      if (m && from >= m.total) { note('download complete'); break; }
      const resp = await fetch(src, { headers: { Range: `bytes=${from}-${from + PIECE - 1}` }, cache: 'no-store' });
      if (resp.status !== 206) throw new Error(`HTTP ${resp.status}`);
      const cr = (resp.headers.get('content-range') || '').match(/bytes (\d+)-(\d+)\/(\d+)/);
      if (!cr || Number(cr[1]) !== from) throw new Error('bad Content-Range');
      const etag = resp.headers.get('etag') || '';
      if (m && m.etag !== etag) throw new Error('the book changed on the server; remove and start again');
      const blob = await resp.blob();
      const end = Number(cr[2]) + 1;
      if (blob.size !== end - from) throw new Error(`short piece: ${blob.size} of ${end - from}`);
      m = { id, src, etag, type: resp.headers.get('content-type'), total: Number(cr[3]), next: end, pieces: [...(m ? m.pieces : []), from] };
      await tx(db, ['manifest', 'pieces'], 'readwrite', (t) => {
        t.objectStore('pieces').put(blob, [id, from]);
        t.objectStore('manifest').put(m);
      });
      fetched += blob.size;
      const secs = (performance.now() - started) / 1000;
      progressEl.textContent = `${mib(m.next)} of ${mib(m.total)} (${((m.next / m.total) * 100).toFixed(1)} %) · ${(fetched / 1048576 / secs).toFixed(1)} MiB/s`;
    }
  } catch (e) {
    note(`download error: ${e.message || e}`);
  } finally {
    db.close();
    showStorage();
  }
}

async function remove() {
  const id = bookId(what());
  const db = await openDb();
  const m = await getManifest(db, id);
  if (m) {
    await tx(db, ['manifest', 'pieces'], 'readwrite', (t) => {
      for (const p of m.pieces) t.objectStore('pieces').delete([id, p]);
      t.objectStore('manifest').delete(id);
    });
  }
  db.close();
  note('removed');
  showProgress();
  showStorage();
}

async function playA() {
  const id = bookId(what());
  const db = await openDb();
  const m = await getManifest(db, id);
  if (!m || m.next < m.total) { note('download it fully first'); db.close(); return; }
  const t0 = performance.now();
  const parts = [];
  const t = db.transaction('pieces');
  for (const p of m.pieces) parts.push(await reqP(t.objectStore('pieces').get([id, p])));
  db.close();
  if (blobUrl) URL.revokeObjectURL(blobUrl);
  blobUrl = URL.createObjectURL(new Blob(parts, { type: m.type }));
  note(`A: Blob URL from ${parts.length} pieces in ${Math.round(performance.now() - t0)} ms`);
  audio.src = blobUrl;
  audio.play().catch((e) => note(`play refused: ${e.name}`));
}

async function playB() {
  if (!('serviceWorker' in navigator)) { note('no service worker support'); return; }
  const reg = await navigator.serviceWorker.register('sw.js');
  await navigator.serviceWorker.ready;
  if (!navigator.serviceWorker.controller) { note('service worker installed: reload the page once, then try B again'); return; }
  note(`B: service worker active (${reg.active ? reg.active.state : '?'})`);
  audio.src = `offline/${bookId(what())}`;
  audio.play().catch((e) => note(`play refused: ${e.name}`));
}

for (const ev of ['loadedmetadata', 'playing', 'pause', 'waiting', 'stalled', 'ended', 'error', 'seeked']) {
  audio.addEventListener(ev, () => {
    const extra = ev === 'error' && audio.error ? ` code ${audio.error.code} ${audio.error.message || ''}` : '';
    note(`${ev} @ ${audio.currentTime.toFixed(1)} s${ev === 'loadedmetadata' ? ` of ${audio.duration.toFixed(0)} s` : ''}${extra}`);
  });
}
document.addEventListener('visibilitychange', () => note(`page ${document.visibilityState}`));
setInterval(() => { statusEl.textContent = `${audio.currentTime.toFixed(0)} s / ${(audio.duration || 0).toFixed(0)} s · ${navigator.onLine ? 'online' : 'OFFLINE'}`; }, 1000);

document.getElementById('start').addEventListener('click', download);
document.getElementById('stop').addEventListener('click', () => { stopRequested = true; });
document.getElementById('remove').addEventListener('click', remove);
document.getElementById('playA').addEventListener('click', playA);
document.getElementById('playB').addEventListener('click', playB);
document.getElementById('persist').addEventListener('click', async () => {
  const ok = navigator.storage && navigator.storage.persist ? await navigator.storage.persist() : false;
  note(`persist() → ${ok}`);
  showStorage();
});
for (const b of document.querySelectorAll('button[data-frac]')) {
  b.addEventListener('click', () => { if (isFinite(audio.duration)) audio.currentTime = Number(b.dataset.frac) * audio.duration; });
}
for (const r of document.querySelectorAll('input[name=what]')) r.addEventListener('change', showProgress);

if ('serviceWorker' in navigator) navigator.serviceWorker.register('sw.js').catch((e) => note(`sw register: ${e.message}`));
showStorage();
showProgress();
note(`offline page loaded: ${ua}`);
