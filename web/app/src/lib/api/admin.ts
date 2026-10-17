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

export const adminApi = {
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
};

/** "Every library" or the names. */
export const librariesLabel = (l: Libraries) => (l === '*' ? 'All libraries' : l.length ? l.join(', ') : 'No libraries');
