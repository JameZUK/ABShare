// Progress sync (SPEC §8.4): every position, finished mark and restart goes into a per-user
// outbox with an idempotency key, and is removed only when the server has answered. Works
// the same online and offline; the outbox flushes on reconnect, oldest first.
//
// Offline listening (SPEC D33): a newer position replaces an unsent older one as progress (it
// would lose anyway), but the older one is kept as a "heard" report, so the time listened
// offline reaches the listening history: on reconnect, heard reports are replayed into it
// (`/v1/me/history/replay`, in batches) before the newest position is sent as progress.
import { ApiError, NetworkError } from '$lib/api/http';
import { post, session } from '$lib/api/session.svelte';
import { all, get, reqP, tx, type LocalPosition, type OutboxItem } from './db';
import { userDb } from './userdb';

export type Action = 'position' | 'finished' | 'unfinished' | 'restart';

/** Heard reports kept while offline, at most (one every 15 s: about 40 hours). */
export const MAX_HEARD = 10_000;
/** Heard reports per replay request. */
export const REPLAY_BATCH = 200;

/** A replay batch's idempotency key, the same on every attempt, so a retry after a lost
 *  answer is recognised (review W11). Item keys are random and at most 36 characters. */
export const batchKey = (batch: OutboxItem[]) => `r${batch[0]!.body.key.slice(0, 36)}n${batch.length}`.replace(/[^A-Za-z0-9_-]/g, '');

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
				let heard = 0;
				const cur = out.openCursor();
				cur.onsuccess = () => {
					const c = cur.result;
					if (!c) {
						out.add(item);
						if (heard > MAX_HEARD) trimHeard(out, heard - MAX_HEARD);
						return;
					}
					const v = c.value as OutboxItem;
					if (v.book === book && v.body.action === 'position') {
						c.update({ ...v, body: { ...v.body, action: 'heard' } });
						heard++;
					} else if (v.body.action === 'heard') heard++;
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
		// While the server has asked us to wait (Retry-After) or the last try failed, the
		// timer already set sends it; don't send every 15 s anyway (review W17).
		if (this.#retry === undefined) void this.flush();
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
		this.#retry = undefined;
		try {
			const db = await dbp;
			const everything = (await all<OutboxItem>(db, 'outbox')).sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0));
			this.pending = everything.length;
			// Heard reports first, a book at a time, so the history is in order before the
			// newest position (which continues it) is sent.
			const heard = everything.filter((it) => it.body.action === 'heard');
			for (const batch of replayBatches(heard)) {
				if (session.user?.username !== owner || session.status !== 'signed-in') return;
				const body = { book: batch[0]!.book, key: batchKey(batch), reports: batch.map((it) => ({ event_time_ms: it.body.event_time_ms, position_ms: it.body.position_ms ?? 0 })) };
				try {
					await post('v1/me/history/replay', body);
				} catch (e) {
					const permanent = e instanceof ApiError && e.status >= 400 && e.status < 500 && e.status !== 401 && e.status !== 429;
					if (!permanent) {
						const wait = e instanceof ApiError && e.retryAfter ? e.retryAfter * 1000 : e instanceof NetworkError ? 30_000 : 15_000;
						this.#retry = setTimeout(() => void this.flush(), wait);
						return;
					}
				}
				const t = db.transaction('outbox', 'readwrite').objectStore('outbox');
				await Promise.all(batch.map((it) => reqP(t.delete(it.seq!))));
				this.pending = Math.max(0, this.pending - batch.length);
			}
			const items = everything.filter((it) => it.body.action !== 'heard');
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

/** Deletes the oldest `n` heard reports (the store is in insertion order). */
function trimHeard(out: IDBObjectStore, n: number) {
	const cur = out.openCursor();
	cur.onsuccess = () => {
		const c = cur.result;
		if (!c || n <= 0) return;
		if ((c.value as OutboxItem).body.action === 'heard') {
			c.delete();
			n--;
		}
		c.continue();
	};
}

/** Heard reports grouped by book, in order, in batches of at most `REPLAY_BATCH`. */
export function replayBatches(heard: OutboxItem[]): OutboxItem[][] {
	const byBook = new Map<string, OutboxItem[]>();
	for (const it of heard) byBook.set(it.book, [...(byBook.get(it.book) ?? []), it]);
	const out: OutboxItem[][] = [];
	for (const items of byBook.values()) for (let i = 0; i < items.length; i += REPLAY_BATCH) out.push(items.slice(i, i + REPLAY_BATCH));
	return out;
}

export const sync = new Sync();

if (typeof window !== 'undefined') {
	addEventListener('online', () => void sync.flush());
}
