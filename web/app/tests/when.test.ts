// Dates for people: today, yesterday, weekdays; grouping by day; "ago".
import { ago, byDay, dayLabel, timeOfDay } from '$lib/when';

const at = (y: number, m: number, d: number, h = 12, min = 0) => new Date(y, m - 1, d, h, min).getTime();
const now = at(2026, 10, 18, 9, 30);

test('days are named relative to now', () => {
	expect(dayLabel(at(2026, 10, 18, 0, 5), now)).toBe('Today');
	expect(dayLabel(at(2026, 10, 17, 23, 50), now)).toBe('Yesterday');
	expect(dayLabel(at(2026, 10, 15), now)).toBe('Thursday 15 October');
	expect(dayLabel(at(2025, 12, 31), now)).toBe('Wednesday 31 December 2025');
});

test('times are 24-hour', () => {
	expect(timeOfDay(at(2026, 10, 17, 22, 41))).toBe('22:41');
	expect(timeOfDay(at(2026, 10, 17, 7, 5))).toBe('07:05');
});

test('ago rounds to something readable', () => {
	expect(ago(now - 30_000, now)).toBe('just now');
	expect(ago(now - 5 * 60_000, now)).toBe('5 min ago');
	expect(ago(now - 3 * 3600_000, now)).toBe('3 h ago');
	expect(ago(at(2026, 10, 17, 8), now)).toBe('Yesterday');
	expect(ago(now + 60_000, now)).toBe('just now');
});

test('items group by day, newest first', () => {
	const items = [at(2026, 10, 18, 1), at(2026, 10, 17, 23), at(2026, 10, 17, 22), at(2026, 10, 15)];
	const g = byDay(items, (t) => t, now);
	expect(g.map((d) => [d.day, d.items.length])).toEqual([
		['Today', 1],
		['Yesterday', 2],
		['Thursday 15 October', 1],
	]);
	expect(byDay([], (t: number) => t, now)).toEqual([]);
});

test('items out of order still make one group per day (a session recorded late, across midnight)', () => {
	const items = [at(2026, 10, 18, 0, 7), at(2026, 10, 18, 0, 1), at(2026, 10, 17, 23, 55), at(2026, 10, 18, 0, 6)];
	const g = byDay(items, (t) => t, now);
	expect(g.map((d) => d.day)).toEqual(['Today', 'Yesterday']);
	expect(g[0]!.items).toEqual([at(2026, 10, 18, 0, 7), at(2026, 10, 18, 0, 6), at(2026, 10, 18, 0, 1)]);
});
