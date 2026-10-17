// Formatting, saved settings, the service worker's policy, offline Range answers and the
// WebAuthn helpers.
import { bytes, clock, duration, names, timeLeft } from '$lib/format';
import { DEFAULTS, sanitise } from '$lib/settings.svelte';
import { cacheable, offlineBook, strategyFor } from '$lib/pwa/sw-policy';
import { parseRange } from '$lib/offline/db';
import { b64uToBuf, bufToB64u, deviceName } from '$lib/api/webauthn';
import { ApiError, NetworkError, callJson, retryAfterOf } from '$lib/api/http';

const ID = 'abcdefghijklmnopqrstuvwxyz';

test('times, lengths and sizes read naturally', () => {
	expect(clock(0)).toBe('0:00');
	expect(clock(65_000)).toBe('1:05');
	expect(clock(3_723_000)).toBe('1:02:03');
	expect(clock(-5)).toBe('0:00');
	expect(clock(Number.NaN)).toBe('0:00');
	expect(duration(20_000)).toBe('under a minute');
	expect(duration(45 * 60_000)).toBe('45 min');
	expect(duration(2 * 3600_000)).toBe('2 h');
	expect(duration(12 * 3600_000 + 5 * 60_000)).toBe('12 h 5 min');
	expect(timeLeft(3600_000, 4000_000)).toBe('under a minute left');
	expect(bytes(500 * 1024)).toBe('500 KB');
	expect(bytes(1.5 * 1024 ** 3)).toBe('1.5 GB');
	expect(names(['A'])).toBe('A');
	expect(names([{ name: 'A' }, { name: 'B' }, { name: 'C' }])).toBe('A, B and C');
	expect(names(null)).toBe('');
});

test('saved settings are checked; anything odd falls back to defaults', () => {
	expect(sanitise(null)).toEqual(DEFAULTS);
	expect(sanitise({ saverKbps: 40, quality: 'saver', speed: 1.5 })).toMatchObject({ saverKbps: 40, quality: 'saver', speed: 1.5 });
	expect(sanitise({ saverKbps: 64, quality: 'turbo', speed: 99, skipBack: '15', autoRewind: 'yes' })).toEqual(DEFAULTS);
	expect(DEFAULTS.saverKbps, 'data saver defaults to 48 kb/s (D23)').toBe(48);
});

test('the service worker never touches the API, streams, covers or other origins', () => {
	const scope = new URL('https://books.example.com/');
	const at = (p: string, m = 'GET') => strategyFor(m, new URL(p, scope), scope);
	expect(at('v1/books')).toBe('bypass');
	expect(at(`v1/media/${ID}`)).toBe('bypass');
	expect(at(`v1/media/covers/${ID}/0123456789abcdef`)).toBe('bypass');
	expect(at('_app/immutable/x.js?v=1')).toBe('bypass');
	expect(at('index.html', 'POST')).toBe('bypass');
	expect(strategyFor('GET', new URL('https://elsewhere.example/_app/immutable/x.js'), scope)).toBe('bypass');
	expect(at('_app/immutable/chunks/a.js')).toBe('cache-first');
	expect(at('')).toBe('network-first');
	expect(at('manifest.webmanifest')).toBe('network-first');
	expect(at(`offline/joe/${ID}`)).toBe('offline-book');
	expect(at('secret.txt')).toBe('bypass');
	// Under a sub-path, only that sub-path is the app's.
	const sub = new URL('https://books.example.com/ab/');
	expect(strategyFor('GET', new URL('https://books.example.com/other/index.html'), sub)).toBe('bypass');
	expect(strategyFor('GET', new URL('https://books.example.com/ab/v1/books'), sub)).toBe('bypass');
});

test('offline book paths accept only a username and a book ID', () => {
	expect(offlineBook(`offline/joe/${ID}`)).toEqual({ user: 'joe', book: ID });
	for (const bad of [`offline/../${ID}`, `offline/joe/${ID}/x`, 'offline/joe/short', `offline/JOE/${ID}`, `offline/jo/${ID}`, `offline/joe/${ID.toUpperCase()}`]) {
		expect(offlineBook(bad), bad).toBeNull();
	}
});

test('only plain 200 same-origin answers are cached', () => {
	const r = (status: number, cc = '', type: ResponseType = 'basic') => {
		const x = new Response('x', { status, headers: { 'cache-control': cc } });
		Object.defineProperty(x, 'type', { value: type });
		return x;
	};
	expect(cacheable(r(200))).toBe(true);
	expect(cacheable(r(200, 'private, no-store'))).toBe(false);
	expect(cacheable(r(206))).toBe(false);
	expect(cacheable(r(200, '', 'opaque'))).toBe(false);
});

test('offline Range requests are answered like a server would', () => {
	expect(parseRange(null, 100)).toBeNull();
	expect(parseRange('bytes=0-9', 100)).toEqual([0, 9]);
	expect(parseRange('bytes=90-', 100)).toEqual([90, 99]);
	expect(parseRange('bytes=-10', 100)).toEqual([90, 99]);
	expect(parseRange('bytes=-1000', 100)).toEqual([0, 99]);
	expect(parseRange('bytes=50-5000', 100)).toEqual([50, 99]);
	for (const bad of ['bytes=100-', 'bytes=-', 'bytes=9-1', 'items=0-1', 'bytes=0-1,5-6', 'bytes=a-b']) expect(parseRange(bad, 100), bad).toBe('bad');
});

test('base64url round-trips and device names are readable', () => {
	const bytesIn = new Uint8Array([0, 1, 250, 251, 252, 253, 254, 255]);
	expect(new Uint8Array(b64uToBuf(bufToB64u(bytesIn.buffer)))).toEqual(bytesIn);
	expect(bufToB64u(new Uint8Array([251, 255]).buffer)).toBe('-_8');
	expect(deviceName('Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1', true)).toBe('iPhone · Safari');
	expect(deviceName('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15', true)).toBe('iPad · Safari');
	expect(deviceName('Mozilla/5.0 (Android 15; Mobile; rv:140.0) Gecko/140.0 Firefox/140.0', true)).toBe('Android · Firefox');
	expect(deviceName('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/140.0 Safari/537.36 Edg/140.0', false)).toBe('Windows · Edge');
	expect(deviceName('', false)).toBe('Device · Browser');
});

describe('callJson', () => {
	afterEach(() => vi.unstubAllGlobals());
	const stub = (impl: () => Promise<Response>) => {
		const f = vi.fn((_url: string, _init?: RequestInit) => impl());
		vi.stubGlobal('fetch', f);
		return f;
	};

	test('sends the CSRF header and parses JSON', async () => {
		const f = stub(async () => new Response('{"ok":true}', { status: 200 }));
		expect(await callJson('v1/x', { method: 'POST', body: '{}' })).toEqual({ ok: true });
		const init = f.mock.calls[0]![1]!;
		expect((init.headers as Record<string, string>)['X-ABShare-Client']).toBe('1');
		expect(init.cache).toBe('no-store');
		expect(init.credentials).toBe('same-origin');
	});

	test('turns refusals into ApiError with the server’s code and message', async () => {
		stub(async () => new Response('{"error":"step_up_required","message":"Confirm it’s you first."}', { status: 403, headers: { 'retry-after': '7' } }));
		const e = (await callJson('v1/x').catch((x: unknown) => x)) as ApiError;
		expect(e).toBeInstanceOf(ApiError);
		expect(e.code).toBe('step_up_required');
		expect(e.status).toBe(403);
		expect(e.retryAfter).toBe(7);
	});

	test('a success that isn’t JSON is not taken as an answer', async () => {
		stub(async () => new Response('<html>proxy page</html>', { status: 200 }));
		await expect(callJson('v1/x')).rejects.toBeInstanceOf(ApiError);
	});

	test('no connection is a NetworkError', async () => {
		stub(async () => {
			throw new TypeError('Failed to fetch');
		});
		await expect(callJson('v1/x')).rejects.toBeInstanceOf(NetworkError);
	});

	test('Retry-After is bounded', () => {
		expect(retryAfterOf('30')).toBe(30);
		expect(retryAfterOf('0')).toBe(1);
		expect(retryAfterOf('99999999')).toBe(86_400);
		expect(retryAfterOf('Wed, 21 Oct 2015 07:28:00 GMT')).toBeUndefined();
		expect(retryAfterOf(null)).toBeUndefined();
	});
});
