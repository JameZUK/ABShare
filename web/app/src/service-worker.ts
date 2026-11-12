/// <reference no-default-lib="true"/>
/// <reference lib="esnext" />
/// <reference lib="webworker" />
// The app shell offline and quick to start, and downloaded books played from IndexedDB
// (Phase 0 option B). See $lib/pwa/sw-policy for what is never stored.
import { build, version } from '$service-worker';
import { offlineBook, shellAnswer, strategyFor } from '$lib/pwa/sw-policy';
import { answerFromPieces, openDb } from '$lib/offline/db';

const sw = self as unknown as ServiceWorkerGlobalScope;
const CACHE = `abshare-shell-${version}`;
const scope = () => new URL(sw.registration.scope);

sw.addEventListener('install', (event) => {
	event.waitUntil(
		(async () => {
			const cache = await caches.open(CACHE);
			const base = scope();
			const shell = [...build, '', 'manifest.webmanifest', 'favicon.svg', 'icons/icon-192.png'].map((p) => new URL(p.replace(/^\//, ''), base).href);
			await cache.addAll(shell.filter((u) => strategyFor('GET', new URL(u), base) !== 'bypass'));
			// The start-up script is moved out of index.html after the build (strict CSP), so it
			// isn't in `build`: find it in the page just stored and keep it too, or the app
			// wouldn't open offline.
			const page = await (await cache.match(base.href))?.text();
			const boot = [...(page ?? '').matchAll(/src="\.\/(_app\/boot\.[0-9a-f]+\.js)"/g)].map((m) => new URL(m[1]!, base).href);
			if (boot.length) await cache.addAll(boot);
			// A new version waits: the app offers "Update available" instead of swapping mid-listen.
		})(),
	);
});

sw.addEventListener('message', (event) => {
	if (event.data === 'skip-waiting') void sw.skipWaiting();
});

sw.addEventListener('activate', (event) => {
	event.waitUntil(
		(async () => {
			for (const key of await caches.keys()) if (key.startsWith('abshare-shell-') && key !== CACHE) await caches.delete(key);
			await sw.clients.claim();
		})(),
	);
});

async function fromDevice(request: Request, user: string, book: string): Promise<Response> {
	const db = await openDb(user);
	try {
		return await answerFromPieces(db, book, request.headers.get('range'));
	} finally {
		db.close();
	}
}

sw.addEventListener('fetch', (event) => {
	const url = new URL(event.request.url);
	const base = scope();
	const strategy = strategyFor(event.request.method, url, base);
	if (strategy === 'bypass') return; // the browser handles it as if there were no worker
	if (strategy === 'offline-book') {
		const o = offlineBook(url.pathname.slice(base.pathname.length))!;
		event.respondWith(fromDevice(event.request, o.user, o.book));
		return;
	}
	// The rules (stored copy on failure or a 5xx; never another build's page) are in sw-policy.
	event.respondWith(
		caches.open(CACHE).then((cache) => shellAnswer({ request: event.request, strategy, navigate: event.request.mode === 'navigate', cache, fetch: (r) => fetch(r), shell: base.href })),
	);
});
