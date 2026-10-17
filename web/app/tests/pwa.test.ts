// The app must start where the browser offers no service worker: Chrome throws when the site
// may not store data, and some browsers leave navigator.serviceWorker undefined. Both used to
// crash the app to a blank page before it drew anything.
import { cookiesBlocked, swContainer, updates } from '$lib/pwa/update.svelte';

function withServiceWorker(get: () => unknown, run: () => void) {
	const had = Object.getOwnPropertyDescriptor(navigator, 'serviceWorker');
	Object.defineProperty(navigator, 'serviceWorker', { configurable: true, get });
	try {
		run();
	} finally {
		if (had) Object.defineProperty(navigator, 'serviceWorker', had);
		else delete (navigator as unknown as Record<string, unknown>).serviceWorker;
	}
}

const denied = () => {
	throw new DOMException('Access to service workers is denied in this document origin.', 'SecurityError');
};

test('no service worker container: the app carries on without offline', () => {
	withServiceWorker(
		() => undefined,
		() => {
			expect(swContainer()).toBeNull();
			expect(() => updates.start()).not.toThrow();
		},
	);
});

test('reading navigator.serviceWorker throws: the app carries on without offline', () => {
	withServiceWorker(denied, () => {
		expect(swContainer()).toBeNull();
		expect(() => updates.start()).not.toThrow();
	});
});

test('blocked cookies are noticed, so the app can say so', () => {
	expect(cookiesBlocked({ cookieEnabled: false } as Navigator)).toBe(true);
	expect(cookiesBlocked({ cookieEnabled: true } as Navigator)).toBe(false);
	expect(cookiesBlocked(undefined)).toBe(false);
});
