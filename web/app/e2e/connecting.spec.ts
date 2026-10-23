// A slow connection at start-up: the app opens as "Connecting…" for the person who last used
// this device, never "Offline", and carries on when the answer comes.
import { expect, test } from './helpers';

// Playwright can't hold back requests the service worker makes on the page's behalf.
test.use({ serviceWorkers: 'block' });

test('a slow sign-in answer opens the app as connecting, then carries on', async ({ page }) => {
	await page.goto('#/');
	await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible();

	let release: () => void = () => {};
	const held = new Promise<void>((r) => (release = r));
	await page.route('**/v1/auth/refresh', async (route) => {
		await held;
		await route.continue();
	});
	const reports: string[] = [];
	page.on('request', (r) => {
		if (r.url().endsWith('/v1/diag')) reports.push(r.postData() ?? '');
	});

	await page.reload();
	const banner = page.getByText('Connecting… The connection is slow. Downloaded books play now.');
	await expect(banner).toBeVisible({ timeout: 6_000 });
	await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible();
	// However long it takes, it is still connecting, not offline.
	await page.waitForTimeout(10_000);
	await expect(banner).toBeVisible();
	await expect(page.getByText('Offline. Downloaded books still play.')).toHaveCount(0);

	release();
	await expect(banner).toHaveCount(0, { timeout: 10_000 });
	await expect(page.getByRole('heading', { level: 1 }).first()).toBeVisible();
	expect(reports).toEqual([]);
});
