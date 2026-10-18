// Admin → Server settings: shown, never changeable.
import { expect, test } from './helpers';
import { ORIGIN, PORT } from './server';

test('admins see the server settings, read-only and without file paths', async ({ page }) => {
	await page.goto('#/admin');
	await page.getByRole('navigation', { name: 'Admin' }).getByRole('link', { name: 'Settings' }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Server settings' })).toBeVisible();
	const main = page.getByRole('main');
	await expect(main.getByText(ORIGIN, { exact: true })).toBeVisible();
	await expect(main.getByText(`127.0.0.1:${PORT}`, { exact: true })).toBeVisible();
	await expect(main.getByText('Audiobooks', { exact: true })).toBeVisible();
	await expect(main.getByText('10 GiB', { exact: true })).toBeVisible();
	await expect(main.getByText(/None: client addresses come from the connection/)).toBeVisible();
	await expect(main.getByText('Single use, valid for 72 hours')).toBeVisible();
	// Nothing to edit, and no paths from the config file (the e2e library lives in a temp dir).
	await expect(main.locator('input, select, textarea')).toHaveCount(0);
	await expect(main.getByRole('button')).toHaveCount(0);
	await expect(main).not.toContainText('/tmp/');
	await expect(main).not.toContainText('abshare-jail');
});
