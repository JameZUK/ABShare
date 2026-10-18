// Browsing: the library, search, the status switch, a book, its series, and genres.
import { books, expect, setProgress, test } from './helpers';

const tiles = (page: import('@playwright/test').Page) => page.locator('main a.tile .title');

test('the library lists the books and shows, and search narrows them', async ({ page }) => {
	await page.goto('#/library');
	await expect(page.getByText('12 titles')).toBeVisible();
	await expect(tiles(page)).toHaveCount(12);

	await page.getByRole('searchbox', { name: 'Search' }).fill('meridian');
	await expect(page.getByText('2 titles')).toBeVisible();
	// Titles sort without a leading "The".
	await expect(tiles(page)).toHaveText(['The Glass Meridian', 'Meridian Falling']);

	// Search ignores case and accents, and covers narrators too.
	await page.getByRole('searchbox', { name: 'Search' }).fill('MÉRIDIAN');
	await expect(page.getByText('2 titles')).toBeVisible();
	await page.getByRole('searchbox', { name: 'Search' }).fill('no such book anywhere');
	await expect(page.getByText('0 titles')).toBeVisible();
});

test('the status switch shows only finished books', async ({ page }) => {
	const all = await books();
	await setProgress(all.get('The Lantern Keeper')!, 'finished');
	await page.goto('#/library');
	await page.getByRole('radio', { name: 'Finished' }).check();
	// The Clockwork Fox was finished before these tests (visual.spec.ts).
	await expect(tiles(page)).toHaveText(['The Clockwork Fox', 'The Lantern Keeper']);
	await page.getByRole('radio', { name: 'All' }).check();
	await expect(page.getByText('12 titles')).toBeVisible();
});

test('a book page shows its details and leads to its series', async ({ page }) => {
	await page.goto('#/library');
	await page.getByRole('link', { name: /The Glass Meridian/ }).first().click();
	await expect(page.getByRole('heading', { level: 1, name: 'The Glass Meridian' })).toBeVisible();
	await expect(page.getByText('By Mara Quill')).toBeVisible();
	await expect(page.locator('.chapters li')).toHaveCount(12);
	await expect(page.getByRole('main').getByRole('button', { name: /^(Play|Resume)/ })).toBeVisible();

	await page.getByRole('link', { name: /The Meridian Cycle/ }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'The Meridian Cycle' })).toBeVisible();
	await expect(page.getByText('2 books')).toBeVisible();
	await expect(tiles(page)).toHaveText(['The Glass Meridian', 'Meridian Falling']);

	// The author link lists their books.
	await page.getByRole('link', { name: /The Glass Meridian/ }).first().click();
	await page.getByRole('link', { name: 'Mara Quill' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Mara Quill' })).toBeVisible();
	await expect(tiles(page)).toHaveText(['The Glass Meridian', 'Meridian Falling']);
});

test('genres lead to their shelves', async ({ page }) => {
	await page.goto('#/genres');
	await expect(page.getByRole('heading', { name: 'Fiction', exact: true })).toBeVisible();
	await page.getByRole('link', { name: /^Fantasy/ }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Fantasy' })).toBeVisible();
	await expect(tiles(page).first()).toBeVisible();
});
