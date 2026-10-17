// WebAuthn without the JSON helpers some browsers lack: converts the server's base64url
// JSON options to ArrayBuffers and the credential back.

export function b64uToBuf(s: string): ArrayBuffer {
	const pad = '='.repeat((4 - (s.length % 4)) % 4);
	const bin = atob((s + pad).replace(/-/g, '+').replace(/_/g, '/'));
	const out = new Uint8Array(bin.length);
	for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
	return out.buffer;
}

export function bufToB64u(buf: ArrayBuffer): string {
	let bin = '';
	for (const b of new Uint8Array(buf)) bin += String.fromCharCode(b);
	return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export const passkeysSupported = () => typeof window !== 'undefined' && 'PublicKeyCredential' in window && !!navigator.credentials;

type Json = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

/** Runs `navigator.credentials.create` from the server's options; returns the JSON to send back. */
export async function createPasskey(options: Json): Promise<Json> {
	const pk = structuredClone(options.publicKey) as Json;
	pk.challenge = b64uToBuf(pk.challenge);
	pk.user.id = b64uToBuf(pk.user.id);
	for (const c of pk.excludeCredentials ?? []) c.id = b64uToBuf(c.id);
	const cred = (await navigator.credentials.create({ publicKey: pk as PublicKeyCredentialCreationOptions })) as PublicKeyCredential | null;
	if (!cred) throw new DOMException('No passkey was created', 'NotAllowedError');
	const r = cred.response as AuthenticatorAttestationResponse;
	return {
		id: cred.id,
		rawId: bufToB64u(cred.rawId),
		type: cred.type,
		extensions: cred.getClientExtensionResults?.() ?? {},
		response: {
			attestationObject: bufToB64u(r.attestationObject),
			clientDataJSON: bufToB64u(r.clientDataJSON),
			transports: r.getTransports?.(),
		},
	};
}

/** Runs `navigator.credentials.get` from the server's options; returns the JSON to send back. */
export async function getPasskey(options: Json, signal?: AbortSignal): Promise<Json> {
	const pk = structuredClone(options.publicKey) as Json;
	pk.challenge = b64uToBuf(pk.challenge);
	for (const c of pk.allowCredentials ?? []) c.id = b64uToBuf(c.id);
	const cred = (await navigator.credentials.get({ publicKey: pk as PublicKeyCredentialRequestOptions, signal })) as PublicKeyCredential | null;
	if (!cred) throw new DOMException('No passkey was used', 'NotAllowedError');
	const r = cred.response as AuthenticatorAssertionResponse;
	return {
		id: cred.id,
		rawId: bufToB64u(cred.rawId),
		type: cred.type,
		extensions: cred.getClientExtensionResults?.() ?? {},
		response: {
			authenticatorData: bufToB64u(r.authenticatorData),
			clientDataJSON: bufToB64u(r.clientDataJSON),
			signature: bufToB64u(r.signature),
			userHandle: r.userHandle ? bufToB64u(r.userHandle) : null,
		},
	};
}

/** A name for this device, shown in the device list ("iPhone · Safari"). */
export function deviceName(ua = typeof navigator === 'undefined' ? '' : navigator.userAgent, touch = typeof navigator !== 'undefined' && navigator.maxTouchPoints > 1): string {
	const os = /iPhone/.test(ua)
		? 'iPhone'
		: /iPad/.test(ua) || (/Macintosh/.test(ua) && touch)
			? 'iPad'
			: /Android/.test(ua)
				? 'Android'
				: /Windows/.test(ua)
					? 'Windows'
					: /Macintosh/.test(ua)
						? 'Mac'
						: /CrOS/.test(ua)
							? 'Chromebook'
							: /Linux/.test(ua)
								? 'Linux'
								: 'Device';
	const browser = /Edg\//.test(ua)
		? 'Edge'
		: /Firefox\/|FxiOS/.test(ua)
			? 'Firefox'
			: /SamsungBrowser/.test(ua)
				? 'Samsung Internet'
				: /Chrome\/|CriOS/.test(ua)
					? 'Chrome'
					: /Safari\//.test(ua)
						? 'Safari'
						: 'Browser';
	return `${os} · ${browser}`;
}
