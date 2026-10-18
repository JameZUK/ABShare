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
export const test = base.extend<{ keepSession: void }>({
	keepSession: [
		async ({ context }, use) => {
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
	const t = await ctx.post('v1/auth/login/totp', { data: { challenge, code: totp(secret, Date.now() + 30_000) } });
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
