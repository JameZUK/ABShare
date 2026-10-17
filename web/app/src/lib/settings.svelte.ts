// The listener's playback and download preferences, kept in this browser.

export type QualityMode = 'auto' | 'original' | 'saver';
export type SaverKbps = 32 | 40 | 48;

export interface Prefs {
	skipBack: number;
	skipForward: number;
	speed: number;
	autoRewind: boolean;
	quality: QualityMode;
	saverKbps: SaverKbps;
	downloadQuality: 'original' | 'saver';
}

export const DEFAULTS: Prefs = { skipBack: 15, skipForward: 30, speed: 1, autoRewind: true, quality: 'auto', saverKbps: 48, downloadQuality: 'original' };
export const SKIPS = [5, 10, 15, 30, 45, 60];
export const SPEEDS = [0.75, 0.9, 1, 1.1, 1.2, 1.3, 1.5, 1.75, 2, 2.5, 3];
export const KBPS: SaverKbps[] = [48, 40, 32];

const KEY = 'abshare.prefs';
// Per person, so the next person on a shared device doesn't see which books were played
// (review W5). Cleared with that person's downloads.
const speedsKey = (user: string | undefined) => `abshare.bookSpeeds.${user ?? '-'}`;

/** Saved preferences, checked: anything odd falls back to the default. */
export function sanitise(v: unknown): Prefs {
	const o = (v && typeof v === 'object' ? v : {}) as Record<string, unknown>;
	const pick = <T>(x: unknown, ok: readonly T[], d: T): T => (ok.includes(x as T) ? (x as T) : d);
	return {
		skipBack: pick(o.skipBack, SKIPS, DEFAULTS.skipBack),
		skipForward: pick(o.skipForward, SKIPS, DEFAULTS.skipForward),
		speed: pick(o.speed, SPEEDS, DEFAULTS.speed),
		autoRewind: typeof o.autoRewind === 'boolean' ? o.autoRewind : DEFAULTS.autoRewind,
		quality: pick(o.quality, ['auto', 'original', 'saver'] as const, DEFAULTS.quality),
		saverKbps: pick(o.saverKbps, KBPS, DEFAULTS.saverKbps),
		downloadQuality: pick(o.downloadQuality, ['original', 'saver'] as const, DEFAULTS.downloadQuality),
	};
}

function load<T>(key: string, fallback: T): T {
	try {
		const v = localStorage.getItem(key);
		return v ? (JSON.parse(v) as T) : fallback;
	} catch {
		return fallback;
	}
}

function save(key: string, v: unknown) {
	try {
		localStorage.setItem(key, JSON.stringify(v));
	} catch {
		/* not essential */
	}
}

class Settings {
	prefs = $state<Prefs>(sanitise(load(KEY, {})));

	set(change: Partial<Prefs>) {
		this.prefs = sanitise({ ...this.prefs, ...change });
		save(KEY, this.prefs);
	}

	/** Whose per-book speeds are in use (set at sign-in). */
	owner: string | undefined = undefined;

	/** The speed for a book: its own if set, else the default. */
	speedFor(book: string): number {
		const v = load<Record<string, number>>(speedsKey(this.owner), {})[book];
		return SPEEDS.includes(v as number) ? (v as number) : this.prefs.speed;
	}

	setSpeedFor(book: string, speed: number) {
		const m = load<Record<string, number>>(speedsKey(this.owner), {});
		delete m[book];
		m[book] = speed;
		const keys = Object.keys(m);
		for (const k of keys.slice(0, Math.max(0, keys.length - 200))) delete m[k];
		save(speedsKey(this.owner), m);
	}

	forgetSpeeds(user: string) {
		try {
			localStorage.removeItem(speedsKey(user));
			localStorage.removeItem('abshare.bookSpeeds');
		} catch {
			/* not essential */
		}
	}
}

export const settings = new Settings();
