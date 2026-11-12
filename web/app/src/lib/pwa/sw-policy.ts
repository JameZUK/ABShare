// What the service worker may do with a request (SPEC §7.4). It makes the app itself start
// fast and open offline, and plays downloaded books. API calls, streams and covers always
// go to the network and are never stored: they carry personal data or change constantly.

export type Strategy = 'cache-first' | 'network-first' | 'offline-book' | 'bypass';

/** `offline/<user>/<book>`: a downloaded book, answered from IndexedDB. */
export function offlineBook(path: string): { user: string; book: string } | null {
	const m = /^offline\/([a-z0-9][a-z0-9._-]{2,31})\/([a-z2-7]{26})$/.exec(path);
	return m ? { user: m[1]!, book: m[2]! } : null;
}

/**
 * `url` is the request; `scope` the service worker's scope (the app's base URL, which may be
 * a sub-path behind a proxy).
 */
export function strategyFor(method: string, url: URL, scope: URL): Strategy {
	if (method !== 'GET' || url.origin !== scope.origin || !url.pathname.startsWith(scope.pathname)) return 'bypass';
	if (url.search) return 'bypass'; // the app's own files never take a query
	const path = url.pathname.slice(scope.pathname.length);
	if (/^v1(\/|$)/.test(path)) return 'bypass'; // the API, streams and covers
	if (offlineBook(path)) return 'offline-book';
	if (path.startsWith('_app/immutable/')) return 'cache-first'; // hashed: never change
	if (path === '' || path === 'index.html' || /^(_app\/[^/]+|icons\/[\w.-]+|favicon\.svg|manifest\.webmanifest)$/.test(path)) {
		return 'network-first';
	}
	return 'bypass';
}

/** Only complete, ordinary, same-origin answers are worth keeping. */
export const cacheable = (r: Response) => r.ok && r.status === 200 && r.type === 'basic' && !/no-store/i.test(r.headers.get('cache-control') ?? '');

/** The app's own files (under `_app/`) that a page refers to, as paths relative to the scope. */
export function pageRefs(html: string): string[] {
	return [...html.matchAll(/\b(?:src|href)="(?:\.?\/)?(_app\/[^"?#]+)"/g)].map((m) => m[1]!);
}

/** The part of the Cache API the shell needs. */
export interface ShellCache {
	match(key: Request | string): Promise<Response | undefined>;
	put(key: Request | string, r: Response): Promise<void>;
}

/**
 * Whether a freshly fetched answer belongs to the build this worker stored at install. A
 * page does only if every file of the app it refers to is already stored. After a deploy,
 * the old worker (still in charge until the update is taken) fetches the new page; storing
 * it would leave a shell whose lazily loaded parts were never stored, and the app wouldn't
 * open offline. Anything that isn't a page belongs.
 */
export async function ownBuild(fresh: Response, cache: ShellCache, shell: string): Promise<boolean> {
	if (!/^text\/html\b/i.test(fresh.headers.get('content-type') ?? '')) return true;
	const html = await fresh.clone().text();
	for (const ref of pageRefs(html)) if (!(await cache.match(new URL(ref, shell).href))) return false;
	return true;
}

/**
 * Answers a request for one of the app's own files: from the cache first (hashed files), or
 * from the network, falling back to the stored copy (for a page load, the app shell) when the
 * network fails or the answer is a server error (a reverse proxy's 502 while the server is
 * down). Server errors are never stored, nor is a page from another build.
 */
export async function shellAnswer(o: {
	request: Request;
	strategy: 'cache-first' | 'network-first';
	/** A page load (`request.mode === 'navigate'`). */
	navigate: boolean;
	cache: ShellCache;
	fetch: (r: Request) => Promise<Response>;
	/** The app shell's URL (the worker's scope). */
	shell: string;
}): Promise<Response> {
	const { request, cache } = o;
	if (o.strategy === 'cache-first') {
		const hit = await cache.match(request);
		if (hit) return hit;
	}
	const stored = async () => (await cache.match(request)) ?? (o.navigate ? await cache.match(o.shell) : undefined);
	let fresh: Response;
	try {
		fresh = await o.fetch(request);
	} catch (e) {
		// Offline: the stored copy.
		const s = await stored();
		if (s) return s;
		throw e;
	}
	if (fresh.status >= 500) return (await stored()) ?? fresh;
	if (cacheable(fresh) && (await ownBuild(fresh, cache, o.shell))) await cache.put(request, fresh.clone());
	return fresh;
}
