// Times and sizes as people read them.

/** 1:02:03, or 2:03 under an hour. */
export function clock(ms: number): string {
	if (!Number.isFinite(ms) || ms < 0) ms = 0;
	const t = Math.floor(ms / 1000);
	const h = Math.floor(t / 3600);
	const m = Math.floor(t / 60) % 60;
	const s = t % 60;
	return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
}

/** "12 h 5 min", "45 min", "under a minute". */
export function duration(ms: number): string {
	const mins = Math.round(ms / 60_000);
	if (mins < 1) return 'under a minute';
	const h = Math.floor(mins / 60);
	const m = mins % 60;
	if (h === 0) return `${m} min`;
	return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

export function timeLeft(durationMs: number, positionMs: number): string {
	return `${duration(Math.max(0, durationMs - positionMs))} left`;
}

export function bytes(n: number): string {
	if (n < 1024 * 1024) return `${Math.max(0, Math.round(n / 1024))} KB`;
	if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(0)} MB`;
	return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

/** "Ann Author and Bob Writer", "A, B and C". */
export function names(list: (string | { name: string })[] | null | undefined): string {
	const n = (list ?? []).map((x) => (typeof x === 'string' ? x : x.name)).filter(Boolean);
	if (n.length <= 1) return n[0] ?? '';
	return `${n.slice(0, -1).join(', ')} and ${n[n.length - 1]}`;
}

export function date(secs: number | null | undefined): string {
	if (!secs) return '—';
	return new Date(secs * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
}
