// Registers the service worker and notices updates. A new version waits until the listener
// chooses to reload, so nothing swaps mid-listen.
import { dev } from '$app/environment';

/** The worker's URL. Trusted Types (enforced by the CSP) only accept it through this policy,
 *  which allows that one same-origin script and nothing else. */
export function workerUrl(base: string): string {
	const url = new URL('./service-worker.js', base).href;
	type TT = { createPolicy: (n: string, p: { createScriptURL: (s: string) => string }) => { createScriptURL: (s: string) => unknown } };
	const tt = (globalThis as { trustedTypes?: TT }).trustedTypes;
	if (!tt) return url;
	const policy = tt.createPolicy('abshare-sw', {
		createScriptURL: (s) => {
			if (s !== url) throw new TypeError('not the ABShare service worker');
			return s;
		},
	});
	return policy.createScriptURL(url) as string;
}

class Updates {
	available = $state(false);
	#waiting: ServiceWorker | null = null;

	start() {
		if (!('serviceWorker' in navigator)) return;
		// Registering fetches the script, which fails offline: only register when no worker
		// controls the page yet, or when online (Phase 0 finding).
		if (navigator.serviceWorker.controller && !navigator.onLine) return;
		navigator.serviceWorker
			.register(workerUrl(document.baseURI), { type: dev ? 'module' : 'classic' })
			.then((reg) => {
				const watch = (w: ServiceWorker | null) => {
					if (!w) return;
					w.addEventListener('statechange', () => {
						if (w.state === 'installed' && navigator.serviceWorker.controller) this.#offer(w);
					});
				};
				if (reg.waiting && navigator.serviceWorker.controller) this.#offer(reg.waiting);
				watch(reg.installing);
				reg.addEventListener('updatefound', () => watch(reg.installing));
			})
			.catch(() => {
				/* the app works without it, minus offline */
			});
		let reloading = false;
		navigator.serviceWorker.addEventListener('controllerchange', () => {
			if (this.#waiting && !reloading) {
				reloading = true;
				location.reload();
			}
		});
	}

	#offer(w: ServiceWorker) {
		this.#waiting = w;
		this.available = true;
	}

	apply() {
		this.#waiting?.postMessage('skip-waiting');
	}
}

export const updates = new Updates();
