// Listening history: how totals and session times read.
import { listened, when } from '$lib/history';

test('time listened reads naturally and never shows nonsense', () => {
	expect(listened(0)).toBe('None');
	expect(listened(-5)).toBe('None');
	expect(listened(Number.NaN)).toBe('None');
	expect(listened(Number.POSITIVE_INFINITY)).toBe('None');
	expect(listened(59)).toBe('Under a minute');
	expect(listened(60)).toBe('1 min');
	expect(listened(45 * 60 + 59)).toBe('45 min');
	expect(listened(3600)).toBe('1 h');
	expect(listened(3 * 3600 + 20 * 60)).toBe('3 h 20 min');
	expect(listened(1204 * 3600 + 59 * 60)).toBe('1,204 h');
});

test('session times show a date, or a dash when missing', () => {
	expect(when(0)).toBe('—');
	expect(when(-1)).toBe('—');
	expect(when(Number.NaN)).toBe('—');
	expect(when(Date.UTC(2026, 9, 17, 12, 0))).toContain('2026');
});
