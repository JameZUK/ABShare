// Reviews, recommendations, "Where was I?", and the admin listening pages.
import { books, confirmItsYou, expect, expectPlaying, listened, test } from './helpers';
import { audioTime } from './helpers';

test('a review is written, shown with its stars, edited and deleted', async ({ page }) => {
	await page.goto('#/library');
	await page.getByRole('link', { name: /The Clockwork Fox/ }).first().click();
	const reviews = page.locator('#reviews');
	await expect(reviews).toBeVisible();
	await expect(page.getByText('No reviews yet', { exact: true })).toBeVisible();

	const save = reviews.getByRole('button', { name: 'Save' });
	await expect(save).toBeDisabled();
	await reviews.getByRole('radio', { name: '4 stars' }).check();
	const comment = reviews.getByRole('textbox');
	// Over 50 words: the counter warns and Save stays off.
	await comment.fill(Array(51).fill('word').join(' '));
	await expect(reviews.getByText('51 / 50 words')).toBeVisible();
	await expect(save).toBeDisabled();
	await comment.fill('A delight from start to finish. <b>Not bold.</b>');
	await save.click();

	await expect(reviews.getByText('A delight from start to finish. <b>Not bold.</b>').first()).toBeVisible();
	await expect(page.getByRole('img', { name: '4 out of 5 stars' }).first()).toBeVisible();
	await expect(page.getByText(/4(\.0)? · 1 review/)).toBeVisible();
	await expect(reviews.locator('b')).toHaveCount(0);

	await reviews.getByRole('button', { name: 'Edit' }).click();
	await reviews.getByRole('radio', { name: '5 stars' }).check();
	await reviews.getByRole('button', { name: 'Save' }).click();
	await expect(page.getByText(/5(\.0)? · 1 review/)).toBeVisible();

	await reviews.getByRole('button', { name: 'Delete' }).click();
	await expect(page.getByText('No reviews yet', { exact: true })).toBeVisible();
});

test('a recommendation reaches Home with its note, and Not now puts it away', async ({ page }) => {
	await page.goto('#/admin/recommend');
	await page.getByPlaceholder('Search titles, authors, narrators').fill('Cartographer');
	await page.getByRole('button', { name: /The Cartographer/ }).first().click();
	await page.getByLabel('Why you recommend it').fill('A map of a family, beautifully read.');
	await page.getByRole('radio', { name: 'Everyone' }).check();
	await page.getByRole('button', { name: 'Send recommendation' }).click();
	await confirmItsYou(page);
	await expect(page.getByText('A map of a family, beautifully read.').first()).toBeVisible();

	await page.goto('#/');
	const section = page.getByRole('region', { name: 'Recommended for you' }).or(page.locator('section', { hasText: 'Recommended for you' })).first();
	await expect(section).toContainText('A map of a family, beautifully read.');
	await expect(section).toContainText('The Cartographer');
	await section.getByRole('button', { name: 'Not now' }).click();
	await expect(page.getByText('A map of a family, beautifully read.')).toHaveCount(0);
	await page.reload();
	await expect(page.getByRole('heading', { name: /Welcome back/ })).toBeVisible();
	await expect(page.getByText('A map of a family, beautifully read.')).toHaveCount(0);
});

test('Where was I? lists your stretches and plays from the one you tap', async ({ page }) => {
	const all = await books();
	const book = all.get('Holding the Line')!;
	// Last night: from 1:00 for a minute and a half, then a rewind to 0:10 for a minute.
	await listened(book, 60_000, 90, 600_000);
	await listened(book, 10_000, 60, 300_000);

	await page.goto('#/library');
	await page.getByRole('link', { name: /Holding the Line/ }).first().click();
	await page.getByRole('main').getByRole('button', { name: 'More' }).click();
	await page.getByRole('button', { name: /Where was I/ }).click();
	const sheet = page.getByRole('dialog', { name: 'Where was I?' });
	await expect(sheet.getByRole('button', { name: /^Play from 0:10/ })).toBeVisible();
	await expect(sheet.getByRole('button', { name: /^Play from 1:00/ })).toBeVisible();
	await sheet.getByRole('button', { name: /^Play from 1:00/ }).click();

	await expect(page.getByRole('dialog', { name: 'Player' })).toBeVisible();
	await expectPlaying(page);
	const t = await audioTime(page);
	expect(t).toBeGreaterThan(59);
	expect(t).toBeLessThan(75);
	await page.getByRole('dialog', { name: 'Player' }).getByRole('button', { name: 'Pause', exact: true }).click();
});

test('admins see everyone’s listening, and each person’s books without positions', async ({ page }) => {
	await page.goto('#/admin/listening');
	await expect(page.getByRole('heading', { level: 1, name: 'Listening' })).toBeVisible();
	await page.getByRole('link', { name: /Ada Lovelace/ }).click();
	await expect(page.getByRole('heading', { level: 1, name: 'Ada Lovelace' })).toBeVisible();
	await expect(page.getByRole('heading', { name: 'Books' })).toBeVisible();
	await expect(page.getByText('Holding the Line').first()).toBeVisible();
	// The History page no longer claims who can see it.
	await page.goto('#/settings/history');
	await expect(page.getByText(/administrators can’t/)).toHaveCount(0);
});

test('finishing a book asks once how it was', async ({ page }) => {
	await page.goto('#/library');
	await page.getByRole('link', { name: /The Year Without a Summer/ }).first().click();
	await page.getByRole('main').getByRole('button', { name: 'More' }).click();
	await page.getByRole('button', { name: 'Mark finished' }).click();
	const prompt = page.getByRole('dialog', { name: 'How was it?' });
	await expect(prompt).toContainText('The Year Without a Summer');
	await prompt.getByRole('radio', { name: '4 stars' }).check();
	await prompt.getByRole('button', { name: 'Save' }).click();
	await expect(prompt).toBeHidden();
	await expect(page.getByText(/4(\.0)? · 1 review/)).toBeVisible();
	// Never asked again for that book.
	await page.getByRole('main').getByRole('button', { name: 'More' }).click();
	await page.getByRole('button', { name: 'Mark not finished' }).click();
	await page.getByRole('main').getByRole('button', { name: 'More' }).click();
	await page.getByRole('button', { name: 'Mark finished' }).click();
	await page.waitForTimeout(1500);
	await expect(page.getByRole('dialog', { name: 'How was it?' })).toBeHidden();
});

test('Downloads settings: quality, Wi-Fi only, and what this browser can tell', async ({ page }) => {
	await page.goto('#/settings');
	await page.locator('a[href="#/settings/downloads"]').click();
	await expect(page.getByRole('heading', { level: 1, name: 'Downloads' })).toBeVisible();
	const wifi = page.getByRole('checkbox', { name: /Download only on Wi-Fi/ });
	await wifi.check();
	// Desktop Chromium doesn't report the connection type, so nothing is held, and it says so.
	await expect(page.getByText('This browser doesn’t say whether you’re on Wi-Fi, so downloads aren’t held.')).toBeVisible();
	await page.reload();
	await expect(page.getByRole('checkbox', { name: /Download only on Wi-Fi/ })).toBeChecked();
	await page.getByRole('checkbox', { name: /Download only on Wi-Fi/ }).uncheck();
});
