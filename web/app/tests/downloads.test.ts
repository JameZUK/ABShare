// @vitest-environment node
// The download manager: Range pieces into IndexedDB, resuming, and refusing to stitch a
// book that changed on the server.
import 'fake-indexeddb/auto';
import type { BookDetail } from '$lib/api/types';

const fake = vi.hoisted(() => ({
	session: { status: 'signed-in' as string, user: { username: 'ada' } as { username: string } | null, refresh: async () => true },
}));
vi.mock('$lib/api/session.svelte', () => ({ session: fake.session, api: vi.fn(), post: vi.fn() }));

const { downloads } = await import('$lib/offline/downloads.svelte');
const { closeUserDb } = await import('$lib/offline/userdb');
const { dropDb, PIECE } = await import('$lib/offline/db');

const ID = 'cccccccccccccccccccccccccc';
const book = { id: ID, title: 'A Book', cover: null, duration_ms: 1000, chapters: [], media: { original: `media/${ID}`, preparing: false } } as unknown as BookDetail;

/** A fake server holding `size` bytes; `etag` can change between requests. */
function server(size: number, opts: { etag?: () => string; status?: () => number } = {}) {
	const urls: string[] = [];
	const f = vi.fn(async (url: string, init?: RequestInit) => {
		urls.push(url);
		if (url.includes('/covers/')) return new Response(null, { status: 404 });
		const status = opts.status?.() ?? 206;
		if (status !== 206) return new Response('{}', { status });
		const m = /bytes=(\d+)-(\d+)/.exec(String((init?.headers as Record<string, string>)?.Range));
		const start = Number(m![1]);
		const end = Math.min(Number(m![2]), size - 1);
		const body = new Uint8Array(end - start + 1).map((_, i) => (start + i) % 251);
		return new Response(body, { status: 206, headers: { 'content-range': `bytes ${start}-${end}/${size}`, etag: opts.etag?.() ?? '"v1"', 'content-type': 'audio/mpeg' } });
	});
	vi.stubGlobal('fetch', f);
	return { f, urls };
}

async function settle() {
	for (let i = 0; i < 200 && downloads.list.some((d) => d.state === 'queued' || d.state === 'downloading'); i++) {
		await new Promise((r) => setTimeout(r, 5));
	}
}

beforeEach(async () => {
	await closeUserDb();
	await dropDb('ada');
	await downloads.load();
});
afterEach(() => vi.unstubAllGlobals());

test('a book downloads in pieces, asking for data saver encoding in one run', async () => {
	const { urls } = server(PIECE * 2 + 10);
	await downloads.add(book, 48);
	await settle();
	const d = downloads.byId(ID)!;
	expect(d.state).toBe('complete');
	expect([d.next, d.total, d.pieces]).toEqual([PIECE * 2 + 10, PIECE * 2 + 10, [0, PIECE, PIECE * 2]]);
	expect(urls.filter((u) => u.startsWith('v1/media/') && !u.includes('covers'))).toEqual(Array(3).fill(`v1/media/${ID}?q=48&dl=1`));
});

test('if the book changes on the server mid-download, the stored part is thrown away', async () => {
	let n = 0;
	server(PIECE * 2, { etag: () => (n++ === 0 ? '"old"' : '"new"') });
	await downloads.add(book, 'original');
	await settle();
	const d = downloads.byId(ID)!;
	expect(d.state).toBe('complete');
	expect(d.etag).toBe('"new"');
	expect(d.pieces).toEqual([0, PIECE]);
});

test('a book still being prepared pauses with a message, and resumes later', async () => {
	let status = 409;
	server(100, { status: () => status });
	await downloads.add(book, 'original');
	await settle();
	expect(downloads.byId(ID)?.state).toBe('paused');
	expect(downloads.byId(ID)?.error).toMatch(/prepared/);
	status = 206;
	await downloads.resume(ID);
	await settle();
	expect(downloads.byId(ID)?.state).toBe('complete');
});

test('removing a download deletes it', async () => {
	server(100);
	await downloads.add(book, 'original');
	await settle();
	await downloads.remove(ID);
	expect(downloads.byId(ID)).toBeUndefined();
	await downloads.load();
	expect(downloads.list).toEqual([]);
});

test('removing a person’s downloads also forgets which books they were asked to review (review W10)', async () => {
	const { markAsked, askedBefore } = await import('$lib/reviewAsked');
	const mem = new Map<string, string>();
	vi.stubGlobal('localStorage', {
		getItem: (k: string) => mem.get(k) ?? null,
		setItem: (k: string, v: string) => void mem.set(k, v),
		removeItem: (k: string) => void mem.delete(k),
	});
	try {
		markAsked('ada', ID);
		markAsked('bob', ID);
		expect(askedBefore('ada', ID)).toBe(true);
		await downloads.removeAll('ada');
		expect(mem.has('abshare.reviewAsked.ada')).toBe(false);
		expect(askedBefore('bob', ID), 'only that person’s').toBe(true);
		// An interrupted wipe, finished at the next start, does the same.
		markAsked('ada', ID);
		mem.set('abshare.pendingWipes', JSON.stringify(['ada']));
		await downloads.finishWipes();
		expect(mem.has('abshare.reviewAsked.ada')).toBe(false);
	} finally {
		vi.unstubAllGlobals();
	}
});

test('the daily limit of different books pauses the download with the server’s reason', async () => {
	let status = 429;
	const { f } = server(100, { status: () => status });
	f.mockImplementationOnce(async () =>
		new Response(JSON.stringify({ error: 'daily_limit', message: 'You’ve played or downloaded 30 different books today.' }), { status: 429, headers: { 'retry-after': '3600' } }),
	);
	await downloads.add(book, 'original');
	await settle();
	expect(downloads.byId(ID)?.state).toBe('paused');
	expect(downloads.byId(ID)?.error).toBe('You’ve played or downloaded 30 different books today.');
	status = 206;
	await downloads.resume(ID);
	await settle();
	expect(downloads.byId(ID)?.state).toBe('complete');
});

test('too many at once (another device busy) waits and carries on by itself', async () => {
	const { f } = server(100);
	f.mockImplementationOnce(async () =>
		new Response(JSON.stringify({ error: 'too_many_at_once', message: 'Too many at once.' }), { status: 429, headers: { 'retry-after': '1' } }),
	);
	await downloads.add(book, 'original');
	for (let i = 0; i < 60 && downloads.byId(ID)?.state !== 'complete'; i++) await new Promise((r) => setTimeout(r, 50));
	expect(downloads.byId(ID)?.state).toBe('complete');
	expect(downloads.byId(ID)?.error).toBeUndefined();
});
