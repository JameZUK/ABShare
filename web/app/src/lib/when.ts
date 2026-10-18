// Dates for people: "Today", "Yesterday", "Saturday 18 October", and times like "22:41".

const DAY = 24 * 3600 * 1000;

function startOfDay(ms: number): number {
	const d = new Date(ms);
	d.setHours(0, 0, 0, 0);
	return d.getTime();
}

/** The day a moment falls on, in words, relative to `now`. */
export function dayLabel(ms: number, now = Date.now()): string {
	const days = Math.round((startOfDay(now) - startOfDay(ms)) / DAY);
	if (days === 0) return 'Today';
	if (days === 1) return 'Yesterday';
	const d = new Date(ms);
	const part = (o: Intl.DateTimeFormatOptions) => d.toLocaleDateString('en-GB', o);
	const words = [part({ weekday: 'long' }), part({ day: 'numeric' }), part({ month: 'long' })];
	if (d.getFullYear() !== new Date(now).getFullYear()) words.push(String(d.getFullYear()));
	return words.join(' ');
}

/** The time of day, 24-hour. */
export const timeOfDay = (ms: number) => new Date(ms).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });

/** How long ago, roughly: "just now", "5 min ago", "3 h ago", or the day. */
export function ago(ms: number, now = Date.now()): string {
	const s = Math.max(0, now - ms) / 1000;
	if (s < 90) return 'just now';
	if (s < 3600) return `${Math.round(s / 60)} min ago`;
	if (s < 12 * 3600) return `${Math.round(s / 3600)} h ago`;
	return dayLabel(ms, now);
}

/** Groups items by the day of `at(item)`, keeping their order. */
export function byDay<T>(items: T[], at: (t: T) => number, now = Date.now()): { day: string; items: T[] }[] {
	const out: { day: string; items: T[] }[] = [];
	for (const it of items) {
		const day = dayLabel(at(it), now);
		const last = out[out.length - 1];
		if (last?.day === day) last.items.push(it);
		else out.push({ day, items: [it] });
	}
	return out;
}
