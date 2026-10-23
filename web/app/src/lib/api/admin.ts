// The admin API (server/src/http/admin_routes.rs).
import { api, post } from './session.svelte';

export type Libraries = '*' | string[];

export interface AdminUser {
	id: number;
	username: string;
	display_name: string;
	role: 'admin' | 'listener';
	disabled: boolean;
	factors: { passkeys: number; password: boolean; totp: boolean; recovery_codes_left: number };
	devices: number;
	last_seen: number | null;
	created_at: number;
	permissions: { libraries: Libraries; offline: boolean; save_files: boolean };
}

export interface Invite {
	id: number;
	purpose: 'enrol' | 'reset';
	display_name: string | null;
	role: string;
	created_at: number;
	expires_at: number;
	for_user: string | null;
	libraries: Libraries;
	offline: boolean;
	save_files: boolean;
}

export interface AuditEntry {
	id: number;
	at: number;
	user: string | null;
	device: number | null;
	event: string;
	detail: string;
	ip: string | null;
}

export interface Problem {
	library: string;
	path: string;
	kind: string;
	detail: string;
}

export interface LibraryView {
	libraries: { id: number; name: string; books: number; needs_copy: number; covers: number; groups: number }[];
	last_scan: { at: number; took_seconds: number; libraries: { library: string; files: number; books: number; problems: Record<string, number> }[] } | null;
	scan_requested: boolean;
	problems: { attention: Problem[]; info: Problem[] };
}

export interface Status {
	version: string;
	users: number;
	admins: number;
	disabled: number;
	pending_invites: number;
	two_step_coverage: number;
	sandbox: { landlock: boolean };
	cache_bytes: number;
	warnings: string[];
}

/** Admin → Server settings (D35): read-only; the config file is the only way to change them. */
export interface ServerSettings {
	config: {
		public_origin: string;
		listen: string;
		trusted_proxies: string[];
		web_app: boolean;
		libraries: string[];
		ffmpeg_jobs: number;
		cache_max_gib: number;
	};
	rules: {
		two_step: 'always';
		access_minutes: number;
		sign_in_days: number;
		step_up_minutes: number;
		max_devices: number;
		invite_hours: number;
		history_days: number;
		abandoned_days: number;
		audit_days: number;
		audit_rows: number;
		writes_per_minute: number;
		books_per_day: number;
		audio_at_once: number;
	};
}

/** A start-up report from a browser left with a blank page (static/diag.js). */
export interface DiagReport {
	at: number;
	addr: string;
	lines: string[];
}

export interface JobsView {
	lanes: Record<'listening' | 'background' | 'copies', { busy: number; of: number }>;
	read_aheads: number;
	building: { id: string; title: string }[];
	failed: { id: string; title: string; failures: number; retry_at: number }[];
	cache_bytes: number;
}

export interface AdminRecommendation {
	id: number;
	from: string;
	note: string;
	created_at: number;
	everyone: boolean;
	users: { id: number; name: string }[];
	books: { id: string; title: string }[];
	dismissed: number;
}

export interface ListeningTotals {
	week_seconds: number;
	month_seconds: number;
	all_seconds: number;
	finished: number;
	in_progress: number;
	abandoned: number;
}

export interface PersonListening {
	id: number;
	username: string;
	name: string;
	history_on: boolean;
	disabled: boolean;
	totals: ListeningTotals;
	last_listened_at: number | null;
}

export interface PersonListeningDetail {
	id: number;
	name: string;
	history_on: boolean;
	totals: ListeningTotals;
	last_listened_at: number | null;
	books: { book: { id: string; title: string | null; authors: string[]; cover: string | null; duration_ms: number | null }; status: 'finished' | 'in_progress' | 'abandoned'; heard_seconds: number; last_at: number }[];
	sessions: { book: { id: string; title: string | null }; started_at: number; ended_at: number; seconds: number }[];
}

export const adminApi = {
	listening: () => api<{ people: PersonListening[] }>('v1/admin/listening').then((r) => r.people),
	personListening: (id: number) => api<PersonListeningDetail>(`v1/admin/users/${id}/listening`),
	users: () => api<{ users: AdminUser[] }>('v1/admin/users').then((r) => r.users),
	editUser: (id: number, change: Partial<{ display_name: string; role: string; disabled: boolean; libraries: Libraries; offline: boolean; save_files: boolean }>) =>
		post<AdminUser>(`v1/admin/users/${id}`, change, 'PATCH'),
	resetUser: (id: number) => post<{ link: string }>(`v1/admin/users/${id}/reset`, {}),
	signOutUser: (id: number) => post<{ signed_out: number }>(`v1/admin/users/${id}/sign-out`, {}),
	invites: () => api<{ invites: Invite[] }>('v1/admin/invites').then((r) => r.invites),
	createInvite: (b: { display_name?: string; role?: string; confirm_admin?: boolean; libraries?: Libraries; offline?: boolean; save_files?: boolean; expires_hours?: number }) =>
		post<{ link: string; expires_in_hours: number }>('v1/admin/invites', b),
	revokeInvite: (id: number) => api(`v1/admin/invites/${id}`, { method: 'DELETE' }),
	audit: (before?: number) =>
		api<{ entries: AuditEntry[]; next: number | null; chain: { intact: boolean; entries?: number; broken_at?: number } }>(
			`v1/admin/audit${before ? `?before=${before}` : ''}`,
		),
	library: () => api<LibraryView>('v1/admin/library'),
	scan: () => api('v1/admin/library/scan', { method: 'POST' }),
	status: () => api<Status>('v1/admin/status'),
	settings: () => api<ServerSettings>('v1/admin/settings'),
	jobs: () => api<JobsView>('v1/admin/jobs'),
	retryCopy: (id: string) => post(`v1/admin/jobs/copies/${id}/retry`, {}),
	clearCache: () => post<{ freed_bytes: number }>('v1/admin/cache/clear', {}),
	diagnostics: () => api<{ reports: DiagReport[] }>('v1/admin/diagnostics').then((r) => r.reports),
	recommendations: () => api<{ recommendations: AdminRecommendation[] }>('v1/admin/recommendations').then((r) => r.recommendations),
	createRecommendation: (b: { books: string[]; note: string; everyone: boolean; users: number[] }) => post<{ id: number }>('v1/admin/recommendations', b),
	withdrawRecommendation: (id: number) => api(`v1/admin/recommendations/${id}`, { method: 'DELETE' }),
	removeReview: (bookId: string, userId: number) => api(`v1/admin/books/${encodeURIComponent(bookId)}/reviews/${userId}`, { method: 'DELETE' }),
};

/** "Every library" or the names. */
export const librariesLabel = (l: Libraries) => (l === '*' ? 'All libraries' : l.length ? l.join(', ') : 'No libraries');
