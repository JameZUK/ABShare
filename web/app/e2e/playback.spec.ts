// Listening: play, the player, seek by chapter, speed, data saver, and picking up where you
// left off after a reload.
import { audioTime, expect, expectPlaying, test } from './helpers';

test('a book plays, seeks by chapter, and resumes after a reload', async ({ page }) => {
	await page.goto('#/library');
	await page.getByRole('link', { name: /The Glass Meridian/ }).first().click();
	await page.getByRole('main').getByRole('button', { name: /^(Play|Resume)/ }).click();

	const mini = page.getByRole('region', { name: 'Now playing' });
	await expect(mini).toContainText('The Glass Meridian');
	await expectPlaying(page);

	// The full player: the next-chapter button jumps to the start of chapter 2.
	await mini.getByRole('button', { name: /Open the player/ }).click();
	const player = page.getByRole('dialog', { name: 'Player' });
	await expect(player.getByRole('heading', { name: 'The Glass Meridian' })).toBeVisible();
	const before = await audioTime(page);
	const chapter = await player.locator('.chapter').textContent();
	await player.getByRole('button', { name: 'Next chapter' }).click();
	await expect.poll(() => audioTime(page)).toBeGreaterThan(before + 30);
	await expect(player.locator('.chapter')).not.toHaveText(chapter ?? '');

	// Speed is applied to the audio.
	await player.getByRole('button', { name: /Speed/ }).click();
	await page.getByRole('button', { name: '1.5×' }).click();
	expect(await page.evaluate(() => document.querySelector('audio')?.playbackRate)).toBe(1.5);

	// Pause: the position is saved. After a reload the book comes back, paused, at that place.
	await player.getByRole('button', { name: 'Pause', exact: true }).click();
	const at = await audioTime(page);
	await page.waitForTimeout(1500);
	await page.reload();
	await expect(page.getByRole('region', { name: 'Now playing' })).toContainText('The Glass Meridian');
	await expect(page.getByRole('main').getByRole('button', { name: /^Resume from/ })).toBeVisible();
	await page.goto('#/');
	await expect(page.getByText('Currently reading')).toBeVisible();
	await expect(page.locator('.hero-title')).toHaveText('The Glass Meridian');
	await page.getByRole('button', { name: /Open the player/ }).click();
	await expect.poll(() => audioTime(page)).toBeGreaterThan(at - 3);
	expect(await audioTime(page)).toBeLessThan(at + 3);
	expect(await page.evaluate(() => document.querySelector('audio')?.paused)).toBe(true);
});

test('switching to data saver keeps playing from the same place', async ({ page }) => {
	await page.goto('#/library');
	await page.getByRole('link', { name: /Small Engines of the Universe/ }).first().click();
	await page.getByRole('main').getByRole('button', { name: /^(Play|Resume)/ }).click();
	await expectPlaying(page);
	await page.getByRole('button', { name: /Open the player/ }).click();
	const player = page.getByRole('dialog', { name: 'Player' });
	await player.getByRole('button', { name: 'Pause', exact: true }).click();
	const at = await audioTime(page);

	await player.getByRole('button', { name: 'More' }).click();
	await page.getByRole('button', { name: 'Data saver 48 kb/s' }).click();
	await expect(player.getByText('Data saver · 48 kb/s')).toBeVisible();
	await player.getByRole('button', { name: 'Play', exact: true }).click();
	// The data-saver stream is transcoded by the jailed ffmpeg as it's asked for.
	await expect.poll(() => audioTime(page), { timeout: 20_000 }).toBeGreaterThan(at + 1);
	expect(await audioTime(page)).toBeLessThan(at + 20);
	expect(await page.evaluate(() => document.querySelector('audio')?.currentSrc ?? '')).toContain('/v1/media/');
	await player.getByRole('button', { name: 'Pause', exact: true }).click();
});
