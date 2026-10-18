// Appearance: light, dark and High contrast apply at once and survive a reload.
import { expect, test } from './helpers';

const root = (page: import('@playwright/test').Page) => page.locator('html');
const bg = (page: import('@playwright/test').Page) => page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--bg').trim());

test('dark mode and High contrast apply and are remembered', async ({ page }) => {
	await page.goto('#/settings/appearance');
	await expect(root(page)).toHaveAttribute('data-theme', 'paperback');
	await expect(root(page)).toHaveAttribute('data-mode', 'light');
	expect(await bg(page)).toBe('#f3eee4');

	await page.getByRole('radio', { name: 'Dark', exact: true }).check();
	await expect(root(page)).toHaveAttribute('data-mode', 'dark');
	expect(await bg(page)).toBe('#17140f');
	await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute('content', '#17140f');

	await page.reload();
	await expect(root(page)).toHaveAttribute('data-mode', 'dark');

	await page.getByRole('radio', { name: /High contrast/ }).check();
	await expect(root(page)).toHaveAttribute('data-theme', 'contrast');
	expect(await bg(page)).toBe('#000000');

	// Back to the defaults for the tests that follow.
	await page.getByRole('radio', { name: /Paperback/ }).check();
	await page.getByRole('radio', { name: 'System', exact: true }).check();
	await expect(root(page)).toHaveAttribute('data-mode', 'light');
});

test('the bundled fonts load (no fallback faces)', async ({ page }) => {
	await page.goto('#/');
	await expect(page.getByRole('heading', { name: /Welcome back/ })).toBeVisible();
	const loaded = await page.evaluate(async () => {
		await document.fonts.ready;
		return ['600 16px Fraunces', 'italic 600 16px Fraunces', '400 16px "Instrument Sans"'].map((f) => document.fonts.check(f));
	});
	expect(loaded).toEqual([true, true, true]);
	const fonts = await page.evaluate(() => performance.getEntriesByType('resource').map((r) => r.name).filter((n) => n.endsWith('.woff2')));
	expect(fonts.length).toBeGreaterThan(0);
	for (const f of fonts) expect(new URL(f).origin).toBe(new URL(page.url()).origin);
});
