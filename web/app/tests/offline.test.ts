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
