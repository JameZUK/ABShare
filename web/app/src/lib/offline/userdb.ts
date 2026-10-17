// The signed-in user's database, opened once and shared.
import { session } from '$lib/api/session.svelte';
import { openDb } from './db';

let cached: { user: string; db: Promise<IDBDatabase> } | null = null;

/** The current user's database, or null when nobody is known. */
export function userDb(): Promise<IDBDatabase> | null {
	const user = session.user?.username;
	if (!user) return null;
	if (cached?.user !== user) {
		cached?.db.then((d) => d.close()).catch(() => {});
		const db = openDb(user).then((d) => {
			// Another tab deleting this database (a sign-out or revocation wipe) must not be
			// blocked by this open connection (review W3).
			d.onversionchange = () => {
				d.close();
				if (cached?.db === db) cached = null;
			};
			return d;
		});
		cached = { user, db };
		cached.db.catch(() => {
			cached = null;
		});
	}
	return cached.db;
}

/** Closes the database (before deleting it). */
export async function closeUserDb() {
	const c = cached;
	cached = null;
	if (c) (await c.db.catch(() => null))?.close();
}
