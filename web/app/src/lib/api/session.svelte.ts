// The signed-in session. The access token lives only in memory; the refresh cookie (HttpOnly,
// path /v1/auth) renews it, and also renews the media cookie the <audio> element uses.
//
// Signing out must reach the server: only it can end the session and clear that cookie. Until
// it confirms, a marker stays in local storage (shared by every tab), and while the marker is
// there nothing renews the session: the sign-out is sent again first (at start-up, when the
// connection returns, before any renewal).
import { ApiError, NetworkError, bearer, callJson } from './http';
import type { SignedIn, User } from './types';

const LAST_USER = 'abshare.lastUser';
export const SIGN_OUT_PENDING = 'abshare.signOutPending';
/** How long before a sign-out the server hasn't confirmed is sent again. */
const SIGN_OUT_RETRY_MS = 30_000;
/** Retries of a start-up renewal answered 429 (too many renewals in a minute). */
const MAX_START_RETRIES = 4;
/** How long start-up waits for the server before opening the app for whoever last used this
 *  device. Still connecting, not offline: on a slow connection the answer is coming, so
 *  nothing times out; downloaded books play meanwhile. */
export const CONNECTING_AFTER_MS = 3_000;

/** `connecting`: start-up is still waiting for the server, with the last person's app open.
 *  It ends in `signed-in`, `offline` (only if the request fails) or `signed-out`. */
type Status = 'starting' | 'connecting' | 'signed-out' | 'signed-in' | 'offline';

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

/** Used when local storage can't be: the marker then lasts as long as the page. */
let pendingHere = false;

/** Whether a sign-out is still to be confirmed by the server. */
export function signOutPending(): boolean {
	try {
		return localStorage.getItem(SIGN_OUT_PENDING) !== null || pendingHere;
	} catch {
		return pendingHere;
	}
}

function setSignOutPending(on: boolean) {
	pendingHere = on;
	try {
		if (on) localStorage.setItem(SIGN_OUT_PENDING, '1');
		else localStorage.removeItem(SIGN_OUT_PENDING);
	} catch {
		/* kept in memory */
	}
}

const unreachable = (e: unknown) => e instanceof NetworkError || (e instanceof ApiError && e.status >= 500);

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
	#signOutTimer: ReturnType<typeof setTimeout> | undefined;
	/** Called once whenever the session ends. `revoked`: this device was signed out remotely,
	 *  so its downloads must go (SPEC §8.2); `user` is whose they are. */
	onEnd: ((reason: 'signed-out' | 'expired' | 'revoked', user: string | null) => void) | null = null;

	/** Signs in from the refresh cookie, or falls back to offline mode with the last user. A
	 *  slow answer opens the app as `connecting` meanwhile, for whoever last used this device. */
	async start() {
		// With a sign-out still to confirm, nobody's app opens while the server is asked.
		const last = signOutPending() ? null : readLastUser();
		const slow = last
			? setTimeout(() => {
					if (this.status !== 'starting') return;
					this.user = last;
					this.status = 'connecting';
				}, CONNECTING_AFTER_MS)
			: undefined;
		try {
			const ok = await this.refresh();
			if (ok || this.status === 'offline') return;
			// Ends anything started while connecting (a downloaded book playing).
			if (this.status === 'starting' || this.status === 'connecting') this.#end('expired');
		} finally {
			clearTimeout(slow);
		}
	}

	/** Resolves once no renewal is in flight (at once if none is). */
	async settled(): Promise<void> {
		await this.#refreshing;
	}

	/** Takes a person's own sign-in (a passkey, a code, an invite). The server has started a
	 *  new session, replacing the old cookie, so a sign-out still waiting has nothing left to end. */
	accept(r: SignedIn) {
		clearTimeout(this.#signOutTimer);
		setSignOutPending(false);
		this.#take(r);
	}

	/** Takes a successful sign-in or refresh answer. */
	#take(r: SignedIn) {
		// Someone else signing in on this page (an invite opened while signed in): end the
		// old session first, so its player and queued progress don't carry over (review W2).
		// The same applies to the last person's app opened while connecting.
		if (this.user && this.user.username !== r.user.username && (this.status === 'signed-in' || this.status === 'offline' || this.status === 'connecting')) {
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
			// Every answer, including one to a retry after a 429, is handled the same way, so a
			// revocation or an outage during the wait isn't missed (review W12).
			for (let attempt = 0; ; attempt++) {
				try {
					// A sign-out the server hasn't confirmed comes first: renewing would sign the
					// person straight back in.
					if (signOutPending()) {
						try {
							await this.#logout();
						} catch (e) {
							// No token survives a sign-out, whatever happens next.
							this.#token = null;
							this.#expires = 0;
							// Unreachable: as any outage below (downloaded books, and another try later).
							if (unreachable(e)) throw e;
							// Refused: signed out here; asked again later.
							this.#end('signed-out');
							this.user = null;
							this.#retrySignOutLater();
							return false;
						}
					}
					const r = await callJson<SignedIn>('v1/auth/refresh', { method: 'POST' });
					// Signed out (in another tab) while that was on its way: don't take the answer;
					// go round again, which ends the session it has just renewed.
					if (signOutPending()) continue;
					this.#take(r);
					return true;
				} catch (e) {
					if (unreachable(e)) {
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
					// wait as asked (at most 15 s, 4 times) and try again rather than signing out.
					if (e instanceof ApiError && e.status === 429) {
						if (this.#token !== null) {
							// The timer that brought us here is spent: set it again, or the token and
							// the media cookie would lapse mid-listen.
							clearTimeout(this.#timer);
							this.#timer = setTimeout(() => void this.refresh(), Math.min(Math.max(e.retryAfter ?? 30, 5), 60) * 1000);
							return true;
						}
						if (attempt >= MAX_START_RETRIES) return false;
						await new Promise((r) => setTimeout(r, Math.min(e.retryAfter ?? 2, 15) * 1000));
						continue;
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
		if (was === 'signed-in' || was === 'offline' || was === 'connecting' || reason === 'revoked') this.onEnd?.(reason, user);
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

	/** Signs out this device. `forget` also forgets who last used it. Signed out here either
	 *  way; if the server can't be told now, it is told later, before anything renews the session. */
	async signOut(forget: boolean) {
		setSignOutPending(true);
		await this.finishSignOut();
		if (forget) writeLastUser(null);
		this.#end('signed-out');
		this.user = null;
	}

	/** Sends a sign-out the server hasn't confirmed, if there is one. True when none is left. */
	async finishSignOut(): Promise<boolean> {
		clearTimeout(this.#signOutTimer);
		if (!signOutPending()) return true;
		// Under the renewal lock, so it can't cross a renewal in another tab, whose answer
		// would otherwise put the cookie back.
		return exclusive(async () => {
			if (!signOutPending()) return true;
			try {
				await this.#logout();
				return true;
			} catch {
				this.#retrySignOutLater();
				return false;
			}
		});
	}

	/** Asks the server to end the session. Confirmed by any success, or by 401 (already signed
	 *  out); anything else throws and leaves the marker in place. */
	async #logout() {
		try {
			await callJson('v1/auth/logout', { method: 'POST' });
		} catch (e) {
			if (!(e instanceof ApiError && e.status === 401)) throw e;
		}
		setSignOutPending(false);
	}

	#retrySignOutLater() {
		clearTimeout(this.#signOutTimer);
		this.#signOutTimer = setTimeout(() => void this.finishSignOut(), SIGN_OUT_RETRY_MS);
	}

	/** Another tab signed out: this one's session is over too. */
	signedOutElsewhere() {
		if (this.status !== 'signed-in' && this.status !== 'offline' && this.status !== 'connecting') return;
		this.#end('signed-out');
		this.user = null;
	}
}

export const session = new Session();

if (typeof window !== 'undefined') {
	// A sign-out waiting for the server goes as soon as the connection is back.
	addEventListener('online', () => void session.finishSignOut());
	// Local storage tells every other tab when one signs out.
	addEventListener('storage', (e) => {
		if (e.key === SIGN_OUT_PENDING && e.newValue !== null) session.signedOutElsewhere();
	});
}

/** Shorthand for authenticated JSON calls. */
export const api = <T>(path: string, init: RequestInit = {}) => session.api<T>(path, init);
export const post = <T>(path: string, body: unknown, method = 'POST') => session.api<T>(path, { method, body: JSON.stringify(body) });
