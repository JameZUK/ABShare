// static/diag.js: a blank page shows what went wrong (as text) instead of staying white, and
// a page that drew normally is left alone unless ?diag asks.
import { readFileSync } from 'node:fs';

const source = readFileSync('static/diag.js', 'utf8');

function load() {
	// The file is a classic script; run it in the global scope, as the page does. (jsdom
	// doesn't execute inserted <script> elements.)
	(0, eval)(source);
}

function overlay(): HTMLElement | null {
	return document.documentElement.querySelector(':scope > pre[role="alert"]');
}

beforeEach(() => {
	vi.useFakeTimers();
	overlay()?.remove();
	document.body.replaceChildren();
	history.replaceState(null, '', '/');
});
afterEach(() => vi.useRealTimers());

test('a blank page shows the diagnostics, with the error that blanked it', () => {
	load();
	window.dispatchEvent(new ErrorEvent('error', { message: 'TypeError: boom', filename: 'https://books.example.com/_app/x.js', lineno: 7 }));
	vi.advanceTimersByTime(8000);
	const box = overlay();
	expect(box).not.toBeNull();
	expect(box!.textContent).toContain('could not start');
	expect(box!.textContent).toContain('TypeError: boom (x.js:7)');
	// Text only: nothing in it is parsed as markup.
	expect(box!.children.length).toBe(0);
});

test('a page that drew is left alone', () => {
	load();
	const main = document.createElement('main');
	main.textContent = 'Welcome';
	document.body.appendChild(main);
	// jsdom has no layout, so innerText is undefined there; give it what a browser reports.
	Object.defineProperty(document.body, 'innerText', { configurable: true, get: () => 'Welcome' });
	vi.advanceTimersByTime(8000);
	expect(overlay()).toBeNull();
	delete (document.body as unknown as Record<string, unknown>).innerText;
});

test('?diag shows the diagnostics even when the page drew', () => {
	history.replaceState(null, '', '/?diag');
	load();
	Object.defineProperty(document.body, 'innerText', { configurable: true, get: () => 'Welcome' });
	vi.advanceTimersByTime(4000);
	expect(overlay()?.textContent).toContain('ABShare diagnostics');
	delete (document.body as unknown as Record<string, unknown>).innerText;
});

test('the report is sent to the server once, same-origin, with the app header', () => {
	const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 204 })));
	vi.stubGlobal('fetch', fetchMock);
	try {
		load();
		vi.advanceTimersByTime(8000);
		expect(fetchMock).toHaveBeenCalledTimes(1);
		const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
		expect(new URL(url).pathname).toBe('/v1/diag');
		expect(new URL(url).origin).toBe(location.origin);
		expect((init.headers as Record<string, string>)['x-abshare-client']).toBe('1');
		expect(init.credentials).toBe('omit');
		const body = JSON.parse(String(init.body)) as { lines: string[] };
		expect(body.lines.length).toBeLessThanOrEqual(40);
		expect(body.lines[0]).toContain('could not start');
	} finally {
		vi.unstubAllGlobals();
	}
});

test('text no one can see counts as blank', () => {
	load();
	const app = document.createElement('div');
	app.id = 'app';
	const h1 = document.createElement('h1');
	h1.textContent = 'Welcome';
	app.appendChild(h1);
	document.body.appendChild(app);
	Object.defineProperty(document.body, 'innerText', { configurable: true, get: () => 'Welcome' });
	// jsdom lays nothing out: the heading has no size, as an off-screen or collapsed one would.
	vi.advanceTimersByTime(8000);
	expect(overlay()?.textContent).toContain('(NOT VISIBLE)');
	delete (document.body as unknown as Record<string, unknown>).innerText;
});

// Slow connections and phones in pockets: what isn't a failure isn't reported as one.
describe('not a failure', () => {
	function setHidden(hidden: boolean) {
		Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
		Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => (hidden ? 'hidden' : 'visible') });
		document.dispatchEvent(new Event('visibilitychange'));
	}
	afterEach(() => {
		delete (document as unknown as Record<string, unknown>).hidden;
		delete (document as unknown as Record<string, unknown>).visibilityState;
		delete (document as unknown as Record<string, unknown>).getAnimations;
		vi.unstubAllGlobals();
	});

	test('a page in the background is judged only after being on screen for the usual time', () => {
		setHidden(true);
		load();
		vi.advanceTimersByTime(120_000);
		expect(overlay()).toBeNull();
		setHidden(false);
		vi.advanceTimersByTime(7_000);
		expect(overlay()).toBeNull();
		vi.advanceTimersByTime(1_000);
		// Still blank once seen: that is a real failure, and it says it was visible.
		expect(overlay()?.textContent).toContain('Visible: visible');
	});

	test('a fade still running is waited for', () => {
		let running = true;
		Object.defineProperty(document, 'getAnimations', {
			configurable: true,
			value: () => (running ? [{ playState: 'running', effect: { getComputedTiming: () => ({ endTime: 280 }) } }] : []),
		});
		load();
		vi.advanceTimersByTime(8_000);
		expect(overlay()).toBeNull();
		running = false;
		vi.advanceTimersByTime(1_000);
		expect(overlay()).not.toBeNull();
	});

	test('a spinner that never ends doesn’t hold the check up', () => {
		Object.defineProperty(document, 'getAnimations', {
			configurable: true,
			value: () => [{ playState: 'running', effect: { getComputedTiming: () => ({ endTime: Infinity }) } }],
		});
		load();
		vi.advanceTimersByTime(8_000);
		expect(overlay()).not.toBeNull();
	});

	test('still connecting is reported quietly after 30 seconds, without covering the screen', () => {
		const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 204 })));
		vi.stubGlobal('fetch', fetchMock);
		const splash = document.createElement('div');
		splash.dataset.connecting = 'start';
		document.body.appendChild(splash);
		load();
		vi.advanceTimersByTime(28_000);
		expect(fetchMock).not.toHaveBeenCalled();
		vi.advanceTimersByTime(4_000);
		expect(overlay()).toBeNull();
		expect(fetchMock).toHaveBeenCalledTimes(1);
		const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
		const lines = (JSON.parse(String(init.body)) as { lines: string[] }).lines;
		expect(lines[0]).toContain('still connecting');
		expect(lines).toContain('Showing: the start screen');
		// Once only.
		vi.advanceTimersByTime(120_000);
		expect(fetchMock).toHaveBeenCalledTimes(1);
	});

	test('the app open with the Connecting banner says so', () => {
		const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 204 })));
		vi.stubGlobal('fetch', fetchMock);
		const banner = document.createElement('p');
		banner.dataset.connecting = 'banner';
		document.body.appendChild(banner);
		load();
		vi.advanceTimersByTime(32_000);
		const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
		expect(JSON.parse(String(init.body)).lines).toContain('Showing: the app, with the Connecting banner');
	});

	test('connected before the limit: nothing is sent', () => {
		const fetchMock = vi.fn(() => Promise.resolve(new Response(null, { status: 204 })));
		vi.stubGlobal('fetch', fetchMock);
		const splash = document.createElement('div');
		splash.dataset.connecting = 'start';
		document.body.appendChild(splash);
		load();
		vi.advanceTimersByTime(20_000);
		splash.remove();
		const main = document.createElement('main');
		document.body.appendChild(main);
		Object.defineProperty(document.body, 'innerText', { configurable: true, get: () => 'Welcome' });
		vi.advanceTimersByTime(60_000);
		expect(fetchMock).not.toHaveBeenCalled();
		delete (document.body as unknown as Record<string, unknown>).innerText;
	});
});
