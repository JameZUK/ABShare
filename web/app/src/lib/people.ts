// Author and narrator pages: links to them, and reading the name back from the route.
// Names come from audiobook tags, so they are untrusted: a link always starts with "#/" and
// encodes the whole name, and the page passes the name to the API as a query value only.

export type PersonKind = 'author' | 'narrator';

/** The longest name worth asking about (the server refuses longer). */
export const MAX_NAME = 200;

export function personHref(kind: PersonKind, name: string): string {
	return `#/${kind}/${encodeURIComponent(name)}`;
}

/** The person named by a route parameter, or null if it isn't a usable name. */
export function personFromParam(param: string | undefined): string | null {
	const name = (param ?? '').trim();
	if (!name || name.length > MAX_NAME || /[\u0000-\u001f\u007f]/.test(name)) return null;
	return name;
}
