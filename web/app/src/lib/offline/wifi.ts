// "Download only on Wi-Fi": whether to hold downloads, from the preference and what the
// browser says about the connection (Network Information API: Chrome on Android reports
// the type; Safari and Firefox don't, and then nothing is held).

export interface ConnectionInfo {
	type?: string;
	saveData?: boolean;
}

/** The browser's connection details, if it offers any. */
export function connection(): (ConnectionInfo & EventTarget) | undefined {
	return (globalThis.navigator as Navigator & { connection?: ConnectionInfo & EventTarget })?.connection;
}

/** Whether the browser says what kind of connection this is. */
export const typeKnown = (c: ConnectionInfo | undefined) => typeof c?.type === 'string' && c.type !== 'unknown';

/** Hold downloads when only Wi-Fi is wanted and the browser reports mobile data (or the
 *  person's data-saving mode). With nothing reported, nothing is held. */
export function shouldHold(wifiOnly: boolean, c: ConnectionInfo | undefined): boolean {
	if (!wifiOnly || !c) return false;
	return c.type === 'cellular' || c.saveData === true;
}

export const WAITING_FOR_WIFI = 'Waiting for Wi-Fi';
