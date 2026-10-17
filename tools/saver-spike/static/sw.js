// v2 · Phase 0(a) option B: answer the player's Range requests for /offline/<id> from the pieces
// stored in IndexedDB. Every other request goes to the network untouched.
'use strict';

const DB = 'abshare-spike';

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

function openDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => {
      req.result.createObjectStore('manifest', { keyPath: 'id' });
      req.result.createObjectStore('pieces');
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function get(db, store, key) {
  return new Promise((resolve, reject) => {
    const r = db.transaction(store).objectStore(store).get(key);
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

async function respond(request, id) {
  const db = await openDb();
  try {
    const m = await get(db, 'manifest', id);
    if (!m || m.next === 0) return new Response('not downloaded', { status: 404 });
    // Serve the downloaded part as if it were the whole file, so a partial download plays
    // (the player sees a shorter book). A complete download is the real thing.
    const total = m.next;
    const range = (request.headers.get('range') || '').match(/^bytes=(\d*)-(\d*)$/);
    let start = 0;
    let end = total - 1;
    if (range) {
      if (range[1] === '' && range[2] !== '') { start = Math.max(0, total - Number(range[2])); }
      else { start = Number(range[1] || 0); if (range[2] !== '') end = Math.min(Number(range[2]), total - 1); }
      if (start >= total || start > end) return new Response(null, { status: 416, headers: { 'Content-Range': `bytes */${total}` } });
    }
    // Answer at most 8 MiB per response; the player asks again for more.
    end = Math.min(end, start + 8 * 1024 * 1024 - 1);
    const parts = [];
    for (let i = 0; i < m.pieces.length; i++) {
      const pStart = m.pieces[i];
      const pEnd = (i + 1 < m.pieces.length ? m.pieces[i + 1] : m.next) - 1;
      if (pEnd < start || pStart > end) continue;
      const blob = await get(db, 'pieces', [id, pStart]);
      parts.push(blob.slice(Math.max(start, pStart) - pStart, Math.min(end, pEnd) - pStart + 1));
    }
    const body = new Blob(parts, { type: m.type });
    const headers = {
      'Content-Type': m.type,
      'Content-Length': String(body.size),
      'Accept-Ranges': 'bytes',
    };
    if (range) headers['Content-Range'] = `bytes ${start}-${start + body.size - 1}/${total}`;
    return new Response(body, { status: range ? 206 : 200, headers });
  } finally {
    db.close();
  }
}

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  const match = url.pathname.match(/\/offline\/([A-Za-z0-9_]+)$/);
  if (url.origin !== self.location.origin || event.request.method !== 'GET' || !match) return;
  event.respondWith(respond(event.request, match[1]));
});
