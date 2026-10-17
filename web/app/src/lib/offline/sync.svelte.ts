// Progress sync (SPEC §8.4): every position, finished mark and restart goes into a per-user
// outbox with an idempotency key, and is removed only when the server has answered. Works
// the same online and offline; the outbox flushes on reconnect, oldest first.
import { ApiError, NetworkError } from '$lib/api/http';
import { post, session } from '$lib/api/session.svelte';
import { all, get, reqP, tx, type LocalPosition, type OutboxItem } from './db';
import { userDb } from './userdb';

export type Action = 'position' | 'finished' | 'unfinished' | 'restart';

const newKey = () => (crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`).replace(/[^A-Za-z0-9_-]/g, '');

class Sync {
	/** Items waiting to reach the server. */
	pending = $state(0);
	#flushing = false;
	#retry: ReturnType<typeof setTimeout> | undefined;

	/** Records an event for a book: stored locally at once, sent when possible. */
	async record(book: string, action: Action, positionMs = 0) {
		const dbp = userDb();
		if (!dbp) return;
		const db = await dbp;
		const now = Date.now();
		const item: OutboxItem = { book, body: { key: newKey(), event_time_ms: now, action, ...(action === 'position' ? { position_ms: Math.max(0, Math.round(positionMs)) } : {}) } };
		await tx(db, ['outbox', 'positions'], 'readwrite', (t) => {
			const out = t.objectStore('outbox');
			// A newer position replaces unsent older ones for the same book (they'd lose anyway).
			// The new item is added only once the cursor is done, so the sweep can't remove it.
			if (action === 'position') {
				const cur = out.openCursor();
				cur.onsuccess = () => {
					const c = cur.result;
					if (!c) {
						out.add(item);
						return;
					}
					const v = c.value as OutboxItem;
					if (v.book === book && v.body.action === 'position') c.delete();
					c.continue();
				};
			} else {
				out.add(item);
			}
			// Finished/unfinished marks carry no position: this device's position stays as it was.
			if (action === 'position' || action === 'restart') {
				const pos: LocalPosition = { book, position_ms: action === 'restart' ? 0 : (item.body.position_ms ?? 0), at: now };
				t.objectStore('positions').put(pos);
			}
		});
		void this.flush();
	}

	/** This device's last known position for a book. */
	async local(book: string): Promise<LocalPosition | undefined> {
		const dbp = userDb();
		return dbp ? get<LocalPosition>(await dbp, 'positions', book) : undefined;
	}

	/** Sends the outbox, oldest first. */
	async flush() {
		if (this.#flushing || session.status !== 'signed-in') return;
		const dbp = userDb();
		const owner = session.user?.username;
		if (!dbp || !owner) return;
		this.#flushing = true;
		clearTimeout(this.#retry);
		try {
			const db = await dbp;
			const items = (await all<OutboxItem>(db, 'outbox')).sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0));
			this.pending = items.length;
			for (const it of items) {
				// Never send one person's queue under another's session (review W2).
				if (session.user?.username !== owner || session.status !== 'signed-in') return;
				try {
					await post(`v1/progress/${it.book}`, it.body, 'PUT');
				} catch (e) {
					// Gone for good (unknown book, too old, malformed): drop it. Otherwise try later.
					const permanent = e instanceof ApiError && e.status >= 400 && e.status < 500 && e.status !== 401 && e.status !== 429;
					if (!permanent) {
						const wait = e instanceof ApiError && e.retryAfter ? e.retryAfter * 1000 : e instanceof NetworkError ? 30_000 : 15_000;
						this.#retry = setTimeout(() => void this.flush(), wait);
						return;
					}
				}
				await reqP(db.transaction('outbox', 'readwrite').objectStore('outbox').delete(it.seq!));
				this.pending = Math.max(0, this.pending - 1);
			}
		} catch {
			this.#retry = setTimeout(() => void this.flush(), 30_000);
		} finally {
			this.#flushing = false;
		}
	}
}

export const sync = new Sync();

if (typeof window !== 'undefined') {
	addEventListener('online', () => void sync.flush());
}
