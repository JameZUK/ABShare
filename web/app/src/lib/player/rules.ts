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
