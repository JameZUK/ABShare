// Browser tests: the built app against a real, fresh server on the invented demo library
// (e2e/server.ts). `npm run e2e`. One worker: the tests share one server and one account.
//
// Projects run in order: `setup` signs up from the invite; `visual` compares screenshots
// (before anything else changes the account); `app` runs the flows.
//
// Reference screenshots differ slightly between machines (font rasterising), so CI keeps its
// own under e2e/__screenshots__/ci, made by CI itself (see docs/BUILDING.md); local runs use
// e2e/__screenshots__/local, which is not committed.
import { defineConfig } from '@playwright/test';
import { AUTH } from './e2e/helpers';
import { ORIGIN } from './e2e/server';

const where = process.env.CI ? 'ci' : 'local';

export default defineConfig({
	testDir: 'e2e',
	globalSetup: './e2e/server.ts',
	workers: 1,
	fullyParallel: false,
	forbidOnly: !!process.env.CI,
	retries: process.env.CI ? 1 : 0,
	timeout: 60_000,
	reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
	snapshotPathTemplate: `{testDir}/__screenshots__/${where}/{arg}{ext}`,
	expect: {
		timeout: 10_000,
		toHaveScreenshot: { maxDiffPixelRatio: 0.005, animations: 'disabled', caret: 'hide' },
	},
	use: {
		baseURL: `${ORIGIN}/`,
		viewport: { width: 390, height: 844 },
		isMobile: true,
		hasTouch: true,
		colorScheme: 'light',
		locale: 'en-GB',
		timezoneId: 'Europe/London',
		trace: 'retain-on-failure',
		screenshot: 'only-on-failure',
		launchOptions: { args: ['--autoplay-policy=no-user-gesture-required'] },
	},
	projects: [
		{ name: 'setup', testMatch: /enrol\.setup\.ts/ },
		{ name: 'visual', testMatch: /visual\.spec\.ts/, dependencies: ['setup'], use: { storageState: AUTH } },
		{ name: 'app', testMatch: /\.spec\.ts/, testIgnore: /visual\.spec\.ts/, dependencies: ['visual'], use: { storageState: AUTH } },
	],
});
