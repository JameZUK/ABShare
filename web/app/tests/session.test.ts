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
