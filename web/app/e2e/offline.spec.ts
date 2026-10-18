// Offline: download a book, lose the connection, and it still opens and plays.
import { apiCall, audioTime, books, expect, expectPlaying, test } from './helpers';

test('a downloaded book plays with no connection at all, and the listening counts', async ({ page, context }) => {
	test.setTimeout(120_000);
	await page.goto('#/library');
	await page.getByRole('link', { name: /The Fen Hauntings/ }).first().click();
	await page.getByRole('button', { name: 'Download', exact: true }).click();
	const sheet = page.getByRole('dialog', { name: 'Download' });
	await expect(sheet.getByText(/Size: \d/)).toBeVisible();
	await sheet.getByRole('button', { name: 'Download', exact: true }).click();
	await expect(page.getByText('Downloaded', { exact: true })).toBeVisible({ timeout: 60_000 });

	await page.goto('#/downloads');
	await expect(page.locator('.item')).toContainText('The Fen Hauntings');
	await expect(page.locator('.item')).toContainText('Downloaded');
	// The service worker must be in charge before the connection goes.
	await page.evaluate(() => navigator.serviceWorker.ready.then(() => undefined));

	await context.setOffline(true);
	await page.reload();
	await expect(page.getByText('Offline. Downloaded books still play.')).toBeVisible({ timeout: 20_000 });
	await page.goto('#/downloads');
	await page.locator('.item a.title', { hasText: 'The Fen Hauntings' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'The Fen Hauntings' })).toBeVisible();
	await page.getByRole('main').getByRole('button', { name: /^(Play|Resume)/ }).click();
	await expectPlaying(page);

	// Listening offline reaches the history once the connection is back (SPEC D33): the app
	// reports every 15 s, keeping the reports it couldn't send, and replays them on return.
	const before = await audioTime(page);
	await page.waitForTimeout(36_000);
	await page.getByRole('button', { name: 'Pause' }).first().click();
	expect(await audioTime(page)).toBeGreaterThan(before + 30);
	await context.setOffline(false);
	await page.reload();
	await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible();
	const fen = (await books()).get('The Fen Hauntings')!;
	await expect
		.poll(async () => (await apiCall<{ sessions: { seconds: number }[] }>('GET', `v1/me/history/books/${fen.id}`)).sessions.reduce((n, s) => n + s.seconds, 0), { timeout: 30_000 })
		.toBeGreaterThanOrEqual(25);
});
