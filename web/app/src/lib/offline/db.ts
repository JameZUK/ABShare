// Per-user IndexedDB (SPEC §8.2): downloaded books in 4 MiB pieces, the progress outbox and
// the last local positions. Also used by the service worker, which answers the player's
// Range requests for `offline/<user>/<book>` from the stored pieces.
import type { BookDetail } from '$lib/api/types';

export const PIECE = 4 * 1024 * 1024;

export type Quality = 'original' | 32 | 40 | 48;

export interface Download {
	id: string;
	detail: BookDetail;
	quality: Quality;
	etag: string;
	type: string;
	/** Total size once known (0 before the first piece). */
	total: number;
	/** Bytes stored, from the start: pieces are contiguous. */
	next: number;
	pieces: number[];
	state: 'queued' | 'downloading' | 'paused' | 'complete' | 'error';
	error?: string;
	cover?: Blob;
	added: number;
}

export interface OutboxItem {
	seq?: number;
	book: string;
	body: { key: string; event_time_ms: number; action: string; position_ms?: number };
}

export interface LocalPosition {
	book: string;
	position_ms: number;
	/** When it was last heard (ms since the epoch). */
	at: number;
}

const VERSION = 1;

/** Database names are per user, so a second user on this browser never sees the first's data. */
export const dbName = (username: string) => `abshare-u-${username}`;

export function openDb(username: string): Promise<IDBDatabase> {
	return new Promise((resolve, reject) => {
		const req = indexedDB.open(dbName(username), VERSION);
		req.onupgradeneeded = () => {
			const db = req.result;
			if (!db.objectStoreNames.contains('books')) db.createObjectStore('books', { keyPath: 'id' });
			if (!db.objectStoreNames.contains('pieces')) db.createObjectStore('pieces');
			if (!db.objectStoreNames.contains('outbox')) db.createObjectStore('outbox', { keyPath: 'seq', autoIncrement: true });
			if (!db.objectStoreNames.contains('positions')) db.createObjectStore('positions', { keyPath: 'book' });
		};
		req.onsuccess = () => resolve(req.result);
		req.onerror = () => reject(req.error);
		req.onblocked = () => reject(new Error('database blocked'));
	});
}

export function reqP<T>(r: IDBRequest<T>): Promise<T> {
	return new Promise((resolve, reject) => {
		r.onsuccess = () => resolve(r.result);
		r.onerror = () => reject(r.error);
	});
}

/** Runs `fn` in one transaction and resolves when it commits. */
export function tx<T>(db: IDBDatabase, stores: string[], mode: IDBTransactionMode, fn: (t: IDBTransaction) => T): Promise<T> {
	return new Promise((resolve, reject) => {
		const t = db.transaction(stores, mode);
		let out: T;
		t.oncomplete = () => resolve(out);
		t.onerror = () => reject(t.error);
		t.onabort = () => reject(t.error ?? new Error('transaction aborted'));
		out = fn(t);
	});
}

export const get = <T>(db: IDBDatabase, store: string, key: IDBValidKey) => reqP<T | undefined>(db.transaction(store).objectStore(store).get(key));
export const all = <T>(db: IDBDatabase, store: string) => reqP<T[]>(db.transaction(store).objectStore(store).getAll());

/** Deletes a user's whole database (sign-out with "Remove downloads"). */
export function dropDb(username: string): Promise<void> {
	return new Promise((resolve) => {
		const req = indexedDB.deleteDatabase(dbName(username));
		req.onsuccess = req.onerror = req.onblocked = () => resolve();
	});
}

/** A Range header against `total` bytes: [start, end] inclusive, 'bad' (416), or null (whole). */
export function parseRange(h: string | null, total: number): [number, number] | 'bad' | null {
	if (!h) return null;
	const m = /^bytes=(\d*)-(\d*)$/.exec(h.trim());
	if (!m || (m[1] === '' && m[2] === '')) return 'bad';
	let start: number;
	let end = total - 1;
	if (m[1] === '') {
		start = Math.max(0, total - Number(m[2]));
	} else {
		start = Number(m[1]);
		if (m[2] !== '') end = Math.min(Number(m[2]), total - 1);
	}
	if (start >= total || start > end) return 'bad';
	return [start, end];
}

/** At most this much per answer from the service worker; the player asks again for more. */
export const MAX_ANSWER = 8 * 1024 * 1024;

/** Answers a Range request from stored pieces. A partial download plays as a shorter book. */
export async function answerFromPieces(db: IDBDatabase, id: string, rangeHeader: string | null): Promise<Response> {
	const d = await get<Download>(db, 'books', id);
	if (!d || d.next === 0) return new Response('Not downloaded', { status: 404 });
	const total = d.next;
	const r = parseRange(rangeHeader, total);
	if (r === 'bad') return new Response(null, { status: 416, headers: { 'Content-Range': `bytes */${total}` } });
	const start = r ? r[0] : 0;
	const end = Math.min(r ? r[1] : total - 1, start + MAX_ANSWER - 1);
	const parts: Blob[] = [];
	const store = db.transaction('pieces').objectStore('pieces');
	const want: Promise<Blob | undefined>[] = [];
	const spans: [number, number][] = [];
	for (let i = 0; i < d.pieces.length; i++) {
		const pStart = d.pieces[i]!;
		const pEnd = (i + 1 < d.pieces.length ? d.pieces[i + 1]! : d.next) - 1;
		if (pEnd < start || pStart > end) continue;
		want.push(reqP(store.get([id, pStart])));
		spans.push([pStart, pEnd]);
	}
	const blobs = await Promise.all(want);
	blobs.forEach((b, i) => {
		const [pStart, pEnd] = spans[i]!;
		if (b) parts.push(b.slice(Math.max(start, pStart) - pStart, Math.min(end, pEnd) - pStart + 1));
	});
	const body = new Blob(parts, { type: d.type });
	const headers: Record<string, string> = { 'Content-Type': d.type, 'Content-Length': String(body.size), 'Accept-Ranges': 'bytes', 'Cache-Control': 'no-store' };
	if (r) headers['Content-Range'] = `bytes ${start}-${start + body.size - 1}/${total}`;
	return new Response(body, { status: r ? 206 : 200, headers });
}
