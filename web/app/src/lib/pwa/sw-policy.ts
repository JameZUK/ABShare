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
