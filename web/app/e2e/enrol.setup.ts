// Signing up from an invite with a password and an authenticator app, as a new person would.
import { expect, test } from '@playwright/test';
import { AUTH, PASSWORD, invite, saveSecret, signInApiDevice, totp } from './helpers';

test('a new person signs up from the invite', async ({ page }) => {
	await page.goto(invite());
	await expect(page.getByRole('heading', { name: 'Welcome, Ada Lovelace' })).toBeVisible();
	await page.getByLabel('Username').fill('ada');
	await page.getByRole('button', { name: 'Continue' }).click();

	await page.getByRole('button', { name: /password and an authenticator/i }).click();
	await page.getByLabel('Password', { exact: true }).fill(PASSWORD);
	await page.getByLabel('Password again').fill(PASSWORD);
	await page.getByRole('button', { name: /continue/i }).click();

	const secret = ((await page.locator('code.secret').textContent()) ?? '').replace(/\s+/g, '');
	expect(secret).toMatch(/^[A-Z2-7]{16,}$/);
	// A wrong code is refused, and the page says so.
	const code = page.getByLabel(/code/i).first();
	await code.fill(totp(secret) === '000000' ? '111111' : '000000');
	await page.getByRole('button', { name: /confirm|continue|verify/i }).first().click();
	await expect(page.getByRole('alert')).toBeVisible();
	await code.fill(totp(secret));
	await page.getByRole('button', { name: /confirm|continue|verify/i }).first().click();

	await expect(page.getByRole('heading', { name: 'Save your recovery codes' })).toBeVisible();
	await expect(page.locator('li', { hasText: /^[a-z0-9]{4}-[a-z0-9]{4}-[a-z0-9]{4}$/ })).toHaveCount(10);
	await page.getByRole('button', { name: /saved them/i }).click();

	await expect(page.getByRole('heading', { name: /Welcome back/ })).toBeVisible();
	await expect(page.getByRole('navigation', { name: 'Main' })).toBeVisible();
	await page.context().storageState({ path: AUTH });
	await signInApiDevice(secret);
	saveSecret(secret);
});
