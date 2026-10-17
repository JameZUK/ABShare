// Chapter maths for the player (positions in ms).
import type { Chapter } from '$lib/api/types';

/** The index of the chapter playing at `pos`, or -1 with no chapters. */
export function chapterAt(chapters: Chapter[], pos: number): number {
	if (chapters.length === 0) return -1;
	let lo = 0;
	let hi = chapters.length - 1;
	while (lo < hi) {
		const mid = (lo + hi + 1) >> 1;
		if (chapters[mid]!.start_ms <= pos) lo = mid;
		else hi = mid - 1;
	}
	return lo;
}

/** The span the scrubber covers: the current chapter, or the whole book without chapters. */
export function span(chapters: Chapter[], pos: number, duration: number): { start: number; end: number; index: number } {
	const i = chapterAt(chapters, pos);
	if (i < 0) return { start: 0, end: duration, index: -1 };
	const c = chapters[i]!;
	const end = Math.min(duration || c.end_ms, i + 1 < chapters.length ? chapters[i + 1]!.start_ms : Math.max(c.end_ms, duration));
	return { start: c.start_ms, end: Math.max(end, c.start_ms + 1), index: i };
}

/**
 * Where "previous chapter" goes: the start of this chapter, or the previous one when within
 * the first few seconds (like a CD player).
 */
export function previousTarget(chapters: Chapter[], pos: number, grace = 3000): number {
	const i = chapterAt(chapters, pos);
	if (i < 0) return 0;
	const start = chapters[i]!.start_ms;
	if (pos - start > grace || i === 0) return start;
	return chapters[i - 1]!.start_ms;
}

/** Where "next chapter" goes, or null at the last chapter. */
export function nextTarget(chapters: Chapter[], pos: number): number | null {
	const i = chapterAt(chapters, pos);
	return i >= 0 && i + 1 < chapters.length ? chapters[i + 1]!.start_ms : null;
}
