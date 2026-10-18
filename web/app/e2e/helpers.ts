// Shared bits for the browser tests.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { test as base, expect, request, type Page } from '@playwright/test';
import { ORIGIN, STATE_DIR } from './server';

export const PASSWORD = 'correct horse battery staple demo';
export const AUTH = path.join(STATE_DIR, 'ada.json');

/** The current TOTP code for a base32 secret (RFC 6238, SHA-1, 6 digits, 30 s). */
export function totp(secret: string, at = Date.now()): string {
	const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
	let bits = '';
	for (const c of secret.replace(/=+$/, '').toUpperCase()) bits += alphabet.indexOf(c).toString(2).padStart(5, '0');
	const key = Buffer.from(bits.match(/.{8}/g)!.map((b) => parseInt(b, 2)));
	const counter = Buffer.alloc(8);
	counter.writeBigUInt64BE(BigInt(Math.floor(at / 30_000)));
	const h = crypto.createHmac('sha1', key).update(counter).digest();
	const o = h[h.length - 1]! & 15;
	return String((h.readUInt32BE(o) & 0x7fffffff) % 1e6).padStart(6, '0');
}

/** The test runner, with one addition: after every test the context's cookies are saved
 *  back to AUTH. Loading the app rotates the refresh token, and the server signs a device
 *  out when a spent token is replayed too often (SPEC D15), so each test must start from the
 *  newest token, not the one saved at sign-up. */
/** The server allows a device 30 session renewals a minute (SPEC D15, `MAX_RENEWALS_PER_MINUTE`),
 *  and these tests load the app far faster than a person would, all on one device. So each
 *  test waits, if need be, until fewer than 20 renewals fall in the last minute. */
const RENEWALS = path.join(STATE_DIR, 'renewals.json');
const RENEWAL_BUDGET = 20;
async function paceRenewals() {
	for (;;) {
		const now = Date.now();
		let times: number[] = [];
		try {
			times = (JSON.parse(fs.readFileSync(RENEWALS, 'utf8')) as number[]).filter((t) => now - t < 60_000);
		} catch {
			/* none yet */
		}
		if (times.length < RENEWAL_BUDGET) return;
		await new Promise((r) => setTimeout(r, Math.min(...times) + 60_000 - now + 250));
	}
}
function noteRenewal() {
	let times: number[] = [];
	try {
		times = JSON.parse(fs.readFileSync(RENEWALS, 'utf8')) as number[];
	} catch {
		/* none yet */
	}
	fs.writeFileSync(RENEWALS, JSON.stringify([...times.filter((t) => Date.now() - t < 60_000), Date.now()]));
}

/** The test runner, with one addition: after every test the context's cookies are saved
 *  back to AUTH. Loading the app rotates the refresh token, and the server signs a device
 *  out when a spent token is replayed too often (SPEC D15), so each test must start from the
 *  newest token, not the one saved at sign-up. Renewals are paced (above). */
export const test = base.extend<{ keepSession: void }>({
	keepSession: [
		async ({ context }, use) => {
			await paceRenewals();
			context.on('request', (r) => {
				if (r.url().endsWith('/v1/auth/refresh')) noteRenewal();
			});
			await use();
			if (fs.existsSync(AUTH)) await context.storageState({ path: AUTH }).catch(() => {});
		},
		{ auto: true },
	],
});
export { expect };

export const invite = () => fs.readFileSync(path.join(STATE_DIR, 'invite.txt'), 'utf8').trim();

/** The tests' own device: signed in from Node at sign-up (e2e/enrol.setup.ts), separate from
 *  the browser's session so that its token rotations never race the app's. */
export const API_AUTH = path.join(STATE_DIR, 'api-device.json');
const CLIENT = { 'X-ABShare-Client': '1', Origin: ORIGIN };

/** Signs in as a separate device with the password and an authenticator code. */
export async function signInApiDevice(secret: string) {
	const ctx = await request.newContext({ baseURL: `${ORIGIN}/`, extraHTTPHeaders: CLIENT });
	const r = await ctx.post('v1/auth/login/password', { data: { username: 'ada', password: PASSWORD, device_name: 'e2e helper' } });
	expect(r.ok(), `login: ${r.status()} ${await r.text()}`).toBe(true);
	const { challenge } = await r.json();
	// Each step's code works once, and sign-up used this one; the next step is accepted too.
	const at = Date.now() + 30_000;
	const t = await ctx.post('v1/auth/login/totp', { data: { challenge, code: totp(secret, at) } });
	usedStep(at);
	expect(t.ok(), `totp: ${t.status()} ${await t.text()}`).toBe(true);
	await ctx.storageState({ path: API_AUTH });
	await ctx.dispose();
}

/** Calls the API as the tests' own device. */
export async function apiCall<T>(method: string, url: string, body?: unknown): Promise<T> {
	const ctx = await request.newContext({ baseURL: `${ORIGIN}/`, extraHTTPHeaders: CLIENT, storageState: API_AUTH });
	try {
		const r = await ctx.post('v1/auth/refresh');
		expect(r.ok(), `refresh: ${r.status()}`).toBe(true);
		const { access_token } = await r.json();
		const resp = await ctx.fetch(url, { method, headers: { Authorization: `Bearer ${access_token}` }, data: body });
		expect(resp.ok(), `${method} ${url}: ${resp.status()}`).toBe(true);
		return (resp.status() === 204 ? null : await resp.json()) as T;
	} finally {
		await ctx.storageState({ path: API_AUTH });
		await ctx.dispose();
	}
}

export interface Book {
	type: string;
	id: string;
	title: string;
	duration_ms: number;
}

/** Every book in the demo library, by title. */
export async function books(): Promise<Map<string, Book>> {
	const r = await apiCall<{ items: Book[] }>('GET', 'v1/books?sort=title&limit=100');
	return new Map(r.items.filter((b) => b.type === 'book').map((b) => [b.title, b]));
}

/** Sets a position (as a fraction of the book) or marks it finished. */
export async function setProgress(book: Book, at: number | 'finished', when = Date.now()) {
	const body = at === 'finished' ? { action: 'finished' } : { action: 'position', position_ms: Math.round(book.duration_ms * at) };
	await apiCall('PUT', `v1/progress/${book.id}`, { key: `e2e${crypto.randomBytes(8).toString('hex')}`, event_time_ms: when, ...body });
}

/** The audio element's current time, in seconds. */
export const audioTime = (page: Page) => page.evaluate(() => document.querySelector('audio')?.currentTime ?? -1);

/** Waits until the audio is really playing: not paused, and its time moving. */
export async function expectPlaying(page: Page, timeout = 20_000) {
	await expect.poll(() => page.evaluate(() => document.querySelector('audio')?.paused === false), { timeout }).toBe(true);
	const t0 = await audioTime(page);
	await expect.poll(() => audioTime(page), { timeout }).toBeGreaterThan(t0 + 1);
}

// ---------------------------------------------------------------- step-up

const SECRET = path.join(STATE_DIR, 'secret.txt');
const LAST_STEP = path.join(STATE_DIR, 'last-step.txt');
export const saveSecret = (s: string) => fs.writeFileSync(SECRET, s);
/** Records that the authenticator code for the time step of `at` has been used (each works once). */
function usedStep(at: number) {
	fs.writeFileSync(LAST_STEP, String(Math.floor(at / 30_000)));
}

/** Answers the "Confirm it's you" sheet with the password and a code not used before. */
export async function confirmItsYou(page: Page) {
	const sheet = page.getByRole('dialog', { name: 'Confirm it’s you' });
	await expect(sheet).toBeVisible();
	const last = fs.existsSync(LAST_STEP) ? Number(fs.readFileSync(LAST_STEP, 'utf8')) : 0;
	while (Math.floor(Date.now() / 30_000) <= last) await page.waitForTimeout(1000);
	const now = Date.now();
	await sheet.getByLabel('Password').fill(PASSWORD);
	await sheet.getByLabel('Authenticator code').fill(totp(fs.readFileSync(SECRET, 'utf8'), now));
	usedStep(now);
	await sheet.getByRole('button', { name: 'Confirm' }).click();
	await expect(sheet).toBeHidden();
}

/** Reports listening of `book` as the tests' own device: `secs` seconds from `fromMs`, one
 *  report every 15 s, ending `agoMs` before now. */
export async function listened(book: Book, fromMs: number, secs: number, agoMs: number) {
	const start = Date.now() - agoMs - secs * 1000;
	for (let i = 0; i <= secs / 15; i++) {
		await apiCall('PUT', `v1/progress/${book.id}`, {
			key: `e2e${crypto.randomBytes(8).toString('hex')}`,
			event_time_ms: start + i * 15_000,
			action: 'position',
			position_ms: fromMs + i * 15_000,
		});
	}
}
