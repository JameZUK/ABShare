// Shapes of the /v1 API's answers (server/src/http/*_routes.rs).

export interface User {
	username: string;
	display_name: string;
	role: 'listener' | 'admin';
	/** Signed in with a recovery code: only the account page works until a new factor is set up. */
	needs_factor?: boolean;
}

export interface Progress {
	position_ms: number;
	finished: boolean;
	updated_at: number;
}

export interface BookSummary {
	type: 'book';
	id: string;
	title: string;
	subtitle: string | null;
	authors: string[];
	narrators: string[];
	series: { name: string; position: string | null } | null;
	duration_ms: number;
	cover: string | null;
	added_at: number;
	progress: Progress | null;
	/** In a group's list: a bonus item, not part of the numbering. */
	extra?: boolean;
}

export interface ShowTile {
	type: 'show';
	id: string;
	title: string;
	episodes: number;
	cover: string | null;
}

export type Tile = BookSummary | ShowTile;

export interface Person {
	name: string;
}

export interface Chapter {
	start_ms: number;
	end_ms: number;
	title: string;
}

export interface Bookmark {
	id: number;
	position_ms: number;
	note: string;
	created_at: number;
	updated_at: number;
}

export interface BookDetail {
	id: string;
	title: string;
	subtitle: string | null;
	authors: Person[] | null;
	narrators: Person[] | null;
	series: { name: string; position?: string | null } | null;
	description: string | null;
	genres: string[];
	/** Genre shelves (core::genre), linking to `#/genre/{id}`. */
	shelves?: { id: string; name: string }[];
	year: number | string | null;
	publisher: string | null;
	language: string | null;
	duration_ms: number;
	cover: string | null;
	added_at: number;
	chapters: Chapter[];
	progress: Progress | null;
	bookmarks: Bookmark[];
	groups: { id: string; name: string; kind: string }[];
	media: { original: string; preparing: boolean };
}

export interface GroupDetail {
	id: string;
	name: string;
	qualifier: string | null;
	kind: 'series' | 'show' | string;
	items: BookSummary[];
	next: string | null;
}

export interface Home {
	continue: BookSummary[];
	series_next: { series: string; book: BookSummary }[];
	recently_added: BookSummary[];
}

export interface Page<T> {
	items: T[];
	total: number;
	next: string | null;
}

export interface Me {
	username: string;
	display_name: string;
	role: string;
	device: number;
	factors: { passkeys: number; password: boolean; totp: boolean; recovery_codes_left: number };
	permissions: { offline: boolean; save_files: boolean };
	needs_factor: boolean;
}

export interface Device {
	id: number;
	name: string;
	created_at: number;
	last_seen: number | null;
	last_ip: string | null;
	user_agent: string | null;
	this_device: boolean;
}

export interface PasskeyInfo {
	id: number;
	name: string;
	created_at: number;
	last_used: number | null;
}

/** What sign-in and refresh return. */
export interface SignedIn {
	access_token: string;
	expires_in: number;
	user: User;
	recovery_codes?: string[];
}
