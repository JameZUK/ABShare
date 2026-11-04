// The playback log (D38): kept in this browser, at most 38 lines, never which book, and sent only
// when the listener asks, as the signed-in listener (POST /v1/diag).
import { forgetLog, KEEP, logEvent, place, playbackLog, sendReport } from '$lib/player/log';

const { post } = vi.hoisted(() => ({ post: vi.fn() }));
vi.mock('$lib/api/session.svelte', () => ({ post }));

beforeEach(() => {
	localStorage.clear();
	forgetLog();
	vi.restoreAllMocks();
	post.mockReset();
});

test('places in the book read as hours, minutes and seconds', () => {
	expect(place(0)).toBe('0:00');
	expect(place(65_000)).toBe('1:05');
	expect(place(3_723_000)).toBe('1:02:03');
	expect(place(-5)).toBe('0:00');
});

test('events are dated, kept in this browser, and only the newest are kept', () => {
	logEvent('pause (by the browser) @ 1:05', new Date(2026, 10, 4, 9, 5, 7));
	expect(playbackLog()).toEqual(['11-04 09:05:07 pause (by the browser) @ 1:05']);
	expect(JSON.parse(localStorage.getItem('abshare.playbackLog')!)).toEqual(playbackLog());
	for (let i = 0; i < KEEP + 10; i++) logEvent(`event ${i}`);
	const l = playbackLog();
	expect(l).toHaveLength(KEEP);
	expect(l.at(-1)).toMatch(/event 47$/);
	expect(l[0]).toMatch(/event 10$/);
});

test('a very long line is cut, so a report stays within the route’s limits', () => {
	logEvent('x'.repeat(1000));
	expect(playbackLog()[0]!.length).toBe(200);
});

test('forgetting the log (at sign-out) empties it here and in storage', () => {
	logEvent('play (app) @ 0:00');
	forgetLog();
	expect(playbackLog()).toEqual([]);
	expect(localStorage.getItem('abshare.playbackLog')).toBeNull();
});

test('a report posts the log as the signed-in listener, within the route’s limits', async () => {
	logEvent('play (app) @ 0:00');
	post.mockResolvedValue(null);
	expect(await sendReport()).toBe(true);
	expect(post).toHaveBeenCalledTimes(1);
	const [path, body] = post.mock.calls[0] as [string, { lines: string[] }];
	expect(path).toBe('v1/diag');
	expect(body.lines[0]).toMatch(/^Playback report/);
	expect(body.lines[1]).toMatch(/^Browser: /);
	expect(body.lines.slice(2)).toEqual(playbackLog());
	expect(body.lines.length, 'within the route’s 40 lines').toBeLessThanOrEqual(40);
});

test('a refused or failed report says so', async () => {
	post.mockRejectedValue(new Error('429'));
	expect(await sendReport()).toBe(false);
});

test('whatever is in storage, only lines of text are kept', async () => {
	for (const [stored, kept] of [['{"a":1}', []], ['[1,"ok",null]', ['ok']], ['not json', []]] as const) {
		localStorage.setItem('abshare.playbackLog', stored);
		vi.resetModules();
		const fresh = await import('$lib/player/log');
		expect(fresh.playbackLog()).toEqual(kept);
	}
});
