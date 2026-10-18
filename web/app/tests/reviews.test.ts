// Reviews and recommendations: the word limits (as the server counts them) and the star input.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { NOTE_CHARS, NOTE_WORDS, REVIEW_CHARS, REVIEW_WORDS, collapse, countChars, countWords, withinLimit } from '$lib/words';
import Stars from '$lib/components/Stars.svelte';

test('words are counted on collapsed whitespace', () => {
	expect(countWords('')).toBe(0);
	expect(countWords('   \n\t ')).toBe(0);
	expect(countWords('Gripping,  but the\nmiddle drags.')).toBe(5);
	expect(collapse('  a \n\n b  ')).toBe('a b');
	expect(countChars('😀 a')).toBe(3);
});

test('the review limit is 50 words and 400 characters', () => {
	const fifty = Array.from({ length: REVIEW_WORDS }, () => 'word').join(' ');
	expect(withinLimit(fifty, REVIEW_WORDS, REVIEW_CHARS)).toBe(true);
	expect(withinLimit(`${fifty} more`, REVIEW_WORDS, REVIEW_CHARS)).toBe(false);
	// Few words but too many characters.
	expect(withinLimit('x'.repeat(REVIEW_CHARS + 1), REVIEW_WORDS, REVIEW_CHARS)).toBe(false);
	expect(withinLimit('x'.repeat(REVIEW_CHARS), REVIEW_WORDS, REVIEW_CHARS)).toBe(true);
	// Recommendation notes allow more.
	const hundred = Array.from({ length: NOTE_WORDS }, () => 'w').join(' ');
	expect(withinLimit(hundred, NOTE_WORDS, NOTE_CHARS)).toBe(true);
	expect(withinLimit(`${hundred} w`, NOTE_WORDS, NOTE_CHARS)).toBe(false);
});

test('read-only stars say the rating, to the nearest half star', () => {
	render(Stars, { value: 4.26 });
	expect(screen.getByRole('img', { name: '4.5 out of 5 stars' })).toBeTruthy();
});

test('the star input is five labelled radio buttons that set the rating', async () => {
	let value = 0;
	render(Stars, {
		input: true,
		label: 'Your rating',
		get value() {
			return value;
		},
		set value(v: number) {
			value = v;
		},
	});
	const group = screen.getByRole('radiogroup', { name: 'Your rating' });
	const radios = screen.getAllByRole('radio');
	expect(group).toBeTruthy();
	expect(radios).toHaveLength(5);
	expect(radios.map((r) => r.getAttribute('aria-label'))).toEqual(['1 star', '2 stars', '3 stars', '4 stars', '5 stars']);
	await fireEvent.click(screen.getByRole('radio', { name: '4 stars' }));
	expect(value).toBe(4);
	expect((screen.getByRole('radio', { name: '4 stars' }) as HTMLInputElement).checked).toBe(true);
	// Radios in one group: the keyboard moves between them natively.
	expect(new Set(radios.map((r) => (r as HTMLInputElement).name)).size).toBe(1);
});
