// Pure player rules: chapters, auto-rewind, the sleep fade, quality choice and resume.
import { chapterAt, nextTarget, previousTarget, span } from '$lib/player/chapters';
import { autoRewindMs, chooseQuality, clampStart, fadeVolume, streamUrl } from '$lib/player/rules';
import type { Chapter } from '$lib/api/types';

vi.mock('$lib/api/session.svelte', () => ({ session: { status: 'signed-out', user: null }, api: vi.fn() }));
vi.mock('$lib/offline/downloads.svelte', () => ({ downloads: { playable: () => false, cover: async () => undefined } }));
vi.mock('$lib/offline/sync.svelte', () => ({ sync: { record: vi.fn(), local: async () => undefined } }));
const { startPosition } = await import('$lib/player/player.svelte');

const ch = (s: number, e: number, title = ''): Chapter => ({ start_ms: s, end_ms: e, title });
const CH = [ch(0, 60_000, 'One'), ch(60_000, 120_000, 'Two'), ch(120_000, 300_000, 'Three')];

test('the chapter playing is found at and around boundaries', () => {
	expect(chapterAt([], 5)).toBe(-1);
	expect(chapterAt(CH, 0)).toBe(0);
	expect(chapterAt(CH, 59_999)).toBe(0);
	expect(chapterAt(CH, 60_000)).toBe(1);
	expect(chapterAt(CH, 10_000_000)).toBe(2);
	expect(chapterAt([ch(5_000, 10_000)], 0)).toBe(0);
});

test('the scrubber spans the current chapter, or the book without chapters', () => {
	expect(span(CH, 90_000, 300_000)).toEqual({ start: 60_000, end: 120_000, index: 1 });
	expect(span(CH, 200_000, 300_000)).toEqual({ start: 120_000, end: 300_000, index: 2 });
	expect(span([], 5_000, 300_000)).toEqual({ start: 0, end: 300_000, index: -1 });
	const s = span([ch(0, 0)], 0, 0);
	expect(s.end).toBeGreaterThan(s.start);
});

test('previous goes to the chapter start, or the one before near the start', () => {
	expect(previousTarget(CH, 90_000)).toBe(60_000);
	expect(previousTarget(CH, 61_000)).toBe(0);
	expect(previousTarget(CH, 1_000)).toBe(0);
	expect(previousTarget([], 50_000)).toBe(0);
	expect(nextTarget(CH, 10)).toBe(60_000);
	expect(nextTarget(CH, 200_000)).toBeNull();
	expect(nextTarget([], 0)).toBeNull();
});

test('auto-rewind grows with the length of the pause', () => {
	expect(autoRewindMs(30_000)).toBe(0);
	expect(autoRewindMs(5 * 60_000)).toBe(2_000);
	expect(autoRewindMs(30 * 60_000)).toBe(5_000);
	expect(autoRewindMs(5 * 3600_000)).toBe(10_000);
});

test('the sleep timer fades over its last ten seconds', () => {
	expect(fadeVolume(60_000)).toBe(1);
	expect(fadeVolume(10_000)).toBe(1);
	expect(fadeVolume(5_000)).toBeCloseTo(0.5);
	expect(fadeVolume(0)).toBe(0);
	expect(fadeVolume(-50)).toBe(0);
});

test('automatic quality uses data saver only when the browser reports mobile data', () => {
	expect(chooseQuality('original', 48, { type: 'cellular' })).toBe('original');
	expect(chooseQuality('saver', 40, undefined)).toBe(40);
	expect(chooseQuality('auto', 48, undefined)).toBe('original');
	expect(chooseQuality('auto', 48, { type: 'wifi' })).toBe('original');
	expect(chooseQuality('auto', 32, { type: 'cellular' })).toBe(32);
	expect(chooseQuality('auto', 48, { saveData: true })).toBe(48);
	expect(chooseQuality('auto', 48, { effectiveType: '2g' })).toBe(48);
	expect(streamUrl('a'.repeat(26), 'original')).toBe(`v1/media/${'a'.repeat(26)}`);
	expect(streamUrl('a'.repeat(26), 32)).toBe(`v1/media/${'a'.repeat(26)}?q=32`);
});

test('resume picks the newer of the server and device positions', () => {
	const hour = 3_600_000;
	expect(startPosition(null, undefined, hour)).toBe(0);
	expect(startPosition({ position_ms: 600_000, finished: false, updated_at: 2_000 }, { position_ms: 300_000, at: 1_000 }, hour)).toBe(600_000);
	expect(startPosition({ position_ms: 600_000, finished: false, updated_at: 1_000 }, { position_ms: 300_000, at: 2_000 }, hour)).toBe(300_000);
	expect(startPosition({ position_ms: 600_000, finished: true, updated_at: 2_000 }, undefined, hour)).toBe(0);
	expect(startPosition(null, { position_ms: hour - 10_000, at: 1 }, hour), 'the very end starts again').toBe(0);
	expect(startPosition(null, { position_ms: -5, at: 1 }, hour)).toBe(0);
	expect(startPosition(null, { position_ms: 13_000, at: 1 }, 20_000), 'short items resume too').toBe(13_000);
	expect(startPosition(null, { position_ms: 19_700, at: 1 }, 20_000)).toBe(0);
});

test('a requested start position stays within the book, short of its end (review W13)', () => {
	const D = 3_600_000;
	expect(clampStart(120_000, D)).toBe(120_000);
	expect(clampStart(D, D)).toBe(D - 500);
	expect(clampStart(D + 1e9, D)).toBe(D - 500);
	expect(clampStart(-1, D)).toBe(0);
	expect(clampStart(NaN, D)).toBe(0);
	expect(clampStart(Infinity, D)).toBe(0);
	expect(clampStart(10_000, 0)).toBe(0);
});
