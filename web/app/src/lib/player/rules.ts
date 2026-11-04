// Pure playback rules: auto-rewind, the sleep timer's fade and the streaming quality choice.
import type { QualityMode, SaverKbps } from '$lib/settings.svelte';

/** How far to rewind on resuming after a pause of `pausedMs` (SPEC §7.3). */
/** A requested start position (a history point, a bookmark) made safe: within the book and
 *  short of its very end, so opening there doesn't finish the book at once (review W13). */
export function clampStart(ms: number, durationMs: number): number {
	if (!Number.isFinite(ms)) return 0;
	const limit = Number.isFinite(durationMs) && durationMs > 500 ? durationMs - 500 : 0;
	return Math.max(0, Math.min(ms, limit));
}

export function autoRewindMs(pausedMs: number): number {
	if (pausedMs < 60_000) return 0;
	if (pausedMs < 10 * 60_000) return 2_000;
	if (pausedMs < 60 * 60_000) return 5_000;
	return 10_000;
}

/** How long after the app pauses or plays the audio its event still counts as the app's. */
export const ASKED_MS = 1_500;

/** Why the audio paused: the app asked (the listener or the sleep timer), the book ended or
 *  failed, or the browser paused it by itself, as it does for a phone call or another app's
 *  sound (SPEC D38). */
export function pauseCause(o: { askedAt: number; now: number; ended: boolean; failed: boolean }): 'asked' | 'ended' | 'failed' | 'browser' {
	if (o.ended) return 'ended';
	if (o.failed) return 'failed';
	return o.askedAt > 0 && o.now - o.askedAt < ASKED_MS ? 'asked' : 'browser';
}

/** What to do when the browser starts the book again by itself. After its own pause (a call that
 *  has ended): carry on, carry on after the chosen wait, or stay paused, as the listener chose.
 *  After the listener's pause: stay paused, whatever the browser does. */
export function afterBrowserPlay(o: { interrupted: boolean; wanted: 'play' | 'pause'; resume: boolean; delayS: number }): { kind: 'go' } | { kind: 'stay' } | { kind: 'later'; ms: number } {
	if (o.wanted === 'pause') return { kind: 'stay' };
	if (!o.interrupted) return { kind: 'go' };
	if (!o.resume) return { kind: 'stay' };
	return o.delayS > 0 ? { kind: 'later', ms: o.delayS * 1000 } : { kind: 'go' };
}

/** The volume during the sleep timer's last 10 s: 1 down to 0. */
export function fadeVolume(remainingMs: number, fadeMs = 10_000): number {
	if (remainingMs >= fadeMs) return 1;
	return Math.max(0, remainingMs / fadeMs);
}

interface Connection {
	type?: string;
	effectiveType?: string;
	saveData?: boolean;
}

/** Original or a data-saver bitrate, given the setting and what the browser says about the network. */
export function chooseQuality(mode: QualityMode, kbps: SaverKbps, conn: Connection | undefined): 'original' | SaverKbps {
	if (mode === 'original') return 'original';
	if (mode === 'saver') return kbps;
	if (conn && (conn.saveData || conn.type === 'cellular' || conn.effectiveType === '2g' || conn.effectiveType === 'slow-2g')) return kbps;
	return 'original';
}

/** The stream URL for a book at a quality. */
export const streamUrl = (book: string, q: 'original' | SaverKbps) => (q === 'original' ? `v1/media/${book}` : `v1/media/${book}?q=${q}`);
