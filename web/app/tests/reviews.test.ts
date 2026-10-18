// Reviews and recommendations: the word limits (as the server counts them) and the star input.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { NOTE_CHARS, NOTE_WORDS, REVIEW_CHARS, REVIEW_WORDS, collapse, countChars, countWords, withinLimit } from '$lib/words';
import Stars from '$lib/components/Stars.svelte';

const mocks = vi.hoisted(() => ({ api: vi.fn() }));
vi.mock('$lib/api/session.svelte', () => ({
	session: { status: 'signed-in', user: { username: 'ada', display_name: 'Ada', role: 'listener' } },
	api: mocks.api,
	post: vi.fn(),
}));

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

describe('Reviews', () => {
	const A = 'aaaaaaaaaaaaaaaaaaaaaaaaaa';
	const B = 'bbbbbbbbbbbbbbbbbbbbbbbbbb';
	const viewOf = (name: string, comment: string) => ({
		average: 4,
		count: 1,
		mine: null,
		reviews: [{ name, rating: 4, comment, updated_at: 1_700_000_000, mine: false }],
	});

	test('a slow answer for the previous book is not shown on the next one (review W14)', async () => {
		const { default: Reviews } = await import('$lib/components/Reviews.svelte');
		let releaseA: (v: unknown) => void = () => {};
		mocks.api.mockImplementation((path: string) =>
			path.includes(A) ? new Promise((r) => (releaseA = r)) : Promise.resolve(viewOf('Bea', 'About book B.')),
		);
		const r = render(Reviews, { bookId: A });
		await r.rerender({ bookId: B });
		await screen.findByText('About book B.');
		releaseA(viewOf('Al', 'About book A.'));
		await new Promise((res) => setTimeout(res, 20));
		expect(screen.queryByText('About book A.')).toBeNull();
		expect(screen.getByText('About book B.')).toBeTruthy();
	});

	test('two reviews with the same name and time both show (review W15)', async () => {
		const { default: Reviews } = await import('$lib/components/Reviews.svelte');
		const v = viewOf('Sam', 'First.');
		v.count = 2;
		v.reviews.push({ ...v.reviews[0]!, comment: 'Second.' });
		mocks.api.mockResolvedValue(v);
		render(Reviews, { bookId: A });
		await screen.findByText('First.');
		expect(screen.getByText('Second.')).toBeTruthy();
	});
});
