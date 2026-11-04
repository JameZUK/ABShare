// Calls (SPEC D38). The browser pauses the book for a call and starts it again when the call
// ends, straight to the audio element. These tests do the same from outside the app, then check
// the player follows the listener's choice: carry on, wait first, or stay paused. The listener's
// own pause always holds.
import type { Page } from '@playwright/test';
import { expect, expectPlaying, test } from './helpers';

/** What the browser does when a call starts and ends: pause and play the element directly. */
const browserPauses = (page: Page) => page.evaluate(() => document.querySelector('audio')!.pause());
const browserPlays = (page: Page) => page.evaluate(() => void document.querySelector('audio')!.play().catch(() => {}));
/** Opens Playback settings by tapping through the app: page.goto would load the app again and
 *  stop the book. */
async function openPlaybackSettings(page: Page) {
	await page.getByRole('navigation', { name: 'Main' }).getByRole('link', { name: 'Settings' }).click();
	await page.getByRole('link', { name: /Playback/ }).click();
	await expect(page.getByRole('heading', { name: 'Playback' })).toBeVisible();
}
const audio = (page: Page) => page.evaluate(() => ({ paused: document.querySelector('audio')!.paused, muted: document.querySelector('audio')!.muted }));

async function startBook(page: Page) {
	await page.goto('#/library');
	await page.getByRole('link', { name: /The Glass Meridian/ }).first().click();
	await page.getByRole('main').getByRole('button', { name: /^(Play|Resume)/ }).click();
	await expectPlaying(page);
}

// Settings are kept in the browser, and the tests share one saved browser state: put back the defaults.
test.afterEach(async ({ page }, info) => {
	// A failure comes with the player's own account of what happened.
	if (info.status !== info.expectedStatus) {
		const log = await page.evaluate(() => localStorage.getItem('abshare.playbackLog') ?? '[]');
		await info.attach('playback log', { body: (JSON.parse(log) as string[]).join('\n'), contentType: 'text/plain' });
		console.log('PLAYBACK LOG\n' + (JSON.parse(log) as string[]).join('\n'));
	}
	await page.evaluate(() => {
		const p = JSON.parse(localStorage.getItem('abshare.prefs') ?? '{}') as Record<string, unknown>;
		localStorage.setItem('abshare.prefs', JSON.stringify({ ...p, resumeAfterCall: true, resumeDelay: 0 }));
	});
});

test('after a call the book carries on by itself, but never undoes the listener’s pause', async ({ page }) => {
	await startBook(page);
	const mini = page.getByRole('region', { name: 'Now playing' });

	await browserPauses(page);
	await expect(mini).toContainText('Paused by your phone');
	await browserPlays(page);
	await expectPlaying(page);
	await expect(mini).not.toContainText('Paused by your phone');

	// The listener pauses; a restart from the browser afterwards is paused again, silently.
	await mini.getByRole('button', { name: 'Pause', exact: true }).click();
	await expect.poll(() => audio(page).then((a) => a.paused)).toBe(true);
	await page.waitForTimeout(1_600);
	await browserPlays(page);
	await expect.poll(() => audio(page)).toEqual({ paused: true, muted: false });
	await page.waitForTimeout(1_000);
	expect(await audio(page)).toEqual({ paused: true, muted: false });
	await expect(mini.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
});

test('the wait after a call is kept to, and switched off the book stays paused', async ({ page }) => {
	await page.goto('#/settings/playback');
	await page.getByLabel('Wait after the call').selectOption('3');
	await startBook(page);
	const mini = page.getByRole('region', { name: 'Now playing' });

	await browserPauses(page);
	await browserPlays(page);
	await expect(mini).toContainText('Carrying on in 3 seconds');
	expect((await audio(page)).paused, 'held, not playing, during the wait').toBe(true);
	await page.waitForTimeout(1_500);
	expect((await audio(page)).paused, 'still waiting').toBe(true);
	await expectPlaying(page, 10_000);
	expect((await audio(page)).muted, 'never left muted').toBe(false);

	await openPlaybackSettings(page);
	await page.getByRole('checkbox', { name: /Carry on after a call/ }).uncheck();
	expect((await audio(page)).paused, 'still playing after moving to Settings').toBe(false);
	await expect(page.getByLabel('Wait after the call')).toBeDisabled();
	await browserPauses(page);
	await browserPlays(page);
	await page.waitForTimeout(4_500);
	expect(await audio(page)).toEqual({ paused: true, muted: false });
	await expect(mini).toContainText('Paused by your phone');

	// Pressing play after the call works as normal.
	await mini.getByRole('button', { name: 'Play', exact: true }).click();
	await expectPlaying(page);
	await mini.getByRole('button', { name: 'Pause', exact: true }).click();
});

test('a playback report shows what will be sent and reaches the admins', async ({ page }) => {
	await startBook(page);
	await browserPauses(page);
	await page.getByRole('region', { name: 'Now playing' }).getByRole('button', { name: 'Play', exact: true }).click();
	await expectPlaying(page);
	await page.getByRole('region', { name: 'Now playing' }).getByRole('button', { name: 'Pause', exact: true }).click();

	await page.goto('#/settings/about');
	await page.getByText(/What will be sent/).click();
	const log = page.locator('pre.log');
	await expect(log).toContainText('pause (by the browser)');
	await expect(log).toContainText('play (app)');
	await expect(log).not.toContainText('Glass Meridian');
	await page.getByRole('button', { name: 'Send a playback report' }).click();
	await expect(page.getByRole('button', { name: 'Report sent' })).toBeDisabled();

	await page.goto('#/admin');
	await expect(page.getByRole('heading', { name: /Playback reports/ })).toBeVisible();
	const report = page.locator('.reports details').first();
	await expect(report.locator('summary'), 'with who sent it').toContainText('ada');
	await report.locator('summary').click();
	await expect(report).toContainText('pause (by the browser)');
	await expect(report).toContainText('Browser: ');
});

test.describe('a call while the book is starting', () => {
	// Playwright can't hold back requests the service worker makes on the page's behalf.
	test.use({ serviceWorkers: 'block' });

	test('waits for the call to end instead of starting again at once', async ({ page }) => {
		// Hold the audio back, so the app's play is still waiting when the call comes.
		await page.route('**/v1/media/**', async (route) => {
			const url = route.request().url();
			if (url.includes('/covers/') || url.endsWith('/state')) return route.continue();
			await new Promise((r) => setTimeout(r, 2_500));
			await route.continue().catch(() => {});
		});
		await page.goto('#/library');
		await page.getByRole('link', { name: /The Glass Meridian/ }).first().click();
		await page.getByRole('main').getByRole('button', { name: /^(Play|Resume)/ }).click();
		await expect.poll(() => audio(page).then((a) => a.paused)).toBe(false);
		expect(await page.evaluate(() => document.querySelector('audio')!.readyState), 'still starting').toBeLessThan(3);

		await browserPauses(page);
		const mini = page.getByRole('region', { name: 'Now playing' });
		await expect(mini).toContainText('Paused by your phone');
		await page.waitForTimeout(4_000);
		expect((await audio(page)).paused, 'not started again during the call').toBe(true);

		await browserPlays(page);
		await expectPlaying(page);
		await mini.getByRole('button', { name: 'Pause', exact: true }).click();
	});
});
