// Author and narrator links: names come from tags, so they are untrusted.
import { MAX_NAME, personFromParam, personHref } from '$lib/people';

test('links always stay inside the app and carry the whole name', () => {
	expect(personHref('author', 'Mara Quill')).toBe('#/author/Mara%20Quill');
	// A slash, a fragment or a scheme in a name can't leave the route or the app.
	expect(personHref('narrator', 'AC/DC #1')).toBe('#/narrator/AC%2FDC%20%231');
	expect(personHref('author', 'javascript:alert(1)')).toBe('#/author/javascript%3Aalert(1)');
	for (const n of ['../../admin', '?x=1', 'a‮b']) expect(personHref('author', n).startsWith('#/author/')).toBe(true);
});

test('route names are checked before use', () => {
	expect(personFromParam(' Mara Quill ')).toBe('Mara Quill');
	expect(personFromParam('AC/DC')).toBe('AC/DC');
	for (const bad of [undefined, '', '   ', 'x'.repeat(MAX_NAME + 1), 'a\u0000b', 'a\nb']) expect(personFromParam(bad)).toBeNull();
});
