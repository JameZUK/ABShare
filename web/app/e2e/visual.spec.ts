// The main screens, light and dark, compared with reference screenshots, so a change that
// breaks the look fails CI. Covers are masked: the demo covers are drawn with the machine's
// own fonts, which differ between machines.
//
// A screen with no reference yet is skipped (not failed). To make or refresh the references,
// run with --update-snapshots; CI's own are made by CI (docs/BUILDING.md).
import fs from 'node:fs';
import type { Locator, Page } from '@playwright/test';
import { books, expect, setProgress, test } from './helpers';


let lastBookId = '';

test.beforeAll(async () => {
	const all = await books();
	const now = Date.now();
	await setProgress(all.get('Salt and Lanterns')!, 0.72, now);
	await setProgress(all.get('The Glass Meridian')!, 0.4, now - 60_000);
	await setProgress(all.get('A Quiet Death in Wexford Lane')!, 0.08, now - 120_000);
	await setProgress(all.get('The Clockwork Fox')!, 'finished', now - 180_000);
	lastBookId = all.get('Salt and Lanterns')!.id;
});

/** `within`: only mask inside this part of the page (a full-screen layer covers the rest,
 *  and masks are painted even over hidden elements). */
async function shoot(page: Page, name: string, within?: Locator) {
	const info = test.info();
	const updating = info.config.updateSnapshots === 'all' || info.config.updateSnapshots === 'changed';
	test.skip(!updating && !fs.existsSync(info.snapshotPath(`${name}.png`)), `no reference screenshot for ${name} on this machine yet`);
	await page.evaluate(() => document.fonts.ready.then(() => undefined));
	await page.waitForTimeout(400);
	await expect(page).toHaveScreenshot(`${name}.png`, {
		mask: within ? [within.locator('.frame')] : [page.locator('.frame'), page.locator('.greet .eyebrow')],
		maskColor: '#8a8a8a',
	});
}

for (const scheme of ['light', 'dark'] as const) {
	test.describe(scheme, () => {
		test.use({ colorScheme: scheme });

		test(`home (${scheme})`, async ({ page }) => {
			await page.goto('#/');
			await expect(page.getByText('Currently reading')).toBeVisible();
			await expect(page.getByRole('heading', { name: 'Continue listening' })).toBeVisible();
			await shoot(page, `home-${scheme}`);
		});

		test(`library (${scheme})`, async ({ page }) => {
			await page.goto('#/library');
			await expect(page.getByText('12 titles')).toBeVisible();
			await shoot(page, `library-${scheme}`);
		});

		test(`book (${scheme})`, async ({ page }) => {
			await page.goto('#/library');
			await page.getByRole('link', { name: /The Glass Meridian/ }).first().click();
			await expect(page.getByRole('heading', { level: 1, name: 'The Glass Meridian' })).toBeVisible();
			await shoot(page, `book-${scheme}`);
		});

		test(`player (${scheme})`, async ({ page }) => {
			// The last book comes back paused at its saved place, so the player is still.
			await page.addInitScript((id) => localStorage.setItem('abshare.lastBook', id), lastBookId);
			await page.goto('#/');
			await page.getByRole('button', { name: /Open the player/ }).click();
			const player = page.getByRole('dialog', { name: 'Player' });
			await expect(player).toBeVisible();
			await shoot(page, `player-${scheme}`, player);
		});

		test(`genres (${scheme})`, async ({ page }) => {
			await page.goto('#/genres');
			await expect(page.getByRole('heading', { name: 'Fiction', exact: true })).toBeVisible();
			await shoot(page, `genres-${scheme}`);
		});

		test(`settings (${scheme})`, async ({ page }) => {
			await page.goto('#/settings');
			await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
			await shoot(page, `settings-${scheme}`);
		});
	});
}

test.describe('desktop', () => {
	test.use({ viewport: { width: 1280, height: 820 }, isMobile: false, hasTouch: false });

	test('home (desktop)', async ({ page }) => {
		await page.goto('#/');
		await expect(page.getByText('Currently reading')).toBeVisible();
		await shoot(page, 'home-desktop');
	});

	test('book (desktop, dark)', async ({ page }) => {
		await page.emulateMedia({ colorScheme: 'dark' });
		await page.goto('#/library');
		await page.getByRole('link', { name: /The Glass Meridian/ }).first().click();
		await expect(page.getByRole('heading', { level: 1, name: 'The Glass Meridian' })).toBeVisible();
		await shoot(page, 'book-desktop-dark');
	});
});
