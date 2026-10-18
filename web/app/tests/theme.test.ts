// Themes: palettes that stay legible, and safe saved choices.
import { contrast, parseHex } from '$lib/theme/color';
import { DEFAULT_PREFS, THEMES, applyTheme, loadPrefs, resolveMode, sanitise, savePrefs, tokens, type ThemePrefs } from '$lib/theme/themes';

test('hex colours parse; anything else is refused', () => {
	expect(parseHex('#abc')).toEqual(parseHex('#aabbcc'));
	expect(parseHex('#ffffff')).toEqual([1, 1, 1]);
	for (const bad of ['', 'red', '#12345', '#ggg', 'javascript:x']) expect(parseHex(bad)).toBeNull();
});

test('WCAG contrast is computed as specified', () => {
	expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 5);
	expect(contrast('#ffffff', '#ffffff')).toBeCloseTo(1, 5);
	expect(contrast('#767676', '#ffffff')).toBeCloseTo(4.54, 1);
});

// Every theme and mode must keep text legible (WCAG AA; body text AAA).
for (const theme of THEMES) {
	for (const dark of [false, true]) {
		test(`${theme.name} ${dark ? 'dark' : 'light'} keeps text legible`, () => {
			const t = tokens({ theme: theme.id, mode: 'auto' }, dark);
			const where = `${theme.id} ${dark ? 'dark' : 'light'}`;
			for (const surface of ['bg', 'bg-raised']) {
				expect(contrast(t.fg!, t[surface]!), `fg on ${surface}, ${where}`).toBeGreaterThanOrEqual(7);
				expect(contrast(t['fg-muted']!, t[surface]!), `muted on ${surface}, ${where}`).toBeGreaterThanOrEqual(4.5);
				expect(contrast(t.danger!, t[surface]!), `danger on ${surface}, ${where}`).toBeGreaterThanOrEqual(4.5);
				// The accent is used for links and small labels, so it must pass as text.
				expect(contrast(t.accent!, t[surface]!), `accent on ${surface}, ${where}`).toBeGreaterThanOrEqual(4.5);
			}
			expect(contrast(t.fg!, t['bg-sunken']!), `fg on sunken, ${where}`).toBeGreaterThanOrEqual(7);
			expect(contrast(t['fg-muted']!, t['bg-sunken']!), `muted on sunken, ${where}`).toBeGreaterThanOrEqual(4.5);
			expect(contrast(t['accent-fg']!, t.accent!), `on accent, ${where}`).toBeGreaterThanOrEqual(4.5);
			expect(contrast(t['accent-soft-fg']!, t['accent-soft']!), `on accent-soft, ${where}`).toBeGreaterThanOrEqual(4.5);
			// Ink buttons: the page colour on the text colour.
			expect(contrast(t.bg!, t.fg!), `ink button, ${where}`).toBeGreaterThanOrEqual(7);
			for (const v of Object.values(t)) expect(v).not.toMatch(/[;{}<>]|url\(|expression/i);
		});
	}
}

test('modes resolve against the system setting', () => {
	expect(resolveMode('auto', true)).toBe('dark');
	expect(resolveMode('auto', false)).toBe('light');
	expect(resolveMode('light', true)).toBe('light');
	expect(resolveMode('dark', false)).toBe('dark');
});

test('saved choices are checked; anything odd falls back to defaults', () => {
	expect(sanitise(null)).toEqual(DEFAULT_PREFS);
	expect(sanitise({ theme: 'contrast', mode: 'dark' })).toEqual({ theme: 'contrast', mode: 'dark' });
	expect(sanitise({ theme: '__proto__', mode: 'x' })).toEqual(DEFAULT_PREFS);
	// Choices saved before Paperback keep their light/dark setting and become Paperback.
	expect(sanitise({ theme: 'material', mode: 'dark', accent: '#1f5fbf' })).toEqual({ theme: 'paperback', mode: 'dark' });
	expect(sanitise({ theme: 'classic', mode: 'light' })).toEqual({ theme: 'paperback', mode: 'light' });
});

test('choices are saved and restored', () => {
	const m = new Map<string, string>();
	const store = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
	expect(loadPrefs(store)).toEqual(DEFAULT_PREFS);
	const p: ThemePrefs = { theme: 'contrast', mode: 'light' };
	savePrefs(p, store);
	expect(loadPrefs(store)).toEqual(p);
	m.set('abshare.appearance', '{broken');
	expect(loadPrefs(store)).toEqual(DEFAULT_PREFS);
	const blocked = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
	expect(loadPrefs(blocked)).toEqual(DEFAULT_PREFS);
	savePrefs(p, blocked);
});

test('applying a theme sets tokens, mode and the browser bar colour', () => {
	applyTheme({ theme: 'paperback', mode: 'auto' }, true);
	const root = document.documentElement;
	expect(root.dataset.theme).toBe('paperback');
	expect(root.dataset.mode).toBe('dark');
	expect(root.style.getPropertyValue('--bg')).toBe('#17140f');
	expect(root.style.getPropertyValue('--font-display')).toContain('Fraunces');
	const meta = document.querySelector('meta[name="theme-color"]') as HTMLMetaElement;
	expect(meta.content).toBe(root.style.getPropertyValue('--bg'));
	applyTheme({ theme: 'paperback', mode: 'light' }, true);
	expect(root.dataset.mode).toBe('light');
	expect(root.style.getPropertyValue('--bg')).toBe('#f3eee4');
	expect(document.querySelectorAll('meta[name="theme-color"]').length).toBe(1);
});
