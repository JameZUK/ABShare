// The session: a refresh refused only for being too frequent (429) must not sign anyone out.
import { session } from '$lib/api/session.svelte';

const signedIn = { access_token: 'a'.repeat(64), expires_in: 900, user: { username: 'ada', display_name: 'Ada', role: 'listener', needs_factor: false } };
const answer = (status: number, body: unknown, headers: Record<string, string> = {}) =>
	new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });

afterEach(() => vi.unstubAllGlobals());

test('a rate-limited refresh at start-up waits and tries again instead of signing out', async () => {
	const calls: string[] = [];
	const replies = [answer(429, { error: 'too_many', message: 'Slow down.' }, { 'retry-after': '1' }), answer(200, signedIn)];
	vi.stubGlobal('fetch', async (url: string) => {
		calls.push(String(url));
		return replies.shift() ?? answer(500, {});
	});
	await session.start();
	expect(calls.filter((u) => u.endsWith('v1/auth/refresh'))).toHaveLength(2);
	expect(session.status).toBe('signed-in');
	expect(session.user?.username).toBe('ada');
}, 10_000);

test('a refresh that keeps being refused for frequency gives up, then signs out', async () => {
	await session.signOut(true).catch(() => {});
	let n = 0;
	vi.stubGlobal('fetch', async () => {
		n++;
		return answer(429, { error: 'too_many', message: 'Slow down.' }, { 'retry-after': '1' });
	});
	await session.start();
	expect(n).toBe(5);
	expect(session.status).toBe('signed-out');
}, 15_000);

test('a revocation answered to the retry after a 429 still removes the downloads (review W12)', async () => {
	await session.signOut(true).catch(() => {});
	const ended: string[] = [];
	const before = session.onEnd;
	session.onEnd = (reason) => void ended.push(reason);
	const replies = [
		answer(429, { error: 'too_many', message: 'Slow down.' }, { 'retry-after': '1' }),
		answer(401, { error: 'device_revoked', message: 'Signed out remotely.' }),
	];
	vi.stubGlobal('fetch', async () => replies.shift() ?? answer(500, {}));
	try {
		await session.start();
		expect(ended).toEqual(['revoked']);
		expect(session.status).toBe('signed-out');
	} finally {
		session.onEnd = before;
	}
}, 10_000);

test('an outage during the wait after a 429 goes offline instead of signing out (review W12)', async () => {
	await session.signOut(true).catch(() => {});
	localStorage.setItem('abshare.lastUser', JSON.stringify(signedIn.user));
	const replies = [answer(429, { error: 'too_many', message: 'Slow down.' }, { 'retry-after': '1' })];
	vi.stubGlobal('fetch', async () => {
		const r = replies.shift();
		if (!r) throw new TypeError('Failed to fetch');
		return r;
	});
	await session.start();
	expect(session.status).toBe('offline');
	expect(session.user?.username).toBe('ada');
}, 10_000);

// Slow connections: a slow answer at start-up opens the last person's app as "connecting",
// never "offline"; only a failed request goes offline.
describe('a slow answer at start-up', () => {
	/** A fetch whose refresh answer arrives only when `reply` is called. */
	function slowFetch() {
		let reply: (r: Response | Error) => void = () => {};
		vi.stubGlobal('fetch', () => new Promise<Response>((ok, fail) => (reply = (r) => (r instanceof Error ? fail(r) : ok(r)))));
		return (r: Response | Error) => reply(r);
	}

	beforeEach(async () => {
		vi.stubGlobal('fetch', () => Promise.reject(new TypeError('Failed to fetch')));
		await session.signOut(true).catch(() => {});
		vi.unstubAllGlobals();
		// A page starts once; this shared session starts again for each test.
		session.status = 'starting';
		session.user = null;
		vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
	});
	afterEach(() => vi.useRealTimers());

	test('opens the last person’s app as connecting, then signs in when the answer comes', async () => {
		localStorage.setItem('abshare.lastUser', JSON.stringify(signedIn.user));
		const reply = slowFetch();
		const started = session.start();
		await vi.advanceTimersByTimeAsync(2_000);
		expect(session.status).toBe('starting');
		await vi.advanceTimersByTimeAsync(1_500);
		expect(session.status).toBe('connecting');
		expect(session.user?.username).toBe('ada');
		// However long it takes, it stays connecting: nothing times out into offline.
		await vi.advanceTimersByTimeAsync(10 * 60_000);
		expect(session.status).toBe('connecting');
		reply(answer(200, signedIn));
		await started;
		expect(session.status).toBe('signed-in');
	});

	test('with no one signed in before on this device, it stays on the start screen', async () => {
		const reply = slowFetch();
		const started = session.start();
		await vi.advanceTimersByTimeAsync(60_000);
		expect(session.status).toBe('starting');
		reply(answer(200, signedIn));
		await started;
		expect(session.status).toBe('signed-in');
	});

	test('goes offline only when the request actually fails', async () => {
		localStorage.setItem('abshare.lastUser', JSON.stringify(signedIn.user));
		const reply = slowFetch();
		const started = session.start();
		await vi.advanceTimersByTimeAsync(5_000);
		expect(session.status).toBe('connecting');
		reply(new TypeError('Failed to fetch'));
		await started;
		expect(session.status).toBe('offline');
	});

	test('a sign-in refused while connecting ends what was started (a downloaded book playing)', async () => {
		localStorage.setItem('abshare.lastUser', JSON.stringify(signedIn.user));
		const ended: string[] = [];
		const before = session.onEnd;
		session.onEnd = (reason, user) => void ended.push(`${reason}:${user}`);
		try {
			const reply = slowFetch();
			const started = session.start();
			await vi.advanceTimersByTimeAsync(5_000);
			expect(session.status).toBe('connecting');
			reply(answer(401, { error: 'unauthenticated', message: 'Please sign in.' }));
			await started;
			expect(session.status).toBe('signed-out');
			expect(ended).toEqual(['expired:ada']);
		} finally {
			session.onEnd = before;
		}
	});

	test('someone else answering ends the last person’s app before signing in', async () => {
		localStorage.setItem('abshare.lastUser', JSON.stringify(signedIn.user));
		const ended: string[] = [];
		const before = session.onEnd;
		session.onEnd = (reason, user) => void ended.push(`${reason}:${user}`);
		try {
			const reply = slowFetch();
			const started = session.start();
			await vi.advanceTimersByTimeAsync(5_000);
			reply(answer(200, { ...signedIn, user: { ...signedIn.user, username: 'grace', display_name: 'Grace' } }));
			await started;
			expect(ended).toEqual(['signed-out:ada']);
			expect(session.status).toBe('signed-in');
			expect(session.user?.username).toBe('grace');
		} finally {
			session.onEnd = before;
		}
	});
});
