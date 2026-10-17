// Genre shelves (server: core::genre). Fetched once per page view; small.
import { api } from './session.svelte';

export interface Genre {
	id: string;
	name: string;
	kind: 'fiction' | 'non-fiction' | 'other';
	count: number;
}

export const loadGenres = () => api<{ genres: Genre[]; unshelved: number }>('v1/genres');

/** The two umbrella views and their names. */
export const KINDS: Record<string, string> = { fiction: 'Fiction', 'non-fiction': 'Non-fiction' };

export const isShelfId = (s: string) => /^[a-z-]{1,40}$/.test(s);
