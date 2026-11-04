// The playback log (SPEC D38): the player's recent events, kept in this browser so that when
// playback misbehaves on someone's phone (it stops in the background, or doesn't pause for a
// call), they can send what happened to the admins: Settings → About → Send a playback report.
// It holds times, positions in the book and what the player saw; never which book. It goes
// nowhere unless the listener sends it, and it is cleared at sign-out.

const KEY = 'abshare.playbackLog';
/** Lines kept: a report is at most 40 lines (the start-up report route), two of them headings. */
export const KEEP = 38;

let lines: string[] | null = null;

function load(): string[] {
	if (lines) return lines;
	try {
		const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '[]');
		lines = Array.isArray(v) ? v.filter((l): l is string => typeof l === 'string').slice(-KEEP) : [];
	} catch {
		lines = [];
	}
	return lines;
}

const two = (n: number) => String(n).padStart(2, '0');

/** A position in the book: 1:02:03 or 2:03. */
export function place(ms: number): string {
	const s = Math.max(0, Math.floor(ms / 1000));
	const h = Math.floor(s / 3600);
	const m = Math.floor((s % 3600) / 60);
	return h ? `${h}:${two(m)}:${two(s % 60)}` : `${m}:${two(s % 60)}`;
}

/** Adds one event, with the date and time. */
export function logEvent(text: string, now = new Date()) {
	const l = load();
	l.push(`${two(now.getMonth() + 1)}-${two(now.getDate())} ${two(now.getHours())}:${two(now.getMinutes())}:${two(now.getSeconds())} ${text}`.slice(0, 200));
	if (l.length > KEEP) l.splice(0, l.length - KEEP);
	try {
		localStorage.setItem(KEY, JSON.stringify(l));
	} catch {
		/* not essential */
	}
}

export function playbackLog(): string[] {
	return [...load()];
}

export function forgetLog() {
	lines = [];
	try {
		localStorage.removeItem(KEY);
	} catch {
		/* not essential */
	}
}

/** Sends the log to the admins through the start-up report route (POST /v1/diag). */
export async function sendReport(): Promise<boolean> {
	const body = { lines: ['Playback report (sent from Settings → About)', `Browser: ${navigator.userAgent}`, ...load()] };
	try {
		const r = await fetch('v1/diag', { method: 'POST', credentials: 'same-origin', headers: { 'content-type': 'application/json', 'x-abshare-client': '1' }, body: JSON.stringify(body) });
		return r.ok;
	} catch {
		return false;
	}
}
