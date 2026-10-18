// "How was it?" (SPEC D32): which finished books this person has already been asked to
// review on this device, so the prompt never appears twice for the same book. Per person,
// bounded, and harmless if storage is blocked.

type Store = Pick<Storage, 'getItem' | 'setItem'>;

/** At most this many remembered books per person; the oldest are forgotten first. */
export const MAX_ASKED = 500;

const keyFor = (user: string) => `abshare.reviewAsked.${user}`;

function safeLocal(): Store | undefined {
	try {
		return globalThis.localStorage;
	} catch {
		return undefined;
	}
}

function read(user: string, store: Store | undefined): string[] {
	try {
		const v = JSON.parse(store?.getItem(keyFor(user)) ?? '[]');
		return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && /^[a-z2-7]{26}$/.test(x)) : [];
	} catch {
		return [];
	}
}

export function askedBefore(user: string, book: string, store: Store | undefined = safeLocal()): boolean {
	return read(user, store).includes(book);
}

export function markAsked(user: string, book: string, store: Store | undefined = safeLocal()) {
	const list = read(user, store).filter((b) => b !== book);
	list.push(book);
	try {
		store?.setItem(keyFor(user), JSON.stringify(list.slice(-MAX_ASKED)));
	} catch {
		/* not essential: at worst the prompt could appear again */
	}
}

/** Whether to show the prompt for a book that has just been finished. */
export function shouldPrompt(s: { online: boolean; asked: boolean; reviewed: boolean }): boolean {
	return s.online && !s.asked && !s.reviewed;
}
