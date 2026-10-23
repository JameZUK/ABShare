// The daily limit of different books: the player says why a book won't play, rather than
// retrying or showing a generic failure.
import { expect, test } from './helpers';

// Playwright can't intercept requests the service worker makes on the page's behalf.
test.use({ serviceWorkers: 'block' });

test('a book past the daily limit says so in the player', async ({ page }) => {
	await page.route('**/v1/media/**', async (route) => {
		const url = route.request().url();
		if (url.includes('/covers/')) return route.continue();
		if (url.endsWith('/state')) return route.fulfill({ json: { ready: true, failed: false, limited: true } });
		return route.fulfill({ status: 429, headers: { 'retry-after': '3600' }, json: { error: 'daily_limit', message: 'limit' } });
	});
	await page.goto('#/library');
	await page.getByRole('link', { name: /Small Engines of the Universe/ }).first().click();
	await page.getByRole('main').getByRole('button', { name: /^(Play|Resume)/ }).click();
	const mini = page.getByRole('region', { name: 'Now playing' });
	await expect(mini).toContainText('the most different books for one day');
	expect(await page.evaluate(() => document.querySelector('audio')?.paused)).toBe(true);
});
