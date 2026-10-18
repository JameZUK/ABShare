// Word limits for free text people write: reviews and recommendation notes. Counted as the
// server counts them (core::text): whitespace collapsed, words split on whitespace.

export const REVIEW_WORDS = 50;
export const REVIEW_CHARS = 400;
export const NOTE_WORDS = 100;
export const NOTE_CHARS = 800;

/** The text as the server will keep it: one paragraph, whitespace collapsed. */
export const collapse = (s: string) => s.replace(/\s+/gu, ' ').trim();

export function countWords(s: string): number {
	const t = collapse(s);
	return t ? t.split(' ').length : 0;
}

/** Characters, as code points (an emoji counts once). */
export const countChars = (s: string) => [...collapse(s)].length;

export function withinLimit(s: string, words: number, chars: number): boolean {
	return countWords(s) <= words && countChars(s) <= chars;
}
