// Downloads for offline listening (SPEC §8): fetched in 4 MiB Range pieces into IndexedDB, so
// they resume after an interruption or a page kill. One download runs at a time, and only
// while the app is open (no browser lets a web app download in the background).
import { session } from '$lib/api/session.svelte';
import type { BookDetail } from '$lib/api/types';
import { streamUrl } from '$lib/player/rules';
import { all, dropDb, get, PIECE, reqP, tx, type Download, type Quality } from './db';
import { closeUserDb, userDb } from './userdb';

/** A download without its blobs, for the UI. */
export type DownloadInfo = Omit<Download, 'cover'>;

export interface StorageInfo {
	usage: number;
	quota: number;
	persisted: boolean | null;
}

const strip = ({ cover: _cover, ...rest }: Download): DownloadInfo => rest;

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

	/** Removes every download of the current user (sign-out with "Remove"). */
	async removeAll(username: string) {
		await closeUserDb();
		await dropDb(username);
		this.list = [];
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
		const url = streamUrl(id, d.quality === 'original' ? 'original' : d.quality);
		const started = performance.now();
		let fetched = 0;
		let retried401 = false;
		while (d.total === 0 || d.next < d.total) {
			if (this.#stop.has(id)) {
				await this.#set(id, { state: 'paused' });
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

