// "How was it?": asked once per book per person, bounded, and never when already reviewed.
import { MAX_ASKED, askedBefore, markAsked, shouldPrompt } from '$lib/reviewAsked';

const mem = () => {
	const m = new Map<string, string>();
	return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), m };
};
const B32 = 'abcdefghijklmnopqrstuvwxyz234567';
/** A distinct, valid book ID for each number. */
const id = (n: number) => {
	let s = '';
	do {
		s = B32[n % 32] + s;
		n = Math.floor(n / 32);
	} while (n > 0);
	return s.padStart(26, 'a');
};
const A = 'a'.repeat(26);
const B = 'b'.repeat(26);

test('a book is asked about once, per person', () => {
	const s = mem();
	expect(askedBefore('ada', A, s)).toBe(false);
	markAsked('ada', A, s);
	expect(askedBefore('ada', A, s)).toBe(true);
	expect(askedBefore('ada', B, s)).toBe(false);
	expect(askedBefore('grace', A, s)).toBe(false);
	markAsked('ada', A, s);
	expect(JSON.parse(s.m.get('abshare.reviewAsked.ada')!)).toEqual([A]);
});

test('the list is bounded, oldest forgotten first', () => {
	const s = mem();
	const ids = Array.from({ length: MAX_ASKED + 10 }, (_, i) => id(i));
	expect(new Set(ids).size).toBe(ids.length);
	for (const b of ids) markAsked('ada', b, s);
	const kept = JSON.parse(s.m.get('abshare.reviewAsked.ada')!) as string[];
	expect(kept).toHaveLength(MAX_ASKED);
	expect(askedBefore('ada', ids[0]!, s)).toBe(false);
	expect(askedBefore('ada', ids[ids.length - 1]!, s)).toBe(true);
});

test('broken or blocked storage is harmless', () => {
	const s = mem();
	s.m.set('abshare.reviewAsked.ada', '{not json');
	expect(askedBefore('ada', A, s)).toBe(false);
	s.m.set('abshare.reviewAsked.ada', JSON.stringify(['javascript:alert(1)', 42, A]));
	expect(askedBefore('ada', A, s)).toBe(true);
	expect(askedBefore('ada', 'javascript:alert(1)', s)).toBe(false);
	const blocked = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
	expect(askedBefore('ada', A, blocked)).toBe(false);
	expect(() => markAsked('ada', A, blocked)).not.toThrow();
	expect(askedBefore('ada', A, undefined)).toBe(false);
});

test('the prompt shows only online, not asked before, not already reviewed', () => {
	expect(shouldPrompt({ online: true, asked: false, reviewed: false })).toBe(true);
	expect(shouldPrompt({ online: false, asked: false, reviewed: false })).toBe(false);
	expect(shouldPrompt({ online: true, asked: true, reviewed: false })).toBe(false);
	expect(shouldPrompt({ online: true, asked: false, reviewed: true })).toBe(false);
});
