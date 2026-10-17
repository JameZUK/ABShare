// Low-level calls to the ABShare /v1 API (SPEC §6).
//
// Every request carries X-ABShare-Client: a cross-site page can't send it without a CORS
// preflight the server never approves, which is the CSRF defence for the cookies.

export const CLIENT_HEADER = { 'X-ABShare-Client': '1' } as const;

export class ApiError extends Error {
	constructor(
		readonly status: number,
		/** The server's error code ("denied", "preparing", "factor_required", …). */
		readonly code: string,
		message: string,
		/** Seconds to wait before trying again, if the server said. */
		readonly retryAfter?: number,
	) {
		super(message);
		this.name = 'ApiError';
	}
}

/** The request never reached the server (offline, or the connection dropped). */
export class NetworkError extends Error {
	constructor() {
		super('ABShare can’t be reached. Check your connection.');
		this.name = 'NetworkError';
	}
}

/** A `Retry-After` in whole seconds (1 to a day); dates and nonsense give undefined. */
export function retryAfterOf(v: string | null): number | undefined {
	if (v === null || !/^\s*\d{1,9}\s*$/.test(v)) return undefined;
	return Math.min(Math.max(Number(v), 1), 86_400);
}

/** fetch() that returns parsed JSON (or null for 204) and throws ApiError on failure. */
export async function callJson<T>(path: string, init: RequestInit = {}): Promise<T> {
	let resp: Response;
	try {
		resp = await fetch(path, {
			credentials: 'same-origin',
			cache: 'no-store',
			...init,
			headers: { ...CLIENT_HEADER, ...(init.body ? { 'Content-Type': 'application/json' } : {}), ...(init.headers as Record<string, string> | undefined) },
		});
	} catch {
		throw new NetworkError();
	}
	if (resp.status === 204) return null as T;
	const text = await resp.text().catch(() => '');
	let parsed: unknown = undefined;
	if (text.trim()) {
		try {
			parsed = JSON.parse(text);
		} catch {
			parsed = undefined;
		}
	}
	const body = (parsed && typeof parsed === 'object' ? parsed : {}) as Record<string, unknown>;
	if (!resp.ok) {
		const code = typeof body.error === 'string' ? body.error : `http_${resp.status}`;
		const message = typeof body.message === 'string' ? body.message : `Something went wrong (${resp.status}).`;
		throw new ApiError(resp.status, code, message, retryAfterOf(resp.headers.get('retry-after')));
	}
	// A success that isn't JSON came from something in the way (a proxy's page), not ABShare.
	if (parsed === undefined) throw new ApiError(502, 'unexpected_answer', 'The server sent something unexpected.');
	return parsed as T;
}

export const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });

/** A message fit to show for any error thrown by the API layer. */
export function messageOf(e: unknown): string {
	if (e instanceof ApiError || e instanceof NetworkError) return e.message;
	if (e instanceof DOMException && e.name === 'NotAllowedError') return 'That was cancelled or timed out.';
	return 'Something went wrong. Please try again.';
}
