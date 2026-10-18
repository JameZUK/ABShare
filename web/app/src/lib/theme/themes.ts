// Themes: each turns the user's choices (theme, light/dark mode) into the design tokens the
// app's CSS uses (--bg, --fg, --accent, …). Applied at start-up, and again whenever the choice
// or the system's light/dark setting changes.
//
// Paperback (the default) is warm and bookish: paper and ink in light, lamplight in dark, a
// serif for titles and one vermilion accent. High contrast is for maximum legibility.

export type ThemeId = 'paperback' | 'contrast';
export type Mode = 'auto' | 'light' | 'dark';

export interface ThemePrefs {
	theme: ThemeId;
	mode: Mode;
}

export interface ThemeInfo {
	id: ThemeId;
	name: string;
	description: string;
}

export const THEMES: ThemeInfo[] = [
	{ id: 'paperback', name: 'Paperback', description: 'Paper and ink, with serif titles. Lamplight in the dark.' },
	{ id: 'contrast', name: 'High contrast', description: 'Maximum legibility: black and white, strong outlines.' },
];

export const DEFAULT_PREFS: ThemePrefs = { theme: 'paperback', mode: 'auto' };

export type Tokens = Record<string, string>;

const PAPERBACK = {
	light: {
		bg: '#f3eee4',
		'bg-raised': '#fbf8f2',
		'bg-sunken': '#e8e1d3',
		fg: '#1e2430',
		'fg-muted': '#5d5548',
		line: '#d9d0c0',
		accent: '#b4442c',
		'accent-fg': '#ffffff',
		'accent-soft': '#f0dcd2',
		'accent-soft-fg': '#1e2430',
		danger: '#a12a1e',
		shadow: '0 1px 2px rgb(60 45 30 / 8%), 0 6px 18px rgb(60 45 30 / 8%)',
		'shadow-book': '0 1px 0 rgb(0 0 0 / 8%), 0 12px 22px -8px rgb(40 30 20 / 45%)',
		scrim: 'rgb(30 24 16 / 45%)',
	},
	dark: {
		bg: '#17140f',
		'bg-raised': '#211d17',
		'bg-sunken': '#2b261e',
		fg: '#ece4d4',
		'fg-muted': '#b8ac97',
		line: '#3a3329',
		accent: '#e87a5d',
		'accent-fg': '#1c0d08',
		'accent-soft': '#3d261d',
		'accent-soft-fg': '#ece4d4',
		danger: '#f08a7a',
		shadow: '0 1px 2px rgb(0 0 0 / 40%), 0 8px 24px rgb(0 0 0 / 35%)',
		'shadow-book': '0 1px 0 rgb(255 240 220 / 6%), 0 14px 24px -8px rgb(0 0 0 / 75%)',
		scrim: 'rgb(0 0 0 / 60%)',
	},
};

const CONTRAST = {
	light: {
		bg: '#ffffff',
		'bg-raised': '#ffffff',
		'bg-sunken': '#e6e6e6',
		fg: '#000000',
		'fg-muted': '#262626',
		line: '#000000',
		accent: '#0037a6',
		'accent-fg': '#ffffff',
		'accent-soft': '#d6e2ff',
		'accent-soft-fg': '#000000',
		danger: '#9b0000',
		shadow: 'none',
		'shadow-book': '0 0 0 2px #000000',
		scrim: 'rgb(0 0 0 / 70%)',
	},
	dark: {
		bg: '#000000',
		'bg-raised': '#000000',
		'bg-sunken': '#262626',
		fg: '#ffffff',
		'fg-muted': '#e6e6e6',
		line: '#ffffff',
		accent: '#ffd60a',
		'accent-fg': '#000000',
		'accent-soft': '#3d3300',
		'accent-soft-fg': '#ffffff',
		danger: '#ff8a8a',
		shadow: 'none',
		'shadow-book': '0 0 0 2px #ffffff',
		scrim: 'rgb(0 0 0 / 80%)',
	},
};

const SANS = "'Instrument Sans', system-ui, -apple-system, 'Segoe UI', sans-serif";
const SERIF = "Fraunces, Georgia, 'Times New Roman', serif";

const SHAPES: Record<ThemeId, Tokens> = {
	paperback: { radius: '14px', 'radius-sm': '10px', font: SANS, 'font-display': SERIF },
	contrast: { radius: '6px', 'radius-sm': '4px', font: SANS, 'font-display': SANS },
};

/** Light or dark, given the mode and the system setting. */
export const resolveMode = (mode: Mode, systemDark: boolean): 'light' | 'dark' =>
	mode === 'auto' ? (systemDark ? 'dark' : 'light') : mode;

/** All design tokens for these choices. */
export function tokens(p: ThemePrefs, dark: boolean): Tokens {
	const colours = (p.theme === 'contrast' ? CONTRAST : PAPERBACK)[dark ? 'dark' : 'light'];
	return { ...colours, ...SHAPES[p.theme] };
}

/** Saved choices, checked (anything odd falls back to the defaults). Themes from before
 *  Paperback (Material, Classic) become Paperback. */
export function sanitise(v: unknown): ThemePrefs {
	const o = (v && typeof v === 'object' ? v : {}) as Record<string, unknown>;
	return {
		theme: THEMES.some((t) => t.id === o.theme) ? (o.theme as ThemeId) : DEFAULT_PREFS.theme,
		mode: o.mode === 'light' || o.mode === 'dark' || o.mode === 'auto' ? o.mode : DEFAULT_PREFS.mode,
	};
}

const KEY = 'abshare.appearance';

type Store = Pick<Storage, 'getItem' | 'setItem'>;

function safeLocal(): Store | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

export function loadPrefs(store: Store | undefined = safeLocal()): ThemePrefs {
	try {
		const saved = store?.getItem(KEY);
		if (saved) return sanitise(JSON.parse(saved));
	} catch {
		/* unreadable: defaults */
	}
	return { ...DEFAULT_PREFS };
}

export function savePrefs(p: ThemePrefs, store: Store | undefined = safeLocal()) {
	try {
		store?.setItem(KEY, JSON.stringify(sanitise(p)));
	} catch {
		/* not essential */
	}
}

/** Put the tokens on the page: CSS variables, data attributes, and the browser bar colour. */
export function applyTheme(p: ThemePrefs, systemDark: boolean, doc: Document = document) {
	const mode = resolveMode(p.mode, systemDark);
	const root = doc.documentElement;
	const t = tokens(p, mode === 'dark');
	for (const [k, v] of Object.entries(t)) root.style.setProperty(`--${k}`, v);
	root.dataset.theme = p.theme;
	root.dataset.mode = mode;
	root.style.colorScheme = mode;
	let meta = doc.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
	if (!meta) {
		meta = doc.createElement('meta');
		meta.name = 'theme-color';
		doc.head.append(meta);
	}
	meta.content = t.bg!;
}
