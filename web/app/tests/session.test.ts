// The session: a refresh refused only for being too frequent (429) must not sign anyone out.
import { session } from '$lib/api/session.svelte';
import type { SignedIn } from '$lib/api/types';

const signedIn = { access_token: 'a'.repeat(64), expires_in: 900, user: { username: 'ada', display_name: 'Ada', role: 'listener', needs_factor: false } };
const answer = (status: number, body: unknown, headers: Record<string, string> = {}) =>
	new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', ...headers } });

afterEach(() => vi.unstubAllGlobals());

/** Between tests: a sign-out the server confirms, forgetting who was here. */
async function reset() {
	vi.stubGlobal('fetch', async () => new Response(null, { status: 204 }));
	await session.signOut(true);
	vi.unstubAllGlobals();
}

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
	await reset();
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
	await reset();
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
	await reset();
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
		await reset();
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

// Signing out must reach the server (review H2): the refresh cookie is HttpOnly, so only the
// server can end the session. Until it confirms, nothing may renew the session.
describe('a sign-out the server has not confirmed', () => {
	const PENDING = 'abshare.signOutPending';
	/** A server with a session cookie: renewals work until a sign-out reaches it. */
	function server(opts: { logout?: () => Response | Error } = {}) {
		const s = { cookie: true, calls: [] as string[] };
		vi.stubGlobal('fetch', async (url: string) => {
			const path = String(url).replace(/^.*v1\/auth\//, '');
			s.calls.push(path);
			if (path === 'logout') {
				const r = opts.logout?.() ?? new Response(null, { status: 204 });
				if (r instanceof Error) throw r;
				if (r.status === 204) s.cookie = false;
				return r;
			}
			return s.cookie ? answer(200, signedIn) : answer(401, { error: 'unauthenticated', message: 'Please sign in.' });
		});
		return s;
	}
	const down = () => new TypeError('Failed to fetch');
	/** A new page: the shared session starts again. */
	function newPage() {
		session.status = 'starting';
		session.user = null;
	}

	beforeEach(async () => {
		await reset();
		newPage();
	});
	afterEach(() => localStorage.removeItem(PENDING));

	test('is sent again at the next start, before the renewal, which then signs nobody in', async () => {
		let reachable = false;
		const s = server({ logout: () => (reachable ? new Response(null, { status: 204 }) : down()) });
		await session.start();
		expect(session.status).toBe('signed-in');
		await session.signOut(false);
		expect(session.status).toBe('signed-out');
		expect(session.token).toBeNull();
		expect(localStorage.getItem(PENDING), 'remembered until the server confirms').not.toBeNull();
		expect(s.cookie, 'the server never heard').toBe(true);

		// The next start, online again: without the marker the cookie would sign them back in.
		reachable = true;
		newPage();
		s.calls.length = 0;
		await session.start();
		expect(s.calls).toEqual(['logout', 'refresh']);
		expect(session.status).toBe('signed-out');
		expect(session.token).toBeNull();
		expect(s.cookie).toBe(false);
		expect(localStorage.getItem(PENDING)).toBeNull();
	});

	test('while the server still can’t be reached, nothing is renewed and nobody’s app opens', async () => {
		const s = server({ logout: down });
		await session.start();
		await session.signOut(true);
		newPage();
		s.calls.length = 0;
		await session.start();
		expect(s.calls, 'no renewal').toEqual(['logout']);
		expect(session.status).toBe('signed-out');
		expect(session.token).toBeNull();
		expect(localStorage.getItem(PENDING)).not.toBeNull();
	});

	test('a refusal (not a confirmation) keeps it pending and signs nobody in', async () => {
		for (const refusal of [() => answer(429, { error: 'too_many', message: 'Slow down.' }), () => answer(403, { error: 'denied', message: 'No.' })]) {
			localStorage.setItem('abshare.lastUser', JSON.stringify(signedIn.user));
			localStorage.setItem(PENDING, '1');
			const s = server({ logout: refusal });
			newPage();
			await session.start();
			expect(s.calls).toEqual(['logout']);
			expect(session.status).toBe('signed-out');
			expect(session.user).toBeNull();
			expect(localStorage.getItem(PENDING)).not.toBeNull();
		}
	});

	test('unreachable with downloads kept: the app opens offline as before, but nothing is renewed', async () => {
		// As after any sign-out that kept downloads: with no server, they still play.
		for (const outage of [down, () => answer(502, {}), () => new Response('<html>a proxy’s page</html>', { status: 200 })]) {
			localStorage.setItem('abshare.lastUser', JSON.stringify(signedIn.user));
			localStorage.setItem(PENDING, '1');
			const s = server({ logout: outage });
			newPage();
			await session.start();
			expect(s.calls, 'no renewal').toEqual(['logout']);
			expect(session.status).toBe('offline');
			expect(session.token).toBeNull();
			expect(s.cookie).toBe(true);
			expect(localStorage.getItem(PENDING)).not.toBeNull();
		}
	});

	test('401 from the server (already signed out) confirms it', async () => {
		localStorage.setItem(PENDING, '1');
		const s = server({ logout: () => answer(401, { error: 'unauthenticated', message: 'Please sign in.' }) });
		expect(await session.finishSignOut()).toBe(true);
		expect(s.calls).toEqual(['logout']);
		expect(localStorage.getItem(PENDING)).toBeNull();
	});

	test('a renewal already on its way when another tab signs out is not taken', async () => {
		const s = server();
		let release: () => void = () => {};
		const real = globalThis.fetch;
		vi.stubGlobal('fetch', async (url: string) => {
			if (s.calls.length === 0) await new Promise<void>((r) => (release = r));
			return real(url);
		});
		const started = session.start();
		await new Promise((r) => setTimeout(r, 10));
		localStorage.setItem(PENDING, '1'); // the other tab's sign-out, not yet confirmed
		release();
		await started;
		expect(s.calls).toEqual(['refresh', 'logout', 'refresh']);
		expect(session.status).toBe('signed-out');
		expect(session.token).toBeNull();
		expect(s.cookie).toBe(false);
	});

	test('another tab signing out ends this tab’s session at once', async () => {
		server();
		await session.start();
		expect(session.status).toBe('signed-in');
		const ended: string[] = [];
		const before = session.onEnd;
		session.onEnd = (reason, user) => void ended.push(`${reason}:${user}`);
		try {
			window.dispatchEvent(new StorageEvent('storage', { key: 'abshare.lastUser', newValue: 'x' }));
			expect(session.status, 'only the sign-out marker counts').toBe('signed-in');
			window.dispatchEvent(new StorageEvent('storage', { key: PENDING, newValue: '1' }));
			expect(session.status).toBe('signed-out');
			expect(session.token).toBeNull();
			expect(session.user).toBeNull();
			expect(ended).toEqual(['signed-out:ada']);
		} finally {
			session.onEnd = before;
		}
	});

	test('it goes as soon as the connection is back', async () => {
		let reachable = false;
		const s = server({ logout: () => (reachable ? new Response(null, { status: 204 }) : down()) });
		await session.start();
		await session.signOut(false);
		expect(s.cookie).toBe(true);
		reachable = true;
		window.dispatchEvent(new Event('online'));
		await vi.waitFor(() => expect(localStorage.getItem(PENDING)).toBeNull());
		expect(s.cookie).toBe(false);
	});

	test('a person signing in afresh clears it: the server has started a new session', async () => {
		const s = server({ logout: down });
		await session.start();
		await session.signOut(false);
		expect(localStorage.getItem(PENDING)).not.toBeNull();
		session.accept(signedIn as SignedIn); // as the sign-in form does with the server's answer
		expect(localStorage.getItem(PENDING)).toBeNull();
		expect(session.status).toBe('signed-in');
		s.calls.length = 0;
		expect(await session.refresh()).toBe(true);
		expect(s.calls, 'renewals work again, with no sign-out sent').toEqual(['refresh']);
	});
});

test('a renewal refused for frequency with a token in hand is tried again later (review L26a)', async () => {
	await reset();
	vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
	try {
		vi.stubGlobal('fetch', async () => answer(200, signedIn));
		session.status = 'starting';
		await session.start();
		let n = 0;
		vi.stubGlobal('fetch', async () => {
			n++;
			return answer(429, { error: 'too_many', message: 'Slow down.' }, { 'retry-after': '20' });
		});
		// The renewal timer (a minute before the 900 s token lapses) fires, and is refused.
		await vi.advanceTimersByTimeAsync(841_000);
		expect(n).toBe(1);
		expect(session.status).toBe('signed-in');
		// It must be set again, or the token and media cookie lapse mid-listen.
		await vi.advanceTimersByTimeAsync(25_000);
		expect(n).toBe(2);
		expect(session.status).toBe('signed-in');
	} finally {
		vi.useRealTimers();
		await reset();
	}
});
