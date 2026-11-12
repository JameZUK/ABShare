// @vitest-environment node
// Offline storage against a real IndexedDB implementation (fake-indexeddb): the progress
// outbox, Range answers from stored pieces, and isolation between users on one browser.
import 'fake-indexeddb/auto';
import { ApiError, NetworkError } from '$lib/api/http';

const fake = vi.hoisted(() => ({
	session: { status: 'signed-in' as string, user: { username: 'ada', display_name: 'Ada', role: 'listener' } as { username: string } | null },
	post: vi.fn(async (_path: string, _body: unknown, _method?: string): Promise<unknown> => ({ result: 'accepted' })),
}));
vi.mock('$lib/api/session.svelte', () => ({ session: fake.session, post: fake.post, api: vi.fn() }));

const { sync } = await import('$lib/offline/sync.svelte');
const { closeUserDb, userDb } = await import('$lib/offline/userdb');
const { all, answerFromPieces, dbName, dropDb, openDb, reqP, tx } = await import('$lib/offline/db');
import type { Download, OutboxItem } from '$lib/offline/db';

const A = 'aaaaaaaaaaaaaaaaaaaaaaaaaa';
const B = 'bbbbbbbbbbbbbbbbbbbbbbbbbb';

async function outbox(): Promise<OutboxItem[]> {
	return (await all<OutboxItem>(await userDb()!, 'outbox')).sort((x, y) => (x.seq ?? 0) - (y.seq ?? 0));
}

async function as(user: string) {
	await closeUserDb();
	fake.session.user = { username: user };
}

beforeEach(async () => {
	fake.session.status = 'offline'; // record without flushing unless a test says so
	fake.post.mockReset();
	fake.post.mockImplementation(async () => ({ result: 'accepted' }));
	for (const u of ['ada', 'bob']) {
		await as(u);
		await closeUserDb();
		await dropDb(u);
	}
	await as('ada');
});

test('a newer position replaces unsent older ones as progress; they are kept as heard reports', async () => {
	await sync.record(A, 'position', 1_000);
	await sync.record(A, 'position', 2_000);
	await sync.record(B, 'position', 5_000);
	await sync.record(A, 'position', 3_000);
	const items = await outbox();
	expect(items.map((i) => [i.book, i.body.action, i.body.position_ms])).toEqual([
		[A, 'heard', 1_000],
		[A, 'heard', 2_000],
		[B, 'position', 5_000],
		[A, 'position', 3_000],
	]);
	expect((await sync.local(A))?.position_ms).toBe(3_000);
});

test('on reconnect, heard reports are replayed into history, by book and in order, before progress', async () => {
	for (let p = 1; p <= 4; p++) await sync.record(A, 'position', p * 15_000);
	await sync.record(B, 'position', 60_000);
	await sync.record(B, 'position', 75_000);
	fake.session.status = 'signed-in';
	await sync.flush();
	const calls = fake.post.mock.calls.map(([path, body]) => [path, body] as [string, Record<string, unknown>]);
	const replays = calls.filter(([p]) => p === 'v1/me/history/replay');
	expect(replays.map(([, b]) => [b.book, (b.reports as { position_ms: number }[]).map((r) => r.position_ms)])).toEqual([
		[A, [15_000, 30_000, 45_000]],
		[B, [60_000]],
	]);
	for (const [, b] of replays) expect(String(b.key)).toMatch(/^[A-Za-z0-9_-]{8,64}$/);
	// Then the newest positions as progress, after every replay.
	const progress = calls.map(([p]) => p).filter((p) => p.startsWith('v1/progress/'));
	expect(progress).toEqual([`v1/progress/${A}`, `v1/progress/${B}`]);
	expect(calls.findIndex(([p]) => p.startsWith('v1/progress/'))).toBeGreaterThan(calls.findLastIndex(([p]) => p === 'v1/me/history/replay'));
	expect(await outbox()).toEqual([]);
});

test('a replay that fails for a moment is kept for later; one refused for good is dropped', async () => {
	await sync.record(A, 'position', 1_000);
	await sync.record(A, 'position', 2_000);
	fake.session.status = 'signed-in';
	fake.post.mockImplementation(async (path: string) => {
		if (path === 'v1/me/history/replay') throw new NetworkError();
		return { result: 'accepted' };
	});
	await sync.flush();
	expect((await outbox()).map((i) => i.body.action)).toEqual(['heard', 'position']);
	fake.post.mockImplementation(async (path: string) => {
		if (path === 'v1/me/history/replay') throw new ApiError(404, 'not_found', 'Not found.');
		return { result: 'accepted' };
	});
	await sync.flush();
	expect(await outbox()).toEqual([]);
});

test('a replay retried after a lost answer carries the same key (review W11)', async () => {
	await sync.record(A, 'position', 1_000);
	await sync.record(A, 'position', 2_000);
	await sync.record(A, 'position', 3_000);
	fake.session.status = 'signed-in';
	fake.post.mockImplementationOnce(async () => {
		throw new NetworkError();
	});
	await sync.flush();
	await sync.flush();
	const keys = fake.post.mock.calls.filter(([p]) => p === 'v1/me/history/replay').map(([, b]) => (b as { key: string }).key);
	expect(keys).toHaveLength(2);
	expect(keys[0]).toMatch(/^[A-Za-z0-9_-]{8,64}$/);
	expect(keys[1]).toBe(keys[0]);
});

test('while the server asks us to wait, new positions don’t send early (review W17)', async () => {
	await sync.record(A, 'position', 1_000);
	fake.session.status = 'signed-in';
	fake.post.mockImplementationOnce(async () => {
		throw new ApiError(429, 'too_many_requests', 'slow down', 600);
	});
	await sync.flush();
	expect(fake.post).toHaveBeenCalledTimes(1);
	await sync.record(A, 'position', 16_000);
	await sync.record(A, 'position', 31_000);
	expect(fake.post).toHaveBeenCalledTimes(1);
	// An explicit flush (reconnecting) still sends.
	await sync.flush();
	expect(fake.post.mock.calls.length).toBeGreaterThan(1);
});

test('heard reports are batched and capped', async () => {
	const { replayBatches, REPLAY_BATCH } = await import('$lib/offline/sync.svelte');
	const items = Array.from({ length: REPLAY_BATCH * 2 + 1 }, (_, i) => ({ seq: i, book: A, body: { key: `k${i}`, event_time_ms: i, action: 'heard', position_ms: i } }));
	const batches = replayBatches([...items, { seq: 999, book: B, body: { key: 'kb', event_time_ms: 1, action: 'heard', position_ms: 1 } }]);
	expect(batches.map((b) => [b[0]!.book, b.length])).toEqual([
		[A, REPLAY_BATCH],
		[A, REPLAY_BATCH],
		[A, 1],
		[B, 1],
	]);
});

test('marks are never collapsed and keep their order', async () => {
	await sync.record(A, 'position', 9_000);
	await sync.record(A, 'restart');
	await sync.record(A, 'position', 1_000);
	await sync.record(A, 'finished');
	const items = await outbox();
	// The position replaced before the restart stays only as a heard report (for history).
	expect(items.map((i) => i.body.action)).toEqual(['heard', 'restart', 'position', 'finished']);
	expect(new Set(items.map((i) => i.body.key)).size, 'every event has its own idempotency key').toBe(4);
	for (const i of items) expect(i.body.key).toMatch(/^[A-Za-z0-9_-]{8,64}$/);
	expect((await sync.local(A))?.position_ms).toBe(1_000);
});

test('the outbox is sent oldest first and emptied only on the server’s answer', async () => {
	await sync.record(A, 'restart');
	await sync.record(A, 'position', 4_000);
	fake.session.status = 'signed-in';
	await sync.flush();
	expect(fake.post.mock.calls.map((c) => [c[0], (c[1] as OutboxItem['body']).action, c[2]])).toEqual([
		[`v1/progress/${A}`, 'restart', 'PUT'],
		[`v1/progress/${A}`, 'position', 'PUT'],
	]);
	expect(await outbox()).toEqual([]);
});

test('offline or throttled: the item stays for later; refused for good: it is dropped', async () => {
	await sync.record(A, 'position', 1_000);
	await sync.record(B, 'position', 2_000);
	fake.session.status = 'signed-in';
	fake.post.mockImplementationOnce(async () => {
		throw new NetworkError();
	});
	await sync.flush();
	expect((await outbox()).length, 'nothing lost while offline').toBe(2);
	fake.post.mockImplementationOnce(async () => {
		throw new ApiError(429, 'too_many_requests', 'slow down', 5);
	});
	await sync.flush();
	expect((await outbox()).length).toBe(2);
	fake.post.mockImplementationOnce(async () => {
		throw new ApiError(404, 'not_found', 'gone');
	});
	await sync.flush();
	expect((await outbox()).map((i) => i.book), 'a book that no longer exists is dropped; the rest is sent').toEqual([]);
	expect(fake.post).toHaveBeenCalledTimes(4);
});

test('another user on the same browser sees none of the first user’s data', async () => {
	await sync.record(A, 'position', 7_000);
	const db = await userDb()!;
	await reqP(db.transaction('books', 'readwrite').objectStore('books').put({ id: A, next: 10 } as unknown as Download));
	expect(dbName('ada')).not.toBe(dbName('bob'));
	await as('bob');
	expect(await sync.local(A)).toBeUndefined();
	expect(await outbox()).toEqual([]);
	expect(await all(await userDb()!, 'books')).toEqual([]);
	// Removing Bob's data leaves Ada's alone.
	await closeUserDb();
	await dropDb('bob');
	await as('ada');
	expect((await sync.local(A))?.position_ms).toBe(7_000);
});

describe('playing from stored pieces', () => {
	async function store(pieces: number[][]) {
		const db = await openDb('ada');
		const offsets: number[] = [];
		let at = 0;
		for (const p of pieces) {
			offsets.push(at);
			at += p.length;
		}
		const d = { id: A, detail: {}, quality: 'original', etag: 'x', type: 'audio/mpeg', total: 100, next: at, pieces: offsets, state: 'paused', added: 1 } as unknown as Download;
		await tx(db, ['books', 'pieces'], 'readwrite', (t) => {
			pieces.forEach((p, i) => t.objectStore('pieces').put(new Blob([new Uint8Array(p)]), [A, offsets[i]!]));
			t.objectStore('books').put(d);
		});
		return db;
	}
	const bytes = async (r: Response) => [...new Uint8Array(await r.arrayBuffer())];

	test('ranges are stitched across pieces, and a partial download plays as a shorter book', async () => {
		const db = await store([[0, 1, 2, 3], [4, 5, 6, 7], [8, 9]]);
		let r = await answerFromPieces(db, A, 'bytes=2-5');
		expect(r.status).toBe(206);
		expect(r.headers.get('content-range')).toBe('bytes 2-5/10');
		expect(await bytes(r)).toEqual([2, 3, 4, 5]);
		r = await answerFromPieces(db, A, null);
		expect(r.status).toBe(200);
		expect(await bytes(r)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
		r = await answerFromPieces(db, A, 'bytes=-3');
		expect(await bytes(r)).toEqual([7, 8, 9]);
		r = await answerFromPieces(db, A, 'bytes=10-');
		expect(r.status).toBe(416);
		expect(r.headers.get('content-range')).toBe('bytes */10');
		expect((await answerFromPieces(db, B, null)).status, 'not downloaded').toBe(404);
		db.close();
	});
});

test('a flush stops if someone else signs in part-way (review W2)', async () => {
	await sync.record(A, 'position', 1_000);
	await sync.record(B, 'position', 2_000);
	fake.session.status = 'signed-in';
	fake.post.mockImplementationOnce(async () => {
		fake.session.user = { username: 'bob' }; // another person signs in on this page
		return { result: 'accepted' };
	});
	await sync.flush();
	expect(fake.post).toHaveBeenCalledTimes(1);
	fake.session.user = { username: 'ada' };
});

test('wiping a user’s data is not blocked by an open connection (review W3)', async () => {
	await sync.record(A, 'position', 3_000);
	await userDb(); // this page keeps its connection open
	await dropDb('ada'); // as another tab's sign-out would
	await closeUserDb();
	expect(await sync.local(A)).toBeUndefined();
});

test('an event recorded while the outbox is being sent goes in a second pass, not at the next trigger (review L24)', async () => {
	await sync.record(A, 'position', 1_000);
	fake.session.status = 'signed-in';
	fake.post.mockImplementationOnce(async () => {
		await sync.record(B, 'position', 2_000); // recorded mid-flush
		return { result: 'accepted' };
	});
	await sync.flush();
	expect(fake.post.mock.calls.map((c) => c[0])).toEqual([`v1/progress/${A}`, `v1/progress/${B}`]);
	expect(await outbox()).toEqual([]);
});

// The server refuses events stamped more than 5 minutes ahead of its clock (422 `clock`), and
// stores nothing for them. Listening must never be dropped for a wrong clock (review L23).
describe('a device whose clock is wrong', () => {
	const clock = (serverTime?: number) => new ApiError(422, 'clock', 'This device’s clock is too far ahead.', undefined, serverTime);
	type Body = OutboxItem['body'];

	beforeEach(() => {
		sync.clock = 'ok';
	});

	test('a refused time is put right by the server’s clock, and the order of events is kept', async () => {
		const m = await import('$lib/offline/sync.svelte');
		// An hour fast: moved back an hour.
		expect(m.correctedTime(5_000_000, 5_000_100, 1_400_100)).toBe(1_400_000);
		// Stamped under a bigger error than today's: never later than the server's now.
		expect(m.correctedTime(9_000_000, 2_000_000, 2_000_000)).toBe(2_000_000);
		// A clock that is not ahead explains nothing: still capped at the server's now.
		expect(m.correctedTime(3_000_000, 1_000_000, 2_000_000)).toBe(2_000_000);
		// Later than the one sent before it for the same book.
		expect(m.correctedTime(9_000_000, 2_000_000, 2_000_000, 2_000_000)).toBe(2_000_001);
		expect(m.correctedTime(5_000_000, 5_000_100, 1_400_100, 1_000)).toBe(1_400_000);
	});

	test('ahead: the event is sent again under the same key with the corrected time, never dropped', async () => {
		const HOUR = 3_600_000;
		vi.spyOn(Date, 'now').mockReturnValue(1_800_000_000_000 + HOUR); // this device is an hour fast
		try {
			await sync.record(A, 'position', 7_000);
			fake.session.status = 'signed-in';
			fake.post.mockImplementationOnce(async () => {
				throw clock(1_800_000_000_000);
			});
			await sync.flush();
			const sent = fake.post.mock.calls.map((c) => c[1] as Body);
			expect(sent).toHaveLength(2);
			expect(sent[1]!.key, 'the same idempotency key').toBe(sent[0]!.key);
			expect(sent[0]!.event_time_ms).toBe(1_800_000_000_000 + HOUR);
			expect(sent[1]!.event_time_ms).toBe(1_800_000_000_000);
			expect(sent[1]!.position_ms).toBe(7_000);
			expect(await outbox(), 'removed only once the server took it').toEqual([]);
			expect(sync.clock).toBe('ok');
		} finally {
			vi.restoreAllMocks();
		}
	});

	test('ahead and refused again, or with no server time to go by: it stays queued and the listener is told', async () => {
		await sync.record(A, 'position', 7_000);
		await sync.record(B, 'finished');
		fake.session.status = 'signed-in';
		fake.post.mockImplementation(async () => {
			throw clock(undefined);
		});
		await sync.flush();
		expect(fake.post).toHaveBeenCalledTimes(1);
		expect((await outbox()).map((i) => i.body.action), 'nothing lost').toEqual(['position', 'finished']);
		expect(sync.clock).toBe('ahead');

		fake.post.mockReset();
		fake.post.mockImplementation(async () => {
			throw clock(Date.now());
		});
		await sync.flush();
		expect(fake.post, 'one corrected attempt, then it waits').toHaveBeenCalledTimes(2);
		expect((await outbox()).map((i) => i.body.action)).toEqual(['position', 'finished']);
		expect(sync.clock).toBe('ahead');

		// The clock put right: everything goes, and the notice with it.
		fake.post.mockReset();
		fake.post.mockImplementation(async () => ({ result: 'accepted' }));
		await sync.flush();
		expect(await outbox()).toEqual([]);
		expect(sync.clock).toBe('ok');
	});

	test('a corrected time that was lost on the way is sent the same again', async () => {
		await sync.record(A, 'position', 7_000);
		fake.session.status = 'signed-in';
		const serverNow = Date.now() - 3_600_000;
		fake.post.mockImplementationOnce(async () => {
			throw clock(serverNow);
		});
		fake.post.mockImplementationOnce(async () => {
			throw new NetworkError();
		});
		await sync.flush();
		const [kept] = await outbox();
		expect(kept!.body.event_time_ms, 'the correction is kept').toBeLessThanOrEqual(serverNow);
		await sync.flush();
		const sent = fake.post.mock.calls.map((c) => c[1] as Body);
		expect(sent[2]).toEqual(sent[1]);
		expect(await outbox()).toEqual([]);
	});

	test('too old is still refused for good (the server will never take it)', async () => {
		await sync.record(A, 'position', 7_000);
		fake.session.status = 'signed-in';
		fake.post.mockImplementationOnce(async () => {
			throw new ApiError(422, 'too_old', 'That listening was too long ago to record.');
		});
		await sync.flush();
		expect(await outbox()).toEqual([]);
		expect(sync.clock).toBe('ok');
	});

	test('behind: a stale answer stamped in this device’s future tells the listener; an ordinary one doesn’t', async () => {
		const m = await import('$lib/offline/sync.svelte');
		expect(m.clockBehind(Date.now() + m.MAX_FUTURE_MS + 1_000, Date.now())).toBe(true);
		expect(m.clockBehind(Date.now() + 60_000, Date.now())).toBe(false);

		await sync.record(A, 'position', 7_000);
		fake.session.status = 'signed-in';
		// Another device listened later: nothing wrong with any clock.
		fake.post.mockImplementationOnce(async () => ({ result: 'stale', progress: { position_ms: 9_000, finished: false, updated_at: Date.now() + 1_000 } }));
		await sync.flush();
		expect(sync.clock).toBe('ok');
		// The server's progress is stamped a day after this device's now: this clock is behind.
		fake.session.status = 'offline';
		await sync.record(A, 'position', 8_000);
		fake.session.status = 'signed-in';
		fake.post.mockImplementationOnce(async () => ({ result: 'stale', progress: { position_ms: 9_000, finished: false, updated_at: Date.now() + 86_400_000 } }));
		await sync.flush();
		expect(sync.clock).toBe('behind');
		expect(await outbox(), 'the server has answered for it').toEqual([]);
	});
});

test('the server’s clock is read from an error’s Date header', async () => {
	const { callJson, serverTimeOf } = await import('$lib/api/http');
	expect(serverTimeOf('Thu, 12 Nov 2026 10:00:00 GMT')).toBe(Date.UTC(2026, 10, 12, 10));
	expect(serverTimeOf(null)).toBeUndefined();
	expect(serverTimeOf('soon')).toBeUndefined();
	vi.stubGlobal('fetch', async () => new Response(JSON.stringify({ error: 'clock', message: 'ahead' }), { status: 422, headers: { date: 'Thu, 12 Nov 2026 10:00:00 GMT' } }));
	try {
		const e = await callJson('v1/progress/x', { method: 'PUT' }).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect((e as ApiError).serverTime).toBe(Date.UTC(2026, 10, 12, 10));
	} finally {
		vi.unstubAllGlobals();
	}
});

// A browser that refuses IndexedDB (private modes, blocked site data): listening still works
// and progress still reaches the server (review L26b).
describe('with no IndexedDB', () => {
	beforeEach(async () => {
		await closeUserDb();
		vi.spyOn(indexedDB, 'open').mockImplementation(() => {
			throw new DOMException('The operation is insecure.', 'SecurityError');
		});
	});
	afterEach(async () => {
		vi.restoreAllMocks();
		await closeUserDb();
	});

	test('the last local position is simply unknown: opening a book doesn’t fail', async () => {
		await expect(sync.local(A)).resolves.toBeUndefined();
	});

	test('progress is sent straight to the server when it can’t be queued', async () => {
		fake.session.status = 'signed-in';
		await sync.record(A, 'position', 4_000);
		await sync.record(A, 'finished');
		expect(fake.post.mock.calls.map((c) => [c[0], (c[1] as OutboxItem['body']).action, (c[1] as OutboxItem['body']).position_ms, c[2]])).toEqual([
			[`v1/progress/${A}`, 'position', 4_000, 'PUT'],
			[`v1/progress/${A}`, 'finished', undefined, 'PUT'],
		]);
		for (const c of fake.post.mock.calls) expect((c[1] as OutboxItem['body']).key).toMatch(/^[A-Za-z0-9_-]{8,64}$/);
	});

	test('offline with no outbox, recording fails quietly and sends nothing', async () => {
		fake.session.status = 'offline';
		await expect(sync.record(A, 'position', 4_000)).resolves.toBeUndefined();
		expect(fake.post).not.toHaveBeenCalled();
	});
});
