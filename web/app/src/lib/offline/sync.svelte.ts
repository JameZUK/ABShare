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

/** How long before trying again when the server refuses an event for this device's clock. */
const CLOCK_RETRY_MS = 5 * 60_000;
/** The server takes events stamped up to this far ahead of its clock (core::progress). */
export const MAX_FUTURE_MS = 5 * 60_000;

/**
 * An event time the server refused as too far ahead, put right by the server's own clock
 * (the Date header of its answer): moved back by how far this device's clock is ahead, and
 * never later than the server's now (an event can't have happened after it was sent).
 * `after` is the time last sent for the same book: the order events were recorded in is kept.
 */
export function correctedTime(eventMs: number, deviceNowMs: number, serverNowMs: number, after?: number): number {
	const t = Math.min(eventMs - Math.max(0, deviceNowMs - serverNowMs), serverNowMs);
	return after !== undefined && t <= after ? after + 1 : t;
}

/** A "stale" answer shows this device's clock is behind when the progress the server keeps
 *  is stamped later than this device's now by more than the server would ever have accepted. */
export const clockBehind = (storedAtMs: number, deviceNowMs: number) => storedAtMs > deviceNowMs + MAX_FUTURE_MS;

const newKey = () => (crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`).replace(/[^A-Za-z0-9_-]/g, '');

class Sync {
	/** Items waiting to reach the server. */
	pending = $state(0);
	/** This device's clock, as far as the server's answers show: `ahead` when events are
	 *  refused for it and can't be put right, `behind` when they lose to older progress. Either
	 *  way the listener is told (the layout's banner); nothing is dropped for it. */
	clock = $state<'ok' | 'ahead' | 'behind'>('ok');
	#flushing = false;
	/** Something asked for a flush while one was running (an event recorded meanwhile). */
	#again = false;
	#retry: ReturnType<typeof setTimeout> | undefined;

	/** Records an event for a book: stored locally at once, sent when possible. */
	async record(book: string, action: Action, positionMs = 0) {
		const dbp = userDb();
		if (!dbp) return;
		const now = Date.now();
		const item: OutboxItem = { book, body: { key: newKey(), event_time_ms: now, action, ...(action === 'position' ? { position_ms: Math.max(0, Math.round(positionMs)) } : {}) } };
		try {
			await this.#store(await dbp, item, now);
		} catch {
			// This browser can't store it (IndexedDB blocked or full): send it straight away
			// rather than lose it. Best effort: with no outbox there is nowhere to keep it.
			if (session.status === 'signed-in') void post(`v1/progress/${book}`, item.body, 'PUT').catch(() => {});
			return;
		}
		// While the server has asked us to wait (Retry-After) or the last try failed, the
		// timer already set sends it; don't send every 15 s anyway (review W17).
		if (this.#retry === undefined) void this.flush();
	}

	/** Puts an event in the outbox, and keeps this device's position. */
	async #store(db: IDBDatabase, item: OutboxItem, now: number) {
		const { book } = item;
		const action = item.body.action;
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
	}

	/** This device's last known position for a book; none if this browser can't store any. */
	async local(book: string): Promise<LocalPosition | undefined> {
		try {
			const dbp = userDb();
			return dbp ? await get<LocalPosition>(await dbp, 'positions', book) : undefined;
		} catch {
			return undefined;
		}
	}

	/** Sends the outbox, oldest first. Asked for while one is running (an event was recorded
	 *  meanwhile), it runs once more afterwards, so nothing waits for the next trigger. */
	async flush() {
		if (session.status !== 'signed-in') return;
		if (this.#flushing) {
			this.#again = true;
			return;
		}
		this.#flushing = true;
		try {
			do {
				this.#again = false;
				clearTimeout(this.#retry);
				this.#retry = undefined;
				await this.#pass();
				// Not when the pass ended waiting to try again: the timer set then sends the rest.
			} while (this.#again && this.#retry === undefined);
		} finally {
			this.#flushing = false;
		}
	}

	/** One pass over what is in the outbox now. */
	async #pass() {
		if (session.status !== 'signed-in') return;
		const dbp = userDb();
		const owner = session.user?.username;
		if (!dbp || !owner) return;
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
			// The corrected time last sent for each book, after a refusal for this device's clock.
			const sentAt = new Map<string, number>();
			for (const queued of items) {
				let it = queued;
				// Never send one person's queue under another's session (review W2).
				if (session.user?.username !== owner || session.status !== 'signed-in') return;
				// After a corrected event for the same book, a later one keeps its place in order.
				const floor = sentAt.get(it.book);
				if (floor !== undefined && it.body.event_time_ms <= floor) {
					it = { ...it, body: { ...it.body, event_time_ms: floor + 1 } };
					sentAt.set(it.book, floor + 1);
				}
				try {
					let r: Answer;
					try {
						r = await post<Answer>(`v1/progress/${it.book}`, it.body, 'PUT');
					} catch (e) {
						// This device's clock is ahead (422 `clock`): the server stored nothing, not
						// even the key. Never dropped for that: the time is put right by the server's
						// clock and the event sent again, under the same key. If that can't be done
						// (no Date header) or is refused too, it stays queued and the listener is told.
						if (!(e instanceof ApiError && e.status === 422 && e.code === 'clock')) throw e;
						if (e.serverTime === undefined) throw new ClockAhead();
						const at = correctedTime(it.body.event_time_ms, Date.now(), e.serverTime, sentAt.get(it.book));
						it = { ...it, body: { ...it.body, event_time_ms: at } };
						// Kept corrected, so a retry after a lost answer sends the same thing. Only the
						// time changes: the item may have become a heard report meanwhile.
						await tx(db, ['outbox'], 'readwrite', (t) => {
							const out = t.objectStore('outbox');
							const cur = out.get(it.seq!);
							cur.onsuccess = () => {
								const v = cur.result as OutboxItem | undefined;
								if (v) out.put({ ...v, body: { ...v.body, event_time_ms: at } });
							};
						});
						try {
							r = await post<Answer>(`v1/progress/${it.book}`, it.body, 'PUT');
						} catch (again) {
							throw again instanceof ApiError && again.status === 422 && again.code === 'clock' ? new ClockAhead() : again;
						}
						sentAt.set(it.book, it.body.event_time_ms);
					}
					// "stale": the server keeps newer progress (another device listened later) and
					// that is right, unless its stamp is in this device's future: then this
					// device's clock is behind, and its listening is losing to older progress.
					const stored = r?.result === 'stale' ? r.progress?.updated_at : undefined;
					if (typeof stored === 'number' && clockBehind(stored, Date.now())) this.clock = 'behind';
					else if (r?.result === 'accepted') this.clock = 'ok';
				} catch (e) {
					if (e instanceof ClockAhead) {
						this.clock = 'ahead';
						this.#retry = setTimeout(() => void this.flush(), CLOCK_RETRY_MS);
						return;
					}
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

/** The server's answer to a progress event. */
type Answer = { result?: string; progress?: { updated_at?: number } | null } | null;

/** The server refuses an event for this device's clock, and it couldn't be put right. */
class ClockAhead extends Error {}

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
