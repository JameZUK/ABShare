// The signed-in session. The access token lives only in memory; the refresh cookie (HttpOnly,
// path /v1/auth) renews it, and also renews the media cookie the <audio> element uses.
import { ApiError, NetworkError, bearer, callJson } from './http';
import type { SignedIn, User } from './types';

const LAST_USER = 'abshare.lastUser';

type Status = 'starting' | 'signed-out' | 'signed-in' | 'offline';

function readLastUser(): User | null {
	try {
		const v = JSON.parse(localStorage.getItem(LAST_USER) ?? 'null');
		return v && typeof v.username === 'string' ? (v as User) : null;
	} catch {
		return null;
	}
}

function writeLastUser(u: User | null) {
	try {
		if (u) localStorage.setItem(LAST_USER, JSON.stringify(u));
		else localStorage.removeItem(LAST_USER);
	} catch {
		/* not essential */
	}
}

/** Runs `fn` while holding a lock shared by every tab, so only one tab rotates the refresh token at a time. */
async function exclusive<T>(fn: () => Promise<T>): Promise<T> {
	const locks = typeof navigator !== 'undefined' ? navigator.locks : undefined;
	return locks ? locks.request('abshare-refresh', fn) : fn();
}

class Session {
	status = $state<Status>('starting');
	user = $state<User | null>(null);
	#token: string | null = null;
	#expires = 0;
	#refreshing: Promise<boolean> | null = null;
	#timer: ReturnType<typeof setTimeout> | undefined;
	/** Called once whenever the session ends. `revoked`: this device was signed out remotely,
	 *  so its downloads must go (SPEC §8.2); `user` is whose they are. */
	onEnd: ((reason: 'signed-out' | 'expired' | 'revoked', user: string | null) => void) | null = null;

	/** Signs in from the refresh cookie, or falls back to offline mode with the last user. */
	async start() {
		const ok = await this.refresh();
		if (ok) return;
		if (this.status === 'offline') return;
		this.status = 'signed-out';
	}

	/** Takes a successful sign-in or refresh answer. */
	accept(r: SignedIn) {
		// Someone else signing in on this page (an invite opened while signed in): end the
		// old session first, so its player and queued progress don't carry over (review W2).
		if (this.user && this.user.username !== r.user.username && (this.status === 'signed-in' || this.status === 'offline')) {
			this.#end('signed-out');
		}
		this.#token = r.access_token;
		this.#expires = Date.now() + r.expires_in * 1000;
		this.user = r.user;
		this.status = 'signed-in';
		writeLastUser(r.user);
		clearTimeout(this.#timer);
		// Renew a minute early, so the media cookie and token never lapse mid-listen.
		this.#timer = setTimeout(() => void this.refresh(), Math.max(30_000, r.expires_in * 1000 - 60_000));
	}

	/** Renews the tokens; one renewal at a time per page and across tabs. */
	refresh(): Promise<boolean> {
		this.#refreshing ??= exclusive(async () => {
			try {
				this.accept(await callJson<SignedIn>('v1/auth/refresh', { method: 'POST' }));
				return true;
			} catch (e) {
				if (e instanceof NetworkError || (e instanceof ApiError && e.status >= 500)) {
					// Can't reach the server: keep going offline with downloaded books.
					const last = this.user ?? readLastUser();
					if (last) {
						this.user = last;
						this.status = 'offline';
					}
					clearTimeout(this.#timer);
					this.#timer = setTimeout(() => void this.refresh(), 30_000);
					return false;
				}
				// Too many renewals in a minute (several tabs, or quick reloads): the token is still
				// good. With a token in hand, carry on with it; with none yet (a page starting up),
				// wait as asked and try again rather than signing out.
				if (e instanceof ApiError && e.status === 429) {
					if (this.#token !== null) return true;
					for (let attempt = 0; attempt < 4; attempt++) {
						await new Promise((r) => setTimeout(r, Math.min(e.retryAfter ?? 2, 15) * 1000));
						try {
							this.accept(await callJson<SignedIn>('v1/auth/refresh', { method: 'POST' }));
							return true;
						} catch (again) {
							if (!(again instanceof ApiError && again.status === 429)) break;
						}
					}
					return false;
				}
				// session_reused (a copied or replayed sign-in) signs out but keeps downloads; only
				// an explicit remote sign-out removes them.
				if (e instanceof ApiError && e.code === 'device_revoked') {
					this.#end('revoked');
					writeLastUser(null);
					this.user = null;
					return false;
				}
				this.#end('expired');
				return false;
			}
		}).finally(() => {
			this.#refreshing = null;
		});
		return this.#refreshing;
	}

	#end(reason: 'signed-out' | 'expired' | 'revoked') {
		const was = this.status;
		const user = this.user?.username ?? readLastUser()?.username ?? null;
		this.#token = null;
		this.#expires = 0;
		clearTimeout(this.#timer);
		this.status = 'signed-out';
		// Revocation is acted on even at start-up, before anyone was signed in on this page.
		if (was === 'signed-in' || was === 'offline' || reason === 'revoked') this.onEnd?.(reason, user);
	}

	/** An authenticated API call; renews the token once on 401. */
	async api<T>(path: string, init: RequestInit = {}): Promise<T> {
		if (!this.#token || Date.now() > this.#expires - 5_000) {
			if (!(await this.refresh())) throw this.status === 'offline' ? new NetworkError() : new ApiError(401, 'unauthenticated', 'Please sign in.');
		}
		const call = () => callJson<T>(path, { ...init, headers: { ...bearer(this.#token ?? ''), ...(init.headers as Record<string, string> | undefined) } });
		try {
			return await call();
		} catch (e) {
			if (e instanceof ApiError && e.status === 401 && (await this.refresh())) return call();
			throw e;
		}
	}

	get token() {
		return this.#token;
	}

	/** Signs out this device. `forget` also forgets who last used it. */
	async signOut(forget: boolean) {
		try {
			await callJson('v1/auth/logout', { method: 'POST' });
		} catch {
			/* signed out locally either way */
		}
		if (forget) writeLastUser(null);
		this.#end('signed-out');
		this.user = null;
	}
}

export const session = new Session();

/** Shorthand for authenticated JSON calls. */
export const api = <T>(path: string, init: RequestInit = {}) => session.api<T>(path, init);
export const post = <T>(path: string, body: unknown, method = 'POST') => session.api<T>(path, { method, body: JSON.stringify(body) });
