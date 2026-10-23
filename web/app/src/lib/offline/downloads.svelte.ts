// Downloads for offline listening (SPEC §8): fetched in 4 MiB Range pieces into IndexedDB, so
// they resume after an interruption or a page kill. One download runs at a time, and only
// while the app is open (no browser lets a web app download in the background).
import { session } from '$lib/api/session.svelte';
import type { BookDetail } from '$lib/api/types';
import { streamUrl } from '$lib/player/rules';
import { all, dropDb, get, PIECE, reqP, tx, type Download, type Quality } from './db';
import { closeUserDb, userDb } from './userdb';
import { forgetAsked } from '$lib/reviewAsked';
import { settings } from '$lib/settings.svelte';
import { WAITING_FOR_WIFI, connection, shouldHold } from './wifi';

/** A download without its blobs, for the UI. */
export type DownloadInfo = Omit<Download, 'cover'>;

export interface StorageInfo {
	usage: number;
	quota: number;
	persisted: boolean | null;
}

const strip = ({ cover: _cover, ...rest }: Download): DownloadInfo => rest;

const WIPES = 'abshare.pendingWipes';

function pendingWipes(): string[] {
	try {
		const v = JSON.parse(localStorage.getItem(WIPES) ?? '[]');
		return Array.isArray(v) ? v.filter((u): u is string => typeof u === 'string' && /^[a-z0-9][a-z0-9._-]{2,31}$/.test(u)) : [];
	} catch {
		return [];
	}
}

function pendingWipe(user: string, on: boolean) {
	try {
		const set = new Set(pendingWipes());
		if (on) set.add(user);
		else set.delete(user);
		localStorage.setItem(WIPES, JSON.stringify([...set]));
	} catch {
		/* best effort */
	}
}

/** Removes what this device keeps about a person beyond their database: everything keyed by
 *  their name in local storage (review W10). */
function forgetPerson(user: string) {
	settings.forgetSpeeds(user);
	forgetAsked(user);
}

class Downloads {
	list = $state<DownloadInfo[]>([]);
	storage = $state<StorageInfo | null>(null);
	/** Bytes per second of the running download. */
	rate = $state(0);
	#running = false;
	#stop = new Set<string>();

	byId(id: string): DownloadInfo | undefined {
		return this.list.find((d) => d.id === id);
	}

	/** Reloads the list from the database (at start-up and after sign-in). */
	async load() {
		const dbp = userDb();
		if (!dbp) {
			this.list = [];
			return;
		}
		const rows = await all<Download>(await dbp, 'books');
		this.list = rows.map(strip).sort((a, b) => b.added - a.added);
		void this.refreshStorage();
		if (session.status === 'signed-in') void this.#run();
	}

	async refreshStorage() {
		if (!navigator.storage?.estimate) return;
		const est = await navigator.storage.estimate();
		const persisted = navigator.storage.persisted ? await navigator.storage.persisted() : null;
		this.storage = { usage: est.usage ?? 0, quota: est.quota ?? 0, persisted };
	}

	/** Queues a book. Asks the browser to keep stored data (it may say no). */
	async add(book: BookDetail, quality: Quality) {
		// A plain copy: IndexedDB can't store the page's reactive object.
		const detail = $state.snapshot(book) as BookDetail;
		const dbp = userDb();
		if (!dbp) return;
		const db = await dbp;
		const existing = await get<Download>(db, 'books', detail.id);
		if (existing && existing.quality === quality) {
			if (existing.state !== 'complete') await this.resume(detail.id);
			return;
		}
		if (existing) await this.remove(detail.id);
		try {
			await navigator.storage?.persist?.();
		} catch {
			/* optional */
		}
		const d: Download = { id: detail.id, detail, quality, etag: '', type: 'audio/mpeg', total: 0, next: 0, pieces: [], state: 'queued', added: Date.now() };
		await reqP(db.transaction('books', 'readwrite').objectStore('books').put(d));
		await this.load();
		void this.#coverFor(d);
	}

	async #coverFor(d: Download) {
		if (!d.detail.cover) return;
		try {
			const r = await fetch(`v1/${d.detail.cover}?size=640`, { credentials: 'same-origin' });
			if (!r.ok) return;
			const blob = await r.blob();
			const db = await userDb()!;
			const cur = await get<Download>(db, 'books', d.id);
			if (cur) await reqP(db.transaction('books', 'readwrite').objectStore('books').put({ ...cur, cover: blob }));
		} catch {
			/* the book still plays without a cover */
		}
	}

	/** The stored cover of a downloaded book. */
	async cover(id: string): Promise<Blob | undefined> {
		const dbp = userDb();
		return dbp ? (await get<Download>(await dbp, 'books', id))?.cover : undefined;
	}

	/** The stored details of a downloaded book (for the book page and player offline). */
	async detail(id: string): Promise<BookDetail | undefined> {
		const dbp = userDb();
		return dbp ? (await get<Download>(await dbp, 'books', id))?.detail : undefined;
	}

	pause(id: string) {
		this.#stop.add(id);
	}

	async resume(id: string) {
		this.#stop.delete(id);
		await this.#set(id, { state: 'queued', error: undefined });
		void this.#run();
	}

	async remove(id: string) {
		this.#stop.add(id);
		const dbp = userDb();
		if (!dbp) return;
		const db = await dbp;
		const d = await get<Download>(db, 'books', id);
		if (d) {
			await tx(db, ['books', 'pieces'], 'readwrite', (t) => {
				t.objectStore('pieces').delete(IDBKeyRange.bound([id, 0], [id, Number.MAX_SAFE_INTEGER]));
				t.objectStore('books').delete(id);
			});
		}
		this.#stop.delete(id);
		await this.load();
	}

	/** Whether downloads are being held for Wi-Fi right now. */
	get holding(): boolean {
		return shouldHold(settings.prefs.wifiOnly, connection());
	}

	/** After the connection or the Wi-Fi preference changes: carry on with downloads that were
	 *  waiting for Wi-Fi, if they no longer need to wait. */
	async recheck() {
		if (this.holding) return;
		for (const d of this.list.filter((x) => x.state === 'paused' && x.error === WAITING_FOR_WIFI)) await this.resume(d.id);
	}

	/** Removes every download of the current user (sign-out with "Remove"). */
	async removeAll(username: string) {
		// Remembered until done, so a wipe interrupted (the app closed, another tab holding
		// the database) is finished at the next start (review W3).
		pendingWipe(username, true);
		await closeUserDb();
		await dropDb(username);
		forgetPerson(username);
		pendingWipe(username, false);
		this.list = [];
	}

	/** Finishes any wipe that didn't complete last time. */
	async finishWipes() {
		for (const u of pendingWipes()) {
			await dropDb(u);
			forgetPerson(u);
			pendingWipe(u, false);
		}
	}

	async #set(id: string, change: Partial<Download>) {
		const db = await userDb()!;
		const cur = await get<Download>(db, 'books', id);
		if (!cur) return;
		await reqP(db.transaction('books', 'readwrite').objectStore('books').put({ ...cur, ...change }));
		const i = this.list.findIndex((d) => d.id === id);
		if (i >= 0) this.list[i] = strip({ ...cur, ...change });
	}

	async #run() {
		if (this.#running) return;
		this.#running = true;
		try {
			for (;;) {
				const next = this.list.find((d) => d.state === 'queued' || d.state === 'downloading');
				if (!next || session.status !== 'signed-in') break;
				if (this.holding) {
					for (const d of this.list.filter((x) => x.state === 'queued' || x.state === 'downloading')) await this.#set(d.id, { state: 'paused', error: WAITING_FOR_WIFI });
					break;
				}
				await this.#fetchBook(next.id);
			}
		} finally {
			this.#running = false;
			this.rate = 0;
			void this.refreshStorage();
		}
	}

	async #fetchBook(id: string) {
		const db = await userDb()!;
		let d = await get<Download>(db, 'books', id);
		if (!d) return;
		await this.#set(id, { state: 'downloading' });
		// dl=1: the server encodes each data-saver piece in one run (downloads read in order).
		const base = streamUrl(id, d.quality === 'original' ? 'original' : d.quality);
		const url = `${base}${base.includes('?') ? '&' : '?'}dl=1`;
		const started = performance.now();
		let fetched = 0;
		let retried401 = false;
		let busyWaits = 0;
		while (d.total === 0 || d.next < d.total) {
			if (this.#stop.has(id)) {
				await this.#set(id, { state: 'paused' });
				return;
			}
			// Switched to mobile data mid-download: stop between pieces and wait for Wi-Fi.
			if (this.holding) {
				await this.#set(id, { state: 'paused', error: WAITING_FOR_WIFI });
				return;
			}
			let resp: Response;
			try {
				resp = await fetch(url, { credentials: 'same-origin', cache: 'no-store', headers: { Range: `bytes=${d.next}-${d.next + PIECE - 1}` } });
			} catch {
				await this.#set(id, { state: 'paused', error: 'The connection dropped. It carries on from where it stopped.' });
				return;
			}
			if (resp.status === 401 && !retried401) {
				retried401 = true;
				if (await session.refresh()) continue;
			}
			// Too many playing or downloading at once (another device): wait a moment and carry on.
			// The daily limit of different books: stop, and say why (the server's words).
			if (resp.status === 429) {
				const why = (await resp.json().catch(() => null)) as { error?: string; message?: string } | null;
				const wait = Number(resp.headers.get('retry-after')) || 5;
				if (why?.error === 'too_many_at_once' && busyWaits++ < 6) {
					await new Promise((r) => setTimeout(r, Math.min(wait, 60) * 1000));
					continue;
				}
				await this.#set(id, { state: 'paused', error: why?.message ?? 'Too many downloads for now. Try again later.' });
				return;
			}
			if (resp.status === 409) {
				await this.#set(id, { state: 'paused', error: 'This book is being prepared. Try again in a few minutes.' });
				return;
			}
			if (resp.status === 416 && d.total > 0) break;
			if (resp.status !== 206) {
				await this.#set(id, { state: 'error', error: `The download failed (${resp.status}).` });
				return;
			}
			const cr = /bytes (\d+)-(\d+)\/(\d+)/.exec(resp.headers.get('content-range') ?? '');
			if (!cr || Number(cr[1]) !== d.next) {
				await this.#set(id, { state: 'error', error: 'The server answered oddly. Remove the download and try again.' });
				return;
			}
			const etag = resp.headers.get('etag') ?? '';
			if (d.next > 0 && d.etag !== etag) {
				// The book changed on the server: the stored part is no good. Start again.
				await tx(db, ['pieces'], 'readwrite', (t) => t.objectStore('pieces').delete(IDBKeyRange.bound([id, 0], [id, Number.MAX_SAFE_INTEGER])));
				d = { ...d, etag: '', next: 0, total: 0, pieces: [] };
				await this.#set(id, d);
				continue;
			}
			const blob = await resp.blob();
			const end = Number(cr[2]) + 1;
			if (blob.size !== end - d.next) {
				await this.#set(id, { state: 'paused', error: 'A piece arrived incomplete. It carries on from where it stopped.' });
				return;
			}
			const updated: Download = { ...d, etag, type: resp.headers.get('content-type') ?? d.type, total: Number(cr[3]), next: end, pieces: [...d.pieces, d.next], state: 'downloading', error: undefined };
			try {
				await tx(db, ['books', 'pieces'], 'readwrite', (t) => {
					t.objectStore('pieces').put(blob, [id, d!.next]);
					// Keep the cover written by #coverFor.
					const books = t.objectStore('books');
					const r = books.get(id);
					r.onsuccess = () => books.put({ ...updated, cover: (r.result as Download | undefined)?.cover });
				});
			} catch {
				await this.#set(id, { state: 'paused', error: 'This device is out of space for downloads.' });
				return;
			}
			d = updated;
			fetched += blob.size;
			this.rate = fetched / Math.max(0.001, (performance.now() - started) / 1000);
			const i = this.list.findIndex((x) => x.id === id);
			if (i >= 0) this.list[i] = strip(updated);
		}
		await this.#set(id, { state: 'complete', error: undefined });
	}

	/** Is this book playable from the device (fully or partly downloaded)? */
	playable(id: string): boolean {
		const d = this.byId(id);
		return !!d && d.next > 0;
	}
}

export const downloads = new Downloads();

/** For book pages: the size of a download before starting it. */
export async function sizeOf(book: string, quality: Quality): Promise<number | null> {
	try {
		const r = await fetch(streamUrl(book, quality === 'original' ? 'original' : quality), { credentials: 'same-origin', headers: { Range: 'bytes=0-0' } });
		const m = /\/(\d+)$/.exec(r.headers.get('content-range') ?? '');
		void r.body?.cancel();
		return m ? Number(m[1]) : null;
	} catch {
		return null;
	}
}

// Back on Wi-Fi (or off mobile data): carry on with downloads that were waiting.
if (typeof window !== 'undefined') {
	connection()?.addEventListener?.('change', () => void downloads.recheck());
}
