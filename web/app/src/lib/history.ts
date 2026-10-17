// Listening history (SPEC §5.6): the shape of GET v1/me/history and how its numbers read.

export interface HistorySession {
	id: number;
	book: { id: string; title: string; authors: string[]; cover: string | null };
	started_at: number;
	ended_at: number;
	seconds: number;
}

export interface History {
	enabled: boolean;
	totals: { week_seconds: number; month_seconds: number; all_seconds: number; books_finished: number };
	sessions: HistorySession[];
	next: string | null;
}

/** Time listened: "None", "Under a minute", "45 min", "3 h 20 min", "1,204 h". */
export function listened(seconds: number): string {
	if (!Number.isFinite(seconds) || seconds <= 0) return 'None';
	if (seconds < 60) return 'Under a minute';
	const mins = Math.floor(seconds / 60);
	const h = Math.floor(mins / 60);
	const m = mins % 60;
	if (h === 0) return `${m} min`;
	if (h >= 100) return `${h.toLocaleString('en-GB')} h`;
	return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** When a session was, from its start (milliseconds), in the reader's locale: "17 Oct 2026, 14:05". */
export function when(ms: number): string {
	if (!Number.isFinite(ms) || ms <= 0) return '—';
	return new Date(ms).toLocaleString(undefined, { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });
}
