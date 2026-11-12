// @vitest-environment node
// How the service worker answers for the app's own files: the stored copy when the network
// fails or a proxy answers 5xx (review M12), and never another build's page in this build's
// cache (review L26d).
import { ownBuild, pageRefs, shellAnswer, type ShellCache } from '$lib/pwa/sw-policy';

const BASE = 'https://books.example.com/app/';
const page = (boot: string, chunk: string) =>
	`<!doctype html><html><head><link href="./_app/immutable/entry/start.${chunk}.js" rel="modulepreload"><link rel="icon" href="./favicon.svg"></head><body><script src="./_app/boot.${boot}.js"></script></body></html>`;
const html = (body: string, status = 200) => new Response(body, { status, headers: { 'content-type': 'text/html' } });
const js = (body: string, status = 200) => new Response(body, { status, headers: { 'content-type': 'text/javascript' } });
/** `type` is 'basic' for a real same-origin answer; one made here is 'default'. */
const basic = (r: Response) => Object.defineProperty(r, 'type', { value: 'basic' });
const keyOf = (k: Request | string) => (typeof k === 'string' ? k : k.url);

/** A cache holding this worker's build: the page and the files it refers to. */
function shellCache(entries: Record<string, Response>) {
	const store = new Map(Object.entries(entries).map(([k, v]) => [new URL(k, BASE).href, v]));
	const cache: ShellCache = {
		match: async (k) => store.get(keyOf(k))?.clone(),
		put: async (k, r) => void store.set(keyOf(k), r),
	};
	return { cache, store };
}
const old = () => shellCache({ '': html(page('aaaa', 'OLD1')), '_app/boot.aaaa.js': js('old boot'), '_app/immutable/entry/start.OLD1.js': js('old start') });
const text = async (r: Response | undefined) => (r ? await r.clone().text() : undefined);

async function ask(o: { path?: string; navigate?: boolean; strategy?: 'cache-first' | 'network-first'; cache: ShellCache; net: () => Promise<Response> }) {
	return shellAnswer({ request: new Request(new URL(o.path ?? '', BASE)), strategy: o.strategy ?? 'network-first', navigate: o.navigate ?? true, cache: o.cache, fetch: o.net, shell: BASE });
}

describe('the network fails or the proxy answers 5xx (review M12)', () => {
	test('a page load gets the stored shell instead of the proxy’s error page', async () => {
		for (const status of [500, 502, 503, 504]) {
			const { cache, store } = old();
			const r = await ask({ cache, net: async () => basic(html('<h1>Server error</h1>', status)) });
			expect(r.status).toBe(200);
			expect(await r.text()).toBe(page('aaaa', 'OLD1'));
			expect(await text(store.get(BASE)), 'the error is never stored').toBe(page('aaaa', 'OLD1'));
		}
	});

	test('any page of the app falls back to the shell; an asset only to its own stored copy', async () => {
		const { cache } = old();
		const down = async () => basic(html('down', 502));
		expect(await (await ask({ path: 'index.html', cache, net: down })).text()).toBe(page('aaaa', 'OLD1'));
		expect(await (await ask({ path: '_app/boot.aaaa.js', navigate: false, cache, net: down })).text()).toBe('old boot');
		const missing = await ask({ path: 'favicon.svg', navigate: false, cache, net: down });
		expect(missing.status, 'nothing stored: the error itself').toBe(502);
	});

	test('with nothing stored at all, the 5xx is passed on; a thrown failure still throws', async () => {
		const { cache } = shellCache({});
		expect((await ask({ cache, net: async () => basic(html('down', 503)) })).status).toBe(503);
		await expect(ask({ cache, net: async () => Promise.reject(new TypeError('Failed to fetch')) })).rejects.toThrow('Failed to fetch');
	});

	test('offline (fetch throws): the stored copy, as before', async () => {
		const { cache } = old();
		const r = await ask({ cache, net: async () => Promise.reject(new TypeError('Failed to fetch')) });
		expect(await r.text()).toBe(page('aaaa', 'OLD1'));
	});

	test('a 4xx is the server’s own answer: passed on, and not stored', async () => {
		const { cache, store } = old();
		const r = await ask({ path: 'favicon.svg', navigate: false, cache, net: async () => basic(new Response('no', { status: 404 })) });
		expect(r.status).toBe(404);
		expect(store.has(new URL('favicon.svg', BASE).href)).toBe(false);
	});

	test('hashed files come from the cache without asking the network', async () => {
		const { cache } = old();
		const net = vi.fn(async () => basic(js('new')));
		const r = await ask({ path: '_app/immutable/entry/start.OLD1.js', navigate: false, strategy: 'cache-first', cache, net });
		expect(await r.text()).toBe('old start');
		expect(net).not.toHaveBeenCalled();
	});
});

describe('after a deploy, the old worker keeps its own shell (review L26d)', () => {
	test('a page finds the app’s own files it refers to, however they are written', () => {
		expect(pageRefs(page('aaaa', 'OLD1'))).toEqual(['_app/immutable/entry/start.OLD1.js', '_app/boot.aaaa.js']);
		expect(pageRefs('<link href="/_app/immutable/chunks/x.js"><script src="_app/boot.1.js"></script><a href="https://example.com/_app/y.js">')).toEqual(['_app/immutable/chunks/x.js', '_app/boot.1.js']);
		expect(pageRefs('<p>no scripts</p>')).toEqual([]);
	});

	test('the new build’s page is served but not stored: the old shell still opens offline', async () => {
		const { cache, store } = old();
		const r = await ask({ cache, net: async () => basic(html(page('bbbb', 'NEW2'))) });
		expect(await r.text(), 'online, the new page is what loads').toBe(page('bbbb', 'NEW2'));
		expect(await text(store.get(BASE)), 'the stored shell is still this worker’s build').toBe(page('aaaa', 'OLD1'));
		// Offline next: the shell served refers only to files this cache holds.
		const offline = await ask({ cache, net: async () => Promise.reject(new TypeError('Failed to fetch')) });
		for (const ref of pageRefs(await offline.text())) expect(store.has(new URL(ref, BASE).href), ref).toBe(true);
	});

	test('the same build’s page is stored afresh, and so are other files', async () => {
		const { cache, store } = old();
		const same = page('aaaa', 'OLD1') + '<!-- served again -->';
		await ask({ cache, net: async () => basic(html(same)) });
		expect(await text(store.get(BASE))).toBe(same);
		await ask({ path: 'manifest.webmanifest', navigate: false, cache, net: async () => basic(new Response('{}', { headers: { 'content-type': 'application/manifest+json' } })) });
		expect(store.has(new URL('manifest.webmanifest', BASE).href)).toBe(true);
	});

	test('a page belongs to this build only if every file it refers to is stored', async () => {
		const { cache } = old();
		expect(await ownBuild(html(page('aaaa', 'OLD1')), cache, BASE)).toBe(true);
		expect(await ownBuild(html(page('aaaa', 'NEW2')), cache, BASE), 'one new chunk is enough').toBe(false);
		expect(await ownBuild(js('anything'), cache, BASE)).toBe(true);
	});
});
