// The player against a stand-in audio element (jsdom has no media): which copy it plays, what
// the end of a part-downloaded copy means, how often it retries after errors, that nothing
// waiting acts on a later book, and the sleep timer across a pause.
import type { BookDetail } from '$lib/api/types';

const fake = vi.hoisted(() => ({
	session: { status: 'signed-in' as string, user: { username: 'ada' } as { username: string } | null, refresh: vi.fn(async () => true), settled: async () => {} },
	api: vi.fn(async (_path: string): Promise<unknown> => ({ ready: true })),
	copy: null as 'complete' | 'partial' | null,
	record: vi.fn(async (_book: string, _action: string, _positionMs?: number) => {}),
}));
vi.mock('$lib/api/session.svelte', () => ({ session: fake.session, api: fake.api, post: vi.fn() }));
vi.mock('$lib/offline/downloads.svelte', () => ({ downloads: { copy: () => fake.copy, playable: () => fake.copy !== null, cover: async () => undefined } }));
vi.mock('$lib/offline/sync.svelte', () => ({ sync: { record: fake.record, local: async () => undefined } }));
vi.mock('$lib/pwa/update.svelte', () => ({ swContainer: () => ({ controller: {} }) }));

const { player, COULD_NOT_PLAY, PART_ENDS_HERE } = await import('$lib/player/player.svelte');

const A = 'aaaaaaaaaaaaaaaaaaaaaaaaaa';
const B = 'bbbbbbbbbbbbbbbbbbbbbbbbbb';
const HOUR = 3_600_000;
const book = (id: string, preparing = false) => ({ id, title: 'A Book', cover: null, duration_ms: HOUR, chapters: [], progress: null, media: { original: `media/${id}`, preparing } }) as unknown as BookDetail;

/** What the stand-in element reports. */
const media = { paused: true, currentTime: 0, readyState: 0, duration: NaN, ended: false, error: null as { code: number } | null };
const load = vi.fn();
HTMLMediaElement.prototype.load = load;
HTMLMediaElement.prototype.play = async function () {
	media.paused = false;
};
HTMLMediaElement.prototype.pause = function () {
	media.paused = true;
};
player.start();
const audio = document.querySelector('audio')!;
for (const k of Object.keys(media) as (keyof typeof media)[]) {
	Object.defineProperty(audio, k, { configurable: true, get: () => media[k], set: (v) => ((media as Record<string, unknown>)[k] = v) });
}
const fire = (name: string) => audio.dispatchEvent(new Event(name));
const src = () => audio.getAttribute('src');
/** The audio loaded: metadata is in, and the pending seek is applied. */
function loaded(durationS = 3600) {
	Object.assign(media, { readyState: 4, duration: durationS, error: null, ended: false });
	fire('loadedmetadata');
}

beforeEach(() => {
	vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'performance'] });
	Object.assign(media, { paused: true, currentTime: 0, readyState: 0, duration: NaN, ended: false, error: null });
	Object.assign(fake.session, { status: 'signed-in', user: { username: 'ada' } });
	fake.copy = null;
	fake.api.mockReset();
	fake.api.mockImplementation(async () => ({ ready: true }));
	fake.session.refresh.mockClear();
	fake.record.mockClear();
	load.mockClear();
	player.finishedNow = null;
});
afterEach(async () => {
	player.close();
	await vi.advanceTimersByTimeAsync(60_000);
	vi.useRealTimers();
});

describe('a part-downloaded copy (review M9)', () => {
	test('is not preferred to the stream while the server is in reach; a complete one is', async () => {
		fake.copy = 'partial';
		await player.open(book(A));
		expect(src()).toBe(`v1/media/${A}`);
		expect(player.source).toBe('original');
		fake.copy = 'complete';
		await player.open(book(A));
		expect(src()).toBe(`offline/ada/${A}`);
		expect(player.source).toBe('device');
	});

	test('still plays with no connection, but its end does not finish the book', async () => {
		fake.session.status = 'offline';
		fake.copy = 'partial';
		await player.open(book(A), { play: true });
		expect(src()).toBe(`offline/ada/${A}`);
		// The stored part is 10 minutes of the hour: the file ends there.
		loaded(600);
		Object.assign(media, { currentTime: 600, ended: true, paused: true });
		fire('ended');
		expect(fake.record.mock.calls.map((c) => c[1]), 'never recorded as finished').not.toContain('finished');
		expect(fake.record).toHaveBeenCalledWith(A, 'position', 600_000);
		expect(player.finishedNow).toBeNull();
		expect(player.notice).toBe(PART_ENDS_HERE);
	});

	test('carries on from the stream when the part ends with the server in reach', async () => {
		fake.session.status = 'offline';
		fake.copy = 'partial';
		await player.open(book(A), { play: true });
		loaded(600);
		fake.session.status = 'signed-in'; // back online meanwhile
		Object.assign(media, { currentTime: 600, ended: true, paused: true });
		fire('ended');
		await vi.advanceTimersByTimeAsync(0);
		expect(fake.record.mock.calls.map((c) => c[1])).not.toContain('finished');
		expect(src()).toBe(`v1/media/${A}`);
		loaded();
		expect(media.currentTime, 'from where the part stopped').toBe(600);
	});

	test('the real end of a complete copy, or of a stream, still finishes the book', async () => {
		fake.session.status = 'offline';
		fake.copy = 'complete';
		await player.open(book(A), { play: true });
		loaded();
		Object.assign(media, { currentTime: 3600, ended: true, paused: true });
		fire('ended');
		expect(fake.record).toHaveBeenCalledWith(A, 'finished');
		expect(player.finishedNow?.id).toBe(A);

		// A stream whose audio is shorter than the library says is still at its end.
		fake.record.mockClear();
		fake.session.status = 'signed-in';
		fake.copy = null;
		await player.open(book(B), { play: true });
		loaded(3500);
		Object.assign(media, { currentTime: 3500, ended: true, paused: true });
		fire('ended');
		expect(fake.record).toHaveBeenCalledWith(B, 'finished');
	});
});

describe('errors after the audio has loaded (review M11)', () => {
	async function fail() {
		loaded();
		media.error = { code: 2 };
		fire('error');
		await vi.advanceTimersByTimeAsync(10_000);
	}

	test('are retried three times, not for ever: loading again is not progress', async () => {
		await player.open(book(A), { play: true });
		for (let i = 0; i < 6; i++) await fail();
		expect(fake.session.refresh, 'one renewal per retry').toHaveBeenCalledTimes(3);
		expect(load, 'the first load and three retries').toHaveBeenCalledTimes(4);
		expect(player.notice).toBe(COULD_NOT_PLAY);
		expect(player.playing).toBe(false);
	});

	test('the count starts again only after half a minute of real playing', async () => {
		await player.open(book(A), { play: true });
		for (let i = 0; i < 2; i++) await fail();
		expect(fake.session.refresh).toHaveBeenCalledTimes(2);
		// 20 s of sound: not enough, so only one retry is left.
		const play = async (seconds: number) => {
			loaded();
			media.paused = false;
			fire('playing');
			for (let i = 0; i < seconds * 4; i++) {
				await vi.advanceTimersByTimeAsync(250);
				media.currentTime += 0.25;
				fire('timeupdate');
			}
		};
		await play(20);
		await fail();
		await fail();
		expect(fake.session.refresh).toHaveBeenCalledTimes(3);
		expect(player.notice).toBe(COULD_NOT_PLAY);
		// Opened again and played for 40 s: an error then is retried afresh.
		await player.open(book(A), { play: true });
		await fail();
		await play(40);
		for (let i = 0; i < 5; i++) await fail();
		expect(fake.session.refresh).toHaveBeenCalledTimes(3 + 1 + 3);
	});
});

describe('work waiting for one book never acts on another (review L22)', () => {
	test('the retry after an error is dropped when another book is opened', async () => {
		await player.open(book(A), { at: 100_000 });
		loaded();
		media.error = { code: 2 };
		fire('error');
		await vi.advanceTimersByTimeAsync(100); // renewed; the retry is now waiting
		await player.open(book(B));
		const loads = load.mock.calls.length;
		await vi.advanceTimersByTimeAsync(10_000);
		expect(load, 'no load at the other book’s position').toHaveBeenCalledTimes(loads);
		expect(src()).toBe(`v1/media/${B}`);
		loaded();
		expect(media.currentTime, 'the second book starts at its own place').toBe(0);
	});

	test('closing the player cancels the retry after an error', async () => {
		await player.open(book(A));
		loaded();
		media.error = { code: 2 };
		fire('error');
		await vi.advanceTimersByTimeAsync(100);
		player.close();
		await vi.advanceTimersByTimeAsync(10_000);
		expect(src()).toBeNull();
		expect(player.book).toBeNull();
	});

	test('closing the player cancels the wait for a book being prepared', async () => {
		fake.api.mockImplementation(async () => ({ ready: false }));
		await player.open(book(A, true));
		expect(player.preparing).toBe(true);
		player.close();
		expect(player.preparing).toBe(false);
		const asked = fake.api.mock.calls.length;
		await vi.advanceTimersByTimeAsync(20_000);
		expect(fake.api, 'the server isn’t asked again').toHaveBeenCalledTimes(asked);
		expect(src()).toBeNull();
	});

	test('an answer about one book’s copy doesn’t load it once another book is open', async () => {
		let answerA: (v: unknown) => void = () => {};
		fake.api.mockImplementationOnce(() => new Promise((r) => (answerA = r)));
		const first = player.open(book(A, true));
		await vi.advanceTimersByTimeAsync(0);
		await player.open(book(B));
		expect(src()).toBe(`v1/media/${B}`);
		answerA({ ready: true });
		await first;
		expect(src(), 'still the second book').toBe(`v1/media/${B}`);
		expect(player.book?.id).toBe(B);
	});
});

test('the sleep timer loses nothing for time spent paused (review L26c)', async () => {
	await player.open(book(A), { play: true });
	loaded();
	await vi.advanceTimersByTimeAsync(5_000); // a page's clock is never at zero
	media.paused = false;
	fire('playing');
	fire('timeupdate');
	player.setSleep({ kind: 'time', remainingMs: 30 * 60_000 });
	// Paused for an hour; a seek while paused brings a tick.
	media.paused = true;
	fire('pause');
	fire('timeupdate');
	await vi.advanceTimersByTimeAsync(HOUR);
	// Playing again: the tick from the rewind's seek arrives before `playing`.
	media.paused = false;
	fire('timeupdate');
	expect(player.sleep).toEqual({ kind: 'time', remainingMs: 30 * 60_000 });
	fire('playing');
	await vi.advanceTimersByTimeAsync(1_000);
	fire('timeupdate');
	expect(player.sleep).toEqual({ kind: 'time', remainingMs: 30 * 60_000 - 1_000 });
});
